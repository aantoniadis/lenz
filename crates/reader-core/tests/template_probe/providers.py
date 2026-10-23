"""Run only on private throwaway projects; retain raw provider messages and failures."""
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import threading
import time

from probe import (HERE, apply_provider_edit, position, project,
                   provider_range, markup_project, apply_markup_edit, haml_context)

HTML = Path('/usr/share/code/resources/app/extensions/html-language-features/server/dist/node/htmlServerMain.js')


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


class Rpc:
    def __init__(self, command, directory, env):
        self.started = time.monotonic()
        self.log = (directory / 'rpc.jsonl').open('w')
        self.stderr = (directory / 'stderr.log').open('w')
        self.child = subprocess.Popen(command, cwd=directory, env=env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr)
        self.messages = queue.Queue()
        self.id = 0
        self.elapsed = []
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        try:
            while True:
                length = None
                while True:
                    line = self.child.stdout.readline()
                    if not line:
                        return
                    if line == b'\r\n':
                        break
                    name, value = line.split(b':', 1)
                    if name.lower() == b'content-length':
                        length = int(value)
                if length is None or length > 64 << 20:
                    return
                body = self.child.stdout.read(length)
                self.messages.put(json.loads(body))
        finally:
            self.messages.put(None)

    def record(self, direction, message):
        self.log.write(json.dumps({'direction': direction, 'ms': (time.monotonic() - self.started) * 1000,
                                   'message': message}) + '\n')
        self.log.flush()

    def send(self, value):
        self.record('send', value)
        data = json.dumps(value).encode()
        self.child.stdin.write(f'Content-Length: {len(data)}\r\n\r\n'.encode() + data)
        self.child.stdin.flush()

    def notify(self, method, params):
        self.send({'jsonrpc': '2.0', 'method': method, 'params': params})

    def ask(self, method, params):
        self.id += 1
        request = self.id
        started = time.monotonic()
        self.send({'jsonrpc': '2.0', 'id': request, 'method': method, 'params': params})
        deadline = started + 30
        while True:
            message = self.messages.get(timeout=max(0, deadline - time.monotonic()))
            if message is None:
                raise RuntimeError('provider exited')
            self.record('receive', message)
            if message.get('id') == request and 'method' not in message:
                self.elapsed.append({'method': method, 'ms': (time.monotonic() - started) * 1000})
                return message
            if 'id' in message and 'method' in message:
                result = None
                if message['method'] == 'workspace/configuration':
                    result = [{} for _ in message['params']['items']]
                self.send({'jsonrpc': '2.0', 'id': message['id'], 'result': result})

    def initialize(self, directory, encoding):
        result = self.ask('initialize', {'processId': os.getpid(), 'rootUri': directory.as_uri(),
                          'capabilities': {'general': {'positionEncodings': [encoding]},
                                           'textDocument': {'completion': {'completionItem': {'snippetSupport': False}},
                                                            'diagnostic': {}},
                                           'workspace': {'configuration': True}},
                          'initializationOptions': {'formatter': 'none'}})
        save(directory / 'initialize.json', result)
        self.encoding = result['result']['capabilities'].get('positionEncoding', 'utf-16')
        if self.encoding != encoding:
            raise ValueError(f'provider negotiated {self.encoding}, expected {encoding}')
        self.notify('initialized', {})

    def open(self, path, text, language='ruby'):
        self.notify('textDocument/didOpen', {'textDocument': {'uri': path.as_uri(), 'languageId': language,
                                                           'version': 1, 'text': text}})

    def change(self, path, old, new, version):
        self.notify('textDocument/didChange', {'textDocument': {'uri': path.as_uri(), 'version': version},
                     'contentChanges': [{'range': {'start': position(old, 0, self.encoding),
                                                  'end': position(old, len(old), self.encoding)}, 'text': new}]})

    def at(self, method, path, text, needle):
        return self.ask(method, {'textDocument': {'uri': path.as_uri()},
                                'position': position(text, text.index(needle) + len(needle), self.encoding)})

    def close(self):
        rss = peak = None
        status = Path(f'/proc/{self.child.pid}/status')
        if status.exists():
            rss = next((int(line.split()[1]) for line in status.read_text().splitlines()
                        if line.startswith('VmRSS:')), None)
            peak = next((int(line.split()[1]) for line in status.read_text().splitlines()
                         if line.startswith('VmHWM:')), None)
        self.child.kill()
        self.child.wait(timeout=5)
        self.child.stdin.close()
        self.child.stdout.close()
        self.log.close()
        self.stderr.close()
        return {'request_timings': self.elapsed, 'elapsed_seconds': time.monotonic() - self.started,
                'settled_rss_kib': rss, 'peak_rss_kib': peak}


def environment(directory):
    env = os.environ.copy()
    for name, suffix in [('HOME', 'home'), ('XDG_CACHE_HOME', 'cache'), ('XDG_CONFIG_HOME', 'config'),
                         ('BUNDLE_USER_HOME', 'bundle-home')]:
        path = directory / suffix
        path.mkdir(exist_ok=True)
        env[name] = str(path)
    env['BUNDLE_IGNORE_CONFIG'] = '1'
    return env


def ruby(directory, patched=False):
    env = environment(directory)
    env['GEM_PATH'] = subprocess.check_output(['ruby', '-e', "puts Gem.path.join(':')"], text=True).strip()
    (directory / 'Gemfile').write_text("source 'https://rubygems.org'\ngem 'ruby-lsp', '=0.26.11'\n")
    env['BUNDLE_GEMFILE'] = str(directory / 'Gemfile')
    result = subprocess.run(['bundle', 'lock', '--local'], cwd=directory, env=env, capture_output=True)
    (directory / 'bundle-local.log').write_bytes(result.stdout + result.stderr)
    result.check_returncode()
    env['RUBYOPT'] = '-rbundler/setup' + (f' -r{HERE / "signature_cursor.rb"}' if patched else '')
    return Rpc(['ruby-lsp'], directory, env)


def items(result):
    result = result.get('result') or []
    return result if isinstance(result, list) else result.get('items', [])


def fixtures(language):
    if language == 'erb':
        return {
            'inline': '<div title="😀"><%= "😀"; ReadyWidget.gre %></div>\r\n',
            'interpolation': '<p>😀<%= "😀 #{ReadyWidget.gre}" %></p>\r\n',
            'multiline': '<p>😀</p>\r\n<%= "😀"; (\r\n  ReadyWidget.gre\r\n) %>\r\n',
        }
    return {
        'inline': '%div(title="😀")\r\n  = "😀"; ReadyWidget.gre\r\n',
        'interpolation': '%p 😀 #{ "😀"; ReadyWidget.gre }\r\n',
        'multiline': '%p(title="😀")\r\n  = "😀"; ( |\r\n      ReadyWidget.gre |\r\n    ) |\r\n',
    }


def ruby_cases(directory, language, encoding, patched=False):
    directory.mkdir(parents=True, exist_ok=True)
    helper = 'class ReadyWidget\n  def self.greet(name, excited: false)\n    name\n  end\nend\n'
    (directory / 'helper.rb').write_text(helper)
    rpc = ruby(directory, patched)
    summary = {'language': language, 'requested_encoding': encoding, 'private_signature_cursor_preload': patched, 'cases': {}}
    try:
        rpc.initialize(directory, encoding)
        rpc.open(directory / 'helper.rb', helper)
        for case, source in fixtures(language).items():
            target = directory / f'{case}.html.{language}'
            target.write_bytes(source.encode())
            projection = project(source, language)
            path = directory / f'{case}.rb'
            path.write_bytes(projection.generated.encode())
            save(directory / f'{case}-projection.json', vars(projection))
            rpc.open(path, projection.generated)
            deadline = time.monotonic() + 15
            while True:
                completion = rpc.at('textDocument/completion', path, projection.generated, 'ReadyWidget.gre')
                members = [item for item in items(completion) if item['label'] == 'greet']
                if members or time.monotonic() > deadline:
                    break
                time.sleep(.1)
            save(directory / f'{case}-completion.json', completion)
            assert members, 'missing real project helper member'
            changed = apply_provider_edit(projection, members[0], encoding, 1, 'fixture')
            assert changed == source.replace('ReadyWidget.gre', 'ReadyWidget.greet'), 'wrong source replacement'
            (directory / f'{case}-accepted.html.{language}').write_bytes(changed.encode())
            signature_source = source.replace('ReadyWidget.gre', 'ReadyWidget.greet(')
            signature_projection = project(signature_source, language)
            rpc.change(path, projection.generated, signature_projection.generated, 2)
            signature = rpc.at('textDocument/signatureHelp', path, signature_projection.generated, 'ReadyWidget.greet(')
            save(directory / f'{case}-signature.json', signature)
            signatures = (signature.get('result') or {}).get('signatures', [])
            invalid = source.replace('ReadyWidget.gre', 'ReadyWidget.greet(]')
            invalid_projection = project(invalid, language)
            rpc.change(path, signature_projection.generated, invalid_projection.generated, 3)
            diagnostics = rpc.ask('textDocument/diagnostic', {'textDocument': {'uri': path.as_uri()}})
            save(directory / f'{case}-diagnostics.json', diagnostics)
            mapped = []
            for diagnostic in (diagnostics.get('result') or {}).get('items', []):
                try:
                    start, end = provider_range(invalid_projection, diagnostic['range'], encoding)
                except ValueError:
                    continue
                mapped.append({'text': invalid[start:end], 'source_bytes': [len(invalid[:start].encode()), len(invalid[:end].encode())],
                               'severity': diagnostic.get('severity'), 'message': diagnostic['message']})
            save(directory / f'{case}-mapped-diagnostics.json', mapped)
            summary['cases'][case] = {'completion_exact': True, 'signature_count': len(signatures),
                                      'signature_label': signatures[0]['label'] if signatures else None,
                                      'active_parameter': (signature.get('result') or {}).get('activeParameter'),
                                      'syntax_diagnostic_exact': any(d['severity'] == 1 and d['text'] == ']' for d in mapped)}
            assert signatures and signatures[0]['label'] == 'greet(name, excited: <default>)', 'wrong project helper signature'
            assert signature['result']['activeParameter'] == 0, 'wrong first active parameter'
            assert summary['cases'][case]['syntax_diagnostic_exact'], 'missing exact syntax diagnostic'
            assert target.read_bytes() == source.encode(), 'original source file changed'
    finally:
        summary.update(rpc.close())
        save(directory / 'summary.json', summary)
    return summary




def signature_edges(directory):
    """Retain counterexamples; desired behavior is separate from observed provider output."""
    directory.mkdir(parents=True, exist_ok=True)
    rpc = ruby(directory, patched=True)
    results = []
    helper = ('class ReadyWidget\n'
              '  def self.greet(name, excited: false)\n    name\n  end\n'
              '  def self.outer(first, second = nil)\n    first\n  end\nend\n')
    (directory / 'helper.rb').write_text(helper)
    cases = [
        ('opening', 'ReadyWidget.greet(|', 'greet(name, excited: <default>)', 0),
        ('populated-first-argument', 'ReadyWidget.greet(|"name", excited: true)', 'greet(name, excited: <default>)', 0),
        ('comma', 'ReadyWidget.greet("😀",|', 'greet(name, excited: <default>)', 1),
        ('comma-space', 'ReadyWidget.greet("😀", |', 'greet(name, excited: <default>)', 1),
        ('nested-opening', 'ReadyWidget.outer(ReadyWidget.greet(|', 'greet(name, excited: <default>)', 0),
        ('nested-comma', 'ReadyWidget.outer(ReadyWidget.greet("😀",|', 'greet(name, excited: <default>)', 1),
        ('within-unicode-argument', 'ReadyWidget.greet("😀|", excited: true)', 'greet(name, excited: <default>)', 0),
        ('nested-comma-in-argument', 'ReadyWidget.greet(ReadyWidget.outer(1, 2), |', 'greet(name, excited: <default>)', 1),
        ('nested-outer-second', 'ReadyWidget.outer(ReadyWidget.greet("😀"), |', 'outer(first, second = <default>)', 1),
        ('within-array-argument', 'ReadyWidget.greet([1, |2])', 'greet(name, excited: <default>)', 0),
        ('existing-closing', 'ReadyWidget.greet(|)', 'greet(name, excited: <default>)', 0),
        ('after-closed-space', 'ReadyWidget.greet("😀") |', None, None),
        ('closed', 'ReadyWidget.greet("😀")|', None, None),
        ('start', '|ReadyWidget.greet("😀")', None, None),
    ]
    try:
        rpc.initialize(directory, 'utf-8')
        rpc.open(directory / 'helper.rb', helper)
        # First real completion waits for indexing, independent of signature results.
        warm = directory / 'warm.rb'
        rpc.open(warm, 'ReadyWidget.gre')
        deadline = time.monotonic() + 15
        while not any(item['label'] == 'greet' for item in items(rpc.at('textDocument/completion', warm, 'ReadyWidget.gre', 'gre'))):
            if time.monotonic() >= deadline:
                raise AssertionError('helper not indexed')
            time.sleep(.1)
        for name, marked, expected_label, expected_parameter in cases:
            at = marked.index('|')
            text = marked.replace('|', '')
            path = directory / f'{name}.rb'
            path.write_bytes(text.encode())
            rpc.open(path, text)
            response = rpc.ask('textDocument/signatureHelp', {'textDocument': {'uri': path.as_uri()},
                                                             'position': position(text, at, 'utf-8')})
            save(directory / f'{name}-signature.json', response)
            result = response.get('result') or {}
            signatures = result.get('signatures', [])
            label = signatures[0]['label'] if signatures else None
            parameter = result.get('activeParameter') if signatures else None
            results.append({'case': name, 'expected_label': expected_label, 'actual_label': label,
                            'expected_active_parameter': expected_parameter, 'actual_active_parameter': parameter,
                            'verified': label == expected_label and parameter == expected_parameter})
    finally:
        save(directory / 'resources.json', rpc.close())
    save(directory / 'summary.json', results)
    return results


def markup_cases(directory):
    directory.mkdir(parents=True, exist_ok=True)
    rpc = Rpc(['node', str(HTML), '--stdio'], directory, environment(directory))
    results = []
    try:
        rpc.initialize(directory, 'utf-16')
        for language in ['erb', 'haml']:
            for case, source, needle, expected in [
                ('tag', '<di' if language == 'erb' else '%main\n  %di', 'di',
                 '<div' if language == 'erb' else '%main\n  %div'),
                ('attribute', '<div title="😀" cla>' if language == 'erb' else '%main\r\n  %div(title="😀" cla)', 'cla',
                 '<div title="😀" class="">' if language == 'erb' else '%main\r\n  %div(title="😀" class="")'),
            ]:
                projection = markup_project(source, language)
                path = directory / f'{language}-{case}.html'
                rpc.open(path, projection.generated, 'html')
                completion = rpc.at('textDocument/completion', path, projection.generated, needle)
                save(directory / f'{language}-{case}-completion.json', completion)
                wanted = 'div' if case == 'tag' else 'class'
                member = next(item for item in items(completion) if item['label'] == wanted)
                changed = apply_markup_edit(projection, member)
                save(directory / f'{language}-{case}-accepted.json', {'source': source, 'generated': projection.generated,
                                                                      'accepted': changed, 'provider_item': member})
                assert changed == expected, f'invalid {language} markup replacement: {changed!r}'
                if language == 'haml':
                    save(directory / f'{language}-{case}-compiled.json', haml_context(changed, validate=True))
                results.append({'language': language, 'case': case, 'accepted': changed})
    finally:
        save(directory / 'resources.json', rpc.close())
    save(directory / 'summary.json', results)
    return results


def main():
    directory = Path(sys.argv[1]).resolve()
    directory.mkdir(parents=True, exist_ok=True)
    encoding = sys.argv[2] if len(sys.argv) > 2 else 'utf-8'
    patched = '--private-fix' in sys.argv[3:]
    gem_path = Path(subprocess.check_output(['ruby', '-e', "puts Gem::Specification.find_by_name('ruby-lsp').full_gem_path"], text=True).strip())
    provider_sources = [gem_path / 'lib/ruby_lsp' / name for name in [
        'document.rb', 'ruby_document.rb', 'global_state.rb',
        'requests/signature_help.rb', 'listeners/signature_help.rb', 'requests/support/common.rb']]
    haml_home = os.environ.get('EDITOR_HAML_GEM_HOME')
    if not haml_home:
        raise ValueError('set EDITOR_HAML_GEM_HOME to owned Haml 7.5.1 test gems')
    haml_sources = list((Path(haml_home) / 'gems' / 'haml-7.5.1' / 'lib').rglob('*.rb'))
    identity = {'base': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
                'ruby': subprocess.check_output(['ruby', '--version'], text=True).strip(),
                'prism': subprocess.check_output(['ruby', '-rprism', '-e', 'puts Prism::VERSION'], text=True).strip(),
                'ruby_lsp': subprocess.check_output(['ruby-lsp', '--version'], text=True).strip(),
                'node': subprocess.check_output(['node', '--version'], text=True).strip(),
                'executables': {tool: shutil.which(tool) for tool in ['ruby', 'ruby-lsp', 'node']},
                'sha256': {str(path): hashlib.sha256(path.read_bytes()).hexdigest()
                           for path in [HERE / 'probe.py', HERE / 'providers.py', HERE / 'regions.rb', HERE / 'signature_cursor.rb', HTML, *provider_sources, *haml_sources]},
                'configuration': 'private HOME/XDG/Bundler; installed gems only',
                'private_signature_cursor_preload': patched, 'haml': '7.5.1',
                'haml_gem_home': haml_home,
                'private_gems': sorted(path.stem for path in (Path(haml_home) / 'specifications').glob('*.gemspec')), 'platform': list(os.uname()),
                'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}
    save(directory / 'identity.json', identity)
    summaries = [ruby_cases(directory / language, language, encoding, patched) for language in ['erb', 'haml']]
    markup = markup_cases(directory / 'markup')
    edges = signature_edges(directory / 'signature-edges') if patched else []
    save(directory / 'summary.json', {'ruby': summaries, 'markup': markup, 'signature_edges': edges,
                                     'scoped_experiment_acceptance': bool(edges) and all(edge['verified'] for edge in edges),
                                     'production_template_feature': False,
                                     'gaps': ['Haml mapping covers expression/pipe/interpolation AST nodes only',
                                              'Markup contexts limited to isolated tags/attributes',
                                              'No production Session or adapter integration']})
    if '--require-complete-signatures' in sys.argv:
        assert all(edge['verified'] for edge in edges), 'signature edge cases remain unresolved'
    print(json.dumps(summaries, indent=2))


if __name__ == '__main__':
    main()

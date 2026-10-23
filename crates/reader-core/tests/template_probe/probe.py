"""Disposable template/provider experiment, never imported by production."""
from dataclasses import dataclass
import json
import os
import re
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent


@dataclass
class Projection:
    source: str
    generated: str
    spans: list

    def source_range(self, start, end):
        for original_start, _original_end, generated_start, generated_end in self.spans:
            if generated_start <= start <= end <= generated_end:
                return original_start + start - generated_start, original_start + end - generated_start
        raise ValueError('edit crosses generated scaffolding or language regions')


def project(source, language):
    if language == 'haml':
        regions = haml_regions(source)
    else:
        result = subprocess.run(['ruby', str(HERE / 'regions.rb')], input=json.dumps(
            {'source': source, 'language': language}), text=True, capture_output=True, check=True)
        regions = json.loads(result.stdout)
    generated, spans = '', []
    for start, end in regions:
        at = len(generated)
        generated += source[start:end]
        spans.append((start, end, at, len(generated)))
        generated += '\n'
    return Projection(source, generated, spans)


def apply_edit(projection, start, end, replacement):
    start, end = projection.source_range(start, end)
    return projection.source[:start] + replacement + projection.source[end:]


def haml_context(source, validate=False):
    gem_home = os.environ.get('EDITOR_HAML_GEM_HOME')
    if not gem_home:
        raise ValueError('Haml parser unverified: set EDITOR_HAML_GEM_HOME to owned test gems')
    env = {**os.environ, 'GEM_HOME': gem_home, 'RUBYOPT': ''}
    env.pop('GEM_PATH', None)
    result = subprocess.run(['ruby', str(HERE / 'regions.rb')], input=json.dumps(
        {'source': source, 'language': 'validate_haml' if validate else 'haml'}),
        text=True, capture_output=True, env=env)
    if result.returncode:
        raise ValueError(result.stderr)
    return json.loads(result.stdout)


def haml_regions(source):
    # shortcut: map expression/pipe/interpolation nodes only; extend against Haml's AST when broader contexts ship.
    contexts = {node['line']: node for node in haml_context(source)['nodes']}
    regions, offset, continuation = [], 0, False
    for line_number, line in enumerate(source.splitlines(keepends=True), 1):
        body = line.rstrip('\r\n')
        if '\t' in body[:len(body) - len(body.lstrip())]:
            raise ValueError('tab indentation unsupported')
        start = len(body) - len(body.lstrip(' '))
        text = body[start:]
        if not text:
            offset += len(line)
            continue
        if text.startswith(('\\', '-#')):
            raise ValueError('escaped lines and comments unsupported')
        if continuation:
            code_start = start
        elif line_number not in contexts:
            raise ValueError('source line lacks a recognized Haml node')
        else:
            tag = re.match(r'%[\w:-]+(?:[.#][\w-]+)*(?:\([^(){}]*\))?', text)
            if tag and '(' in tag[0]:
                attributes = tag[0].split('(', 1)[1][:-1]
                if not re.fullmatch(r"(?:[\w:-]+=(?:\"[^\"\\]*\"|'[^'\\]*')\s*)*", attributes):
                    raise ValueError('dynamic Haml attributes unsupported')
            code_start = start + (tag.end() if tag else 0)
            tail = body[code_start:]
            marker = re.match(r'(?:[&!]?=|-) ?', tail)
            if marker:
                code_start += marker.end()
            else:
                if text[0] in ':!~/#.' or tail.startswith(('(', '{')) or ('{' in tail and '#{' not in tail):
                    raise ValueError('unsupported Haml construct')
                for match in re.finditer(r'(?<!\\)#\{([^{}]*)\}', tail):
                    regions.append((offset + code_start + match.start(1), offset + code_start + match.end(1)))
                if '#{' in re.sub(r'(?<!\\)#\{([^{}]*)\}', '', tail):
                    raise ValueError('nested or incomplete interpolation unsupported')
                offset += len(line)
                continue
        code = body[code_start:]
        if re.match(r'(?:if|unless|case|while|until|for|else|elsif|end|begin|rescue)\b', code) or re.search(r'\bdo\s*$', code):
            raise ValueError('Haml indentation-controlled Ruby blocks unsupported')
        continuation = bool(re.search(r'\s\|$', code))
        end = len(body) - (1 if continuation else 0)
        regions.append((offset + code_start, offset + end))
        offset += len(line)
    return regions


def units(text, encoding):
    if encoding == 'utf-8':
        return len(text.encode('utf-8'))
    if encoding == 'utf-16':
        return len(text.encode('utf-16-le')) // 2
    if encoding == 'utf-32':
        return len(text)
    raise ValueError('unsupported encoding')


def position(text, at, encoding):
    prefix = text[:at]
    return {'line': prefix.count('\n'), 'character': units(prefix.rsplit('\n', 1)[-1], encoding)}


def offset(text, pos, encoding):
    line, column = pos['line'], pos['character']
    if type(line) is not int or type(column) is not int or min(line, column) < 0:
        raise ValueError('malformed position')
    lines = text.split('\n')
    if line >= len(lines):
        raise ValueError('position outside document')
    content = lines[line].removesuffix('\r')
    consumed = 0
    for index in range(len(content) + 1):
        if consumed == column:
            return sum(len(part) + 1 for part in lines[:line]) + index
        if index < len(content):
            consumed += units(content[index], encoding)
    raise ValueError('position splits a character or exceeds line')


def provider_range(projection, wire_range, encoding):
    return projection.source_range(offset(projection.generated, wire_range['start'], encoding),
                                   offset(projection.generated, wire_range['end'], encoding))


def apply_provider_edit(projection, item, encoding, version, uri):
    if version != 1 or uri != 'fixture':
        raise ValueError('stale or foreign edit')
    if item.get('additionalTextEdits') or item.get('insertTextFormat', 1) != 1:
        raise ValueError('unsupported transaction or snippet')
    edit = item['textEdit']
    start, end = provider_range(projection, edit['range'], encoding)
    return projection.source[:start] + edit['newText'] + projection.source[end:]


def markup_project(source, language):
    if language == 'erb':
        # Only isolated tags/attributes in this experiment, never arbitrary template markup.
        if not re.fullmatch(r'\s*<[a-z][\w:-]*(?: [^<>%]*)?>?', source):
            raise ValueError('unsupported ERB markup context')
        return Projection(source, source, [(0, len(source), 0, len(source))])
    if language != 'haml':
        raise ValueError('unsupported markup language')
    prefix, separator, tail = source.rpartition('\n')
    base = len(prefix) + len(separator)
    match = re.fullmatch(r'( *)(%)([\w:-]+)(?:\(([^(){}]*)\))?', tail)
    if not match:
        raise ValueError('unsupported Haml markup context')
    indent, _, tag, attributes = match.groups()
    generated = indent + '<' + tag
    spans = [(base + match.start(3), base + match.end(3), len(indent) + 1, len(generated))]
    if attributes is not None:
        generated += ' ' + attributes + '>'
        spans.append((base + match.start(4), base + match.end(4), len(indent) + len(tag) + 2, len(generated) - 1))
    return Projection(source, generated, spans)


def apply_markup_edit(projection, item):
    if item.get('insertTextFormat', 1) == 2:
        text = item['textEdit']['newText']
        match = re.fullmatch(r'([a-z][\w:-]*)="\$1"', text)
        if not match:
            raise ValueError('unsupported markup snippet')
        # Materialize this one empty attribute placeholder; snippet navigation is outside the experiment.
        item = {**item, 'insertTextFormat': 1,
                'textEdit': {**item['textEdit'], 'newText': match[1] + '=""'}}
    return apply_provider_edit(projection, item, 'utf-16', 1, 'fixture')

#!/usr/bin/env python3
"""Crash/report smoke checks using disposable files, an owned PTY and a fake browser.

Build the abort probe with:
  cargo build --locked --profile dist -p lenz --no-default-features --example crash_probe
Pass the lenz-tui test executable emitted by cargo test -p lenz-tui --no-run.
No desktop input, network access or issue submission is performed.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import resource
import select
import struct
import subprocess
import tempfile
import termios
import time
from urllib.parse import parse_qs, urlparse


def no_core():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def draft(url):
    parsed = urlparse(url.strip())
    assert parsed.scheme == 'https' and parsed.netloc == 'github.com'
    assert parsed.path == '/aantoniadis/lenz/issues/new'
    query = parse_qs(parsed.query)
    assert query['title'] and 'Reproduction steps' in query['body'][0]
    assert 'original main probe failure' not in query['body'][0]
    assert 'Permission denied' not in query['body'][0]


class Pty:
    def __init__(self, command, root, env):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 0, 0))
        self.output = bytearray()
        self.proc = subprocess.Popen(command, cwd=root, env=env,
                                     stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                     start_new_session=True, preexec_fn=no_core)

    def read(self):
        if select.select([self.master], [], [], .02)[0]:
            self.output.extend(os.read(self.master, 65536))

    def wait(self, predicate):
        until = time.monotonic() + 15
        while not predicate():
            self.read()
            assert time.monotonic() < until, f'PTY timed out: {bytes(self.output[-1200:])!r}'

    def send(self, data):
        os.write(self.master, data)

    def restored(self):
        assert termios.tcgetattr(self.slave) == self.original, 'raw mode was not restored'
        for name, escape in [('alternate screen', b'\x1b[?1049l'), ('mouse', b'\x1b[?1000l'),
                             ('paste', b'\x1b[?2004l'), ('focus', b'\x1b[?1004l')]:
            assert escape in self.output, f'{name} was not restored'

    def close(self):
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(timeout=5)
        os.close(self.master)
        os.close(self.slave)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--panic-probe', type=Path, required=True)
    cli.add_argument('--tui-test-binary', type=Path, required=True)
    cli.add_argument('--gui', type=Path, default=Path('target/release/lenz'))
    cli.add_argument('--tui', type=Path, default=Path('target/release/lenz-tui'))
    cli.add_argument('--output', type=Path, required=True, help='New directory for raw evidence')
    args = cli.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    executables = {name: str(getattr(args, name).resolve()) for name in ['panic_probe', 'tui_test_binary', 'gui', 'tui']}
    (args.output / 'configuration.json').write_text(json.dumps(executables, indent=2))
    with tempfile.TemporaryDirectory(prefix='lenz-crash-check-') as directory:
        root = Path(directory)
        reports = root / 'reports'
        browser = root / 'browser'; browser.mkdir()
        opened = root / 'opened-url'
        for name in ['xdg-open', 'open']:
            stub = browser / name
            stub.write_text('#!/bin/sh\nprintf "%s" "$1" > "$LENZ_TEST_OPENED_URL"\n')
            stub.chmod(0o700)
        env = dict(os.environ, LENZ_LOG_DIR=str(reports), RUST_BACKTRACE='0',
                   TERM='xterm-256color', PATH=f'{browser}:{os.environ.get("PATH", "")}',
                   LENZ_TEST_OPENED_URL=str(opened), GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1')
        for mode in ['main', 'worker', 'error', 'unwritable']:
            log_dir = root / mode
            if mode == 'unwritable': log_dir.write_text('regular file, not a directory')
            result = subprocess.run([executables['panic_probe'], mode], cwd=root,
                                    env=dict(env, LENZ_LOG_DIR=str(log_dir)), capture_output=True,
                                    timeout=15, preexec_fn=no_core)
            (args.output / f'{mode}-stderr.log').write_bytes(result.stderr)
            assert result.returncode == (0 if mode == 'error' else -6), (mode, result.returncode)
            if mode == 'unwritable':
                assert b'Could not save diagnostics' in result.stderr and b'original main probe failure' in result.stderr
            else:
                files = list(log_dir.glob('*.log')); assert files
                text = files[0].read_text()
                (args.output / f'{mode}-report.log').write_text(text)
                if mode == 'error': assert 'Permission denied (os error 13)' in text and 'Next step:' in text
                else:
                    assert f'original {mode} probe failure' in text
                    assert 'crash_probe::' in text and 'Reload comparison' in text
                draft(text.split('Report on GitHub: ')[-1])
        # Seed a report once; retrieval must work from outside a repository/terminal.
        subprocess.run([executables['panic_probe'], 'error'], cwd=root, env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=15)
        for name in ['gui', 'tui']:
            for extra in [['--print'], []]:
                result = subprocess.run([executables[name], 'report', '--last', *extra], cwd=root,
                                        env=env, capture_output=True, timeout=15)
                assert result.returncode == 0, result.stderr
                output = result.stdout.decode()
                (args.output / f'{name}-cli-{len(extra)}.log').write_text(output)
                assert 'Diagnostic report:' in output
                draft(output.split('Report on GitHub: ')[-1])
                if not extra:
                    until = time.monotonic() + 5
                    while not opened.exists():
                        assert time.monotonic() < until
                        time.sleep(.01)
                    draft(opened.read_text()); opened.unlink()
        # Real Ratatui initialization and panic, rather than test-built terminal state.
        probe = Pty([executables['tui_test_binary'], '--exact', 'tests::terminal_panic_child', '--nocapture'],
                    root, dict(env, LENZ_TEST_TERMINAL_PANIC='1'))
        try:
            probe.wait(lambda: probe.proc.poll() is not None)
            while select.select([probe.master], [], [], .05)[0]: probe.read()
            assert probe.proc.returncode != 0
            probe.restored()
            assert probe.output.find(b'\x1b[?1049l') < probe.output.find(b'Diagnostic report:')
            (args.output / 'terminal-panic.log').write_bytes(probe.output)
        finally: probe.close()
        # Exercise actual terminal command menus against the fake browser.
        (root / 'left').write_text('OLD_REPORT_FIXTURE\n')
        (root / 'right').write_text('NEW_REPORT_FIXTURE\n')
        for name, command in [('comparison', ['--no-index', 'left', 'right']),
                              ('merge', ['merge', 'base', 'ours', 'theirs', 'result'])]:
            for file, content in [('base', 'base\n'), ('ours', 'ours\n'), ('theirs', 'theirs\n'), ('result', 'base\n')]:
                (root / file).write_text(content)
            probe = Pty([executables['tui'], *command], root, env)
            try:
                probe.wait(lambda: (b'Lenz' if name == 'comparison' else b'Merge') in probe.output)
                if name == 'comparison':
                    probe.wait(lambda: b'NEW_REPORT_FIXTURE' in probe.output)
                    probe.send(b':report last error\r')
                else:
                    probe.wait(lambda: b'ours' in probe.output)
                    probe.send(b':' + b'\x1b[B' * 100 + b'\x1b[A\r')
                probe.wait(opened.exists)
                draft(opened.read_text()); opened.unlink()
                assert (root / 'right').read_text() == 'NEW_REPORT_FIXTURE\n'
                assert (root / 'result').read_text() == 'base\n'
                probe.send(b'\x03')
                probe.wait(lambda: probe.proc.poll() is not None)
                probe.restored()
                (args.output / f'tui-{name}-menu.log').write_bytes(probe.output)
            finally: probe.close()
    print('PASS: abort diagnostics, failure fallback, GUI/TUI CLI, terminal panic cleanup and real comparison/merge report menus')


if __name__ == '__main__':
    main()

#!/usr/bin/env python3
"""Real terminal repository workflows and timings; owns its PTY and disposable Git repo."""
import argparse
import codecs
import datetime
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import unicodedata
from perf_support import isolate_user_state

_spec = importlib.util.spec_from_file_location('tui_bench', Path(__file__).with_name('tui-bench.py'))
_bench = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_bench)
TerminalProbe = _bench.TerminalProbe
# This line exists only after commit details load, never in the background log.
COMMIT_DETAILS_MARKER = 'Workflow Author <workflow@example.invalid>'


class Screen:
    """Small VT reader for ratatui's cursor-addressed updates, including UTF-8 split reads.

    This checks reconstructed cells, so old output and partially updated strings cannot
    satisfy assertions. It is intentionally not a general terminal emulator.
    """
    def __init__(self, width=120, height=36):
        self.width, self.height = width, height
        self.cells = [[' '] * width for _ in range(height)]
        self.x = self.y = 0
        self.offset = 0
        self.pending = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')

    def update(self, probe):
        data = bytes(probe.output[self.offset:])
        self.offset = len(probe.output)
        self.pending += self.decoder.decode(data)
        i = 0
        while i < len(self.pending):
            ch = self.pending[i]
            if ch == '\x1b':
                if i + 1 == len(self.pending):
                    break
                if self.pending[i + 1] == '[':
                    end = i + 2
                    while end < len(self.pending) and not ('@' <= self.pending[end] <= '~'):
                        end += 1
                    if end == len(self.pending):
                        break
                    self.csi(self.pending[i + 2:end], self.pending[end])
                    i = end + 1
                    continue
                if self.pending[i + 1] == ']':
                    end = self.pending.find('\x07', i + 2)
                    st = self.pending.find('\x1b\\', i + 2)
                    if end < 0 and st < 0:
                        break
                    i = min(end + 1 if end >= 0 else len(self.pending) + 1,
                            st + 2 if st >= 0 else len(self.pending) + 1)
                    continue
                i += 2
                continue
            if ch == '\r':
                self.x = 0
            elif ch == '\n':
                self.y = min(self.y + 1, self.height - 1)
            elif ch == '\b':
                self.x = max(0, self.x - 1)
            elif ord(ch) >= 32 and not unicodedata.combining(ch):
                if self.x >= self.width:
                    self.x = 0
                    self.y = min(self.y + 1, self.height - 1)
                self.cells[self.y][self.x] = ch
                width = 2 if unicodedata.east_asian_width(ch) in ('W', 'F') else 1
                if width == 2 and self.x + 1 < self.width:
                    self.cells[self.y][self.x + 1] = ''
                self.x += width
            i += 1
        self.pending = self.pending[i:]

    def csi(self, params, final):
        if params.startswith('?'):
            return
        values = [int(p) if p.isdigit() else 0 for p in params.split(';')]
        first = values[0] or 1
        if final in 'Hf':
            self.y = min(max(first - 1, 0), self.height - 1)
            self.x = min(max((values[1] if len(values) > 1 else 1) - 1, 0), self.width - 1)
        elif final == 'G':
            self.x = min(first - 1, self.width - 1)
        elif final == 'd':
            self.y = min(first - 1, self.height - 1)
        elif final == 'A':
            self.y = max(0, self.y - first)
        elif final == 'B':
            self.y = min(self.height - 1, self.y + first)
        elif final == 'C':
            self.x = min(self.width - 1, self.x + first)
        elif final == 'D':
            self.x = max(0, self.x - first)
        elif final in 'EF':
            self.y = min(self.height - 1, self.y + first) if final == 'E' else max(0, self.y - first)
            self.x = 0
        elif final == 'b':
            previous = self.cells[self.y][max(0, self.x - 1)]
            for _ in range(first):
                if self.x < self.width:
                    self.cells[self.y][self.x] = previous
                    self.x += 1
        elif final == 'J':
            value = values[0]
            if value in (2, 3):
                self.cells = [[' '] * self.width for _ in range(self.height)]
            elif value == 0:
                self.cells[self.y][self.x:] = [' '] * (self.width - self.x)
                for row in range(self.y + 1, self.height):
                    self.cells[row] = [' '] * self.width
        elif final == 'K':
            value = values[0]
            start, end = (0, self.width) if value == 2 else ((0, self.x + 1) if value == 1 else (self.x, self.width))
            self.cells[self.y][start:end] = [' '] * (end - start)

    def text(self):
        return '\n'.join(''.join(row) for row in self.cells)


class Workflow:
    def __init__(self, app, root, args, scenario="repository"):
        self.scenario = scenario
        self.probe = TerminalProbe([app, *args], root)
        self.screen = Screen()
        self.metrics = {}

    def wait(self, *texts, absent=(), timeout=15):
        started = time.monotonic()
        deadline = started + timeout
        while True:
            self.screen.update(self.probe)
            visible = self.screen.text()
            if all(text in visible for text in texts) and all(text not in visible for text in absent):
                self.probe.sample()
                return (time.monotonic() - started) * 1000
            if self.probe.proc.poll() is not None or time.monotonic() > deadline:
                Path('/tmp/lenz-tui-workflow-failed.ansi').write_bytes(self.probe.output)
                raise AssertionError(f'expected {texts}, absent {absent}; exit={self.probe.proc.poll()}\n{visible}')
            self.probe.read(.01)
            self.probe.sample()

    def key(self, data, *texts, absent=(), metric=None):
        self.probe.quiet(.04)
        self.screen.update(self.probe)
        started = time.monotonic()
        os.write(self.probe.master, data)
        self.wait(*texts, absent=absent)
        if metric:
            self.metrics.setdefault(metric, []).append((time.monotonic() - started) * 1000)

    def menu(self, text, *expected, metric=None, repository=False):
        # Separate Escape from the next key: terminals distinguish a literal Escape
        # by timeout, and rapid ESC+letter is intentionally decoded as Alt-letter.
        # F10 opens the shared command palette in every pane; ':' is list-search text. `repository` is kept for
        # callers: the palette lists the repository commands by their labels.
        self.key(b'\x1b[21~', 'Commands:')
        self.key(text.encode(), text)
        self.key(b'\r', *expected, metric=metric)

    def history_filter(self, text, *expected, absent=(), metric=None):
        self.menu('filter history', 'Log filter:', repository=True)
        self.key(text.encode() + b'\r', *expected, absent=absent, metric=metric)

    def close_overlay(self):
        self.key(b'\x1b')
        self.probe.quiet(.1)

    def finish(self):
        self.probe.quiet(.1)
        stats = self.probe.sample()
        if stats['rss_mib'] is not None:
            self.metrics['settled-rss'] = [stats['rss_mib']]
        if self.probe.peak is not None:
            self.metrics['peak-rss'] = [self.probe.peak]
        self.probe.finish()
        for name, samples in self.metrics.items():
            unit = 'MiB' if name.endswith('rss') else 'ms'
            metric = self.scenario + '-' + name if name.endswith('rss') else name
            print('LENZ_PERF ' + json.dumps({'name': 'tui/pty/workflow/' + metric, 'unit': unit, 'samples': samples}), flush=True)


def git(root, *args, expected=0):
    deadline = time.monotonic()+2
    while True:
        result = subprocess.run(['git', '-c', 'user.name=Workflow Author', '-c', 'user.email=workflow@example.invalid',
                                 '-c', 'commit.gpgsign=false', *args], cwd=root, capture_output=True,
                                env=dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1'))
        if result.returncode != 128 or b'index.lock' not in result.stderr or b'File exists' not in result.stderr or time.monotonic() >= deadline:
            break
        # The child may still be finishing its read-only status refresh. This
        # fixture setup happens before the measured refresh/input boundary.
        time.sleep(.02)
    if result.returncode != expected:
        raise RuntimeError(f'git {args}: exit {result.returncode}, expected {expected}: {result.stderr.decode()}')
    return result.stdout.decode().strip()


def fixture(root, lines):
    git(root, 'init', '-q', '-b', 'main')
    for number, subject in [(1, 'Workflow first commit'), (2, 'Workflow second commit')]:
        for name in ['file.rs', 'other.rs']:
            (root / name).write_text(f'const LABEL: &str = "WORKFLOW_{number} needle";\n' +
                                    ''.join(f'const ITEM_{i}: &str = "needle café";\n' for i in range(lines)))
        git(root, 'add', '.')
        git(root, 'commit', '-qm', subject)
    git(root, 'branch', 'topic')
    return git(root, 'rev-parse', 'HEAD')


def author_date(timestamp):
    """Blame displays the author's UTC date, independent of Git's local timezone."""
    return datetime.datetime.fromtimestamp(int(timestamp), datetime.timezone.utc).date().isoformat()


def repository_workflow(app, root, lines, rounds):
    head = fixture(root, lines)
    date = author_date(git(root, 'show', '-s', '--format=%at', head))
    matches = f'{min(4 * (lines + 1), 10000)} matches'
    workflow = Workflow(app, root, ['log'])
    try:
        workflow.metrics['repository-ready'] = [workflow.wait('Workflow second commit', 'Workflow first commit', 'WORKFLOW_2')]
        workflow.history_filter('first', 'Workflow first commit', absent=('Workflow second commit',), metric='history-filter')
        workflow.close_overlay()  # clear log filter
        workflow.wait('Workflow second commit', 'Workflow first commit')
        # Fresh external commit and refs, then real repository menu refresh.
        (root / 'new-file').write_text('new local file\n')
        git(root, 'add', 'new-file')
        git(root, 'commit', '-qm', 'Workflow refreshed commit')
        git(root, 'tag', 'workflow-tag')
        workflow.menu('refresh repository', 'Workflow refreshed commit', metric='repository-refresh', repository=True)
        # Select the Rust commit explicitly. Focus cycles Log -> Files -> Left.
        workflow.key(b'Workflow second commit\r', 'WORKFLOW_2')
        workflow.close_overlay()  # leave committed pane search before cycling focus
        workflow.key(b'\t\t', 'WORKFLOW_2')
        workflow.menu('Find in Change Set', 'Find in Change Set:')
        workflow.key(b'needle\r', 'Find in Change Set ·', matches, 'needle', metric='change-set-search')
        workflow.key(b'\r', 'needle')
        workflow.close_overlay()
        workflow.key(b'\t')
        workflow.key(b'b', head[:7] + ' ' + date, metric='blame-ready')
        workflow.key(b'g1\r', head[:7] + ' ' + date)
        workflow.key(b'B', COMMIT_DETAILS_MARKER, 'Workflow second commit', 'Commit', metric='commit-inspection')
        workflow.key(b'd', 'view 2/2', metric='new-independent-view')
        workflow.key(b'\x1b[19~', 'view 1/2', metric='view-switch')  # F8
        workflow.key(b'\x1b[19~', 'view 2/2')
        workflow.key(b'q', absent=('view 2/2', 'view 1/2'))
        workflow.close_overlay()
        # Repeated real search/query cancellation checks absence of stale panel.
        for _ in range(rounds):
            workflow.menu('Find in Change Set', 'Find in Change Set:')
            workflow.key(b'needle\r', matches, metric='search-stress')
            workflow.close_overlay()
            workflow.wait(absent=('Find in Change Set:', 'Find in Change Set ·'))
            workflow.menu('Find in Change Set', 'Find in Change Set:')
            workflow.key(b'needle\r')
            workflow.key(b'\x1b', absent=('Find in Change Set:', 'Find in Change Set ·'), metric='search-cancel')
            sample = workflow.probe.sample()
            if sample['rss_mib'] is not None:
                workflow.metrics.setdefault('search-stress-rss', []).append(sample['rss_mib'])
        workflow.menu('Find in Change Set', 'Find in Change Set:')
        workflow.key(b'\x1bc', 'changed lines only: on')
        workflow.key(b'needle\r', '4 matches', 'changed lines only', metric='changed-only-search')
        workflow.close_overlay()
        # Commit modal can be reopened after stress; cancelled work must not corrupt it.
        workflow.key(b'B', COMMIT_DETAILS_MARKER, 'Commit')
        workflow.close_overlay()
        # Include About only after the existing measured workflows. Readiness requires
        # author/version rows absent from the toolbar and the command menu.
        for _ in range(rounds):
            workflow.key(b'\x1b[21~', 'Commands:')
            workflow.key(b'about', 'About Lenz')
            workflow.key(b'\r', 'About Lenz', 'Version:', 'MIT; engine Apache-2.0', metric='about-open')
            workflow.key(b'w', 'About Lenz', 'MIT; engine Apache-2.0')
            workflow.key(b'\x1b', absent=('About Lenz', 'MIT; engine Apache-2.0'), metric='about-close')
            workflow.probe.quiet(.1)

        workflow.finish()
    finally:
        workflow.probe.close()


def conflict_workflow(app, root):
    git(root, 'init', '-q', '-b', 'main')
    (root / 'file').write_text('base\n')
    git(root, 'add', 'file'); git(root, 'commit', '-qm', 'Conflict base')
    git(root, 'checkout', '-qb', 'side')
    (root / 'file').write_text('side\n'); git(root, 'commit', '-qam', 'Conflict side')
    git(root, 'checkout', '-q', 'main')
    (root / 'file').write_text('main\n'); git(root, 'commit', '-qam', 'Conflict main')
    workflow = Workflow(app, root, ['log'], scenario='conflict')
    try:
        workflow.wait('Conflict main', 'Repository')
        git(root, 'merge', 'side', expected=1)
        workflow.menu('refresh repository', '1 of 1 left', metric='conflict-source-refresh', repository=True)
        # First sidebar row is the conflict source (one-based terminal coordinates).
        workflow.screen.update(workflow.probe)
        row = next(y + 1 for y, line in enumerate(workflow.screen.text().splitlines()) if '1 of 1 left' in line)
        workflow.key(f'\x1b[<0;4;{row}M\x1b[<0;4;{row}m'.encode(), '1 conflict left', 'Result', metric='open-conflicts')
        (root / 'file').write_text('resolved\n')
        git(root, 'add', 'file')
        workflow.key(b'\x1b[I')  # owned terminal focus reporting, no desktop focus
        workflow.key(b'\x12', 'Repository', 'All resolved', 'git merge --continue', metric='conflict-return-refresh')
        assert (root / 'file').read_text() == 'resolved\n'
        assert not git(root, 'ls-files', '-u'), 'return must preserve externally staged resolution'
        workflow.finish()
    finally:
        workflow.probe.close()


def main():
    isolate_user_state()  # never measure with the user's own settings or saved positions
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--lines', type=int, default=2000)
    parser.add_argument('--rounds', type=int, default=11)
    args = parser.parse_args()
    if args.lines < 1 or args.rounds < 1:
        parser.error('lines and rounds must be positive')
    with tempfile.TemporaryDirectory(prefix='lenz-tui-workflow-') as directory:
        root = Path(directory)
        repository = root / 'repository'; repository.mkdir()
        conflict = root / 'conflict'; conflict.mkdir()
        repository_workflow(str(args.app.resolve()), repository, args.lines, args.rounds)
        conflict_workflow(str(args.app.resolve()), conflict)
    print('Owned PTY workflows passed: history/filter/refresh/search/blame/modal/tabs/cancellation/conflict return. Timings end at reconstructed terminal cells, not physical display.')


if __name__ == '__main__':
    main()

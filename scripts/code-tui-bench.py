#!/usr/bin/env python3
"""Owned-PTY reader correctness/performance probe. Latency ends at PTY bytes, not displayed pixels.

  scripts/code-tui-bench.py --binary target/release/lenz-reader-tui --out perf-results/code-tui --lines 100000 --rounds 30 --stress-seconds 10

Uses disposable documents and private XDG/provider state; HOME is inherited. Linux reads /proc; macOS samples ps RSS,
not physical footprint. Sampled RSS and the process high-water mark do not measure GPU memory or prove no leak.
"""
import argparse
import base64
import codecs
from contextlib import suppress
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unicodedata


def parse_linux(status, stat, ticks):
    def memory(name):
        match = re.search(rf'^{name}:\s+(\d+)\s+kB\s*$', status, re.M)
        return int(match[1]) / 1024 if match else None
    cpu = None
    try:
        fields = stat.rsplit(')', 1)[1].split()
        cpu = (int(fields[11]) + int(fields[12])) / ticks
    except (IndexError, ValueError, ZeroDivisionError):
        pass
    return dict(rss_mib=memory('VmRSS'), peak_rss_mib=memory('VmHWM'), cpu_seconds=cpu)


def parse_ps(text):
    result = dict(rss_mib=None, peak_rss_mib=None, cpu_seconds=None)
    try:
        rss, clock = text.split()
        result['rss_mib'] = int(rss) / 1024
        days, clock = clock.split('-', 1) if '-' in clock else ('0', clock)
        parts = [float(p) for p in clock.split(':')]
        result['cpu_seconds'] = int(days) * 86400 + sum(n * 60 ** i for i, n in enumerate(reversed(parts)))
    except (ValueError, IndexError):
        pass
    return result


def process_stats(pid):
    if sys.platform.startswith('linux'):
        try:
            return parse_linux(Path(f'/proc/{pid}/status').read_text(), Path(f'/proc/{pid}/stat').read_text(), os.sysconf('SC_CLK_TCK'))
        except OSError:
            return parse_linux('', '', 1)
    try:
        result = subprocess.run(['ps', '-p', str(pid), '-o', 'rss=,time='], text=True, capture_output=True, timeout=2)
        return parse_ps(result.stdout)
    except (OSError, subprocess.TimeoutExpired):
        return parse_ps('')


class Screen:
    """Minimal VT cell view for deterministic assertions on Ratatui's incremental output."""
    def __init__(self, width, height):
        self.width, self.height = width, height
        self.cells = [[' '] * width for _ in range(height)]
        self.row = self.col = 0
        self.pending = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')(errors='replace')

    def text(self):
        return '\n'.join(''.join(row) for row in self.cells)

    def feed(self, data):
        text = self.pending + self.decoder.decode(data)
        self.pending = ''
        i = 0
        while i < len(text):
            char = text[i]
            if char == '\x1b':
                if i + 1 >= len(text):
                    break
                if text[i + 1] == '[':
                    match = re.match(r'\x1b\[([0-?]*)([ -/]*)([@-~])', text[i:])
                    if not match:
                        break
                    self.csi(match[1], match[3])
                    i += match.end()
                    continue
                if text[i + 1] == ']':
                    match = re.search(r'\x07|\x1b\\', text[i + 2:])
                    if not match:
                        break
                    i += 2 + match.end()
                    continue
                i += 2
                continue
            if char == '\r':
                self.col = 0
            elif char == '\n':
                self.row = min(self.height - 1, self.row + 1)
            elif char == '\b':
                self.col = max(0, self.col - 1)
            elif char >= ' ' and char != '\x7f':
                width = 0 if unicodedata.combining(char) else 2 if unicodedata.east_asian_width(char) in ('W', 'F') else 1
                if 0 <= self.row < self.height and 0 <= self.col < self.width:
                    self.cells[self.row][self.col] = char
                    if width == 2 and self.col + 1 < self.width:
                        self.cells[self.row][self.col + 1] = ''
                self.col += width
            i += 1
        self.pending = text[i:]

    def csi(self, args, command):
        if args.startswith(('?', '>', '<', '=')):
            return
        values = [int(v or 0) for v in args.split(';')]
        first = values[0] or 1
        if command in ('H', 'f'):
            self.row = first - 1
            self.col = (values[1] or 1) - 1 if len(values) > 1 else 0
        elif command == 'G': self.col = first - 1
        elif command == 'd': self.row = first - 1
        elif command == 'A': self.row = max(0, self.row - first)
        elif command == 'B': self.row += first
        elif command == 'C': self.col += first
        elif command == 'D': self.col = max(0, self.col - first)
        elif command == 'J' and values[0] in (2, 3):
            self.cells = [[' '] * self.width for _ in range(self.height)]
        elif command == 'K' and self.row < self.height:
            start = 0 if values[0] in (1, 2) else min(self.width, self.col)
            end = min(self.width, self.col + 1) if values[0] == 1 else self.width
            self.cells[self.row][start:end] = [' '] * (end - start)


class TerminalProbe:
    def __init__(self, command, cwd, env=None):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.output = bytearray()
        self.samples = []
        self.peak = None
        self.restored = None
        self.started = time.monotonic()
        self.screen = Screen(120, 36)
        self.resize(120, 36)
        try:
            self.proc = subprocess.Popen(command, cwd=cwd, stdin=self.slave, stdout=self.slave, stderr=self.slave,
                start_new_session=True, preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0),
                env=dict(os.environ, TERM='xterm-256color', **(env or {})))
        except BaseException:
            os.close(self.master)
            os.close(self.slave)
            raise

    def resize(self, width, height):
        self.screen = Screen(width, height)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
        if hasattr(self, 'proc'):
            self.proc.send_signal(signal.SIGWINCH)

    def sample(self):
        sample = dict(at_seconds=time.monotonic() - self.started, **process_stats(self.proc.pid))
        self.samples.append(sample)
        peak = sample['peak_rss_mib'] or sample['rss_mib']
        if peak is not None:
            self.peak = max(self.peak or peak, peak)
        return sample

    def read(self, timeout=.01):
        if select.select([self.master], [], [], max(0, timeout))[0]:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                data = b''
            before = len(self.output)
            self.output.extend(data)
            self.screen.feed(data)
            # Answer complete toolkit queries, including ones split between PTY reads, once each.
            start = max(0, before - 8)
            for query in re.finditer(rb'\x1b\[(?:0)?c|\x1b\[\?u', self.output[start:]):
                if start + query.end() > before:
                    os.write(self.master, b'\x1b[?0u' if query[0].endswith(b'u') else b'\x1b[?62;22c')
            return data
        return b''

    def expect(self, text, timeout=15, raw=False, since=0):
        deadline = time.monotonic() + timeout
        while True:
            haystack = bytes(self.output[since:]) if raw else self.screen.text()
            if text in haystack:
                return (time.monotonic() - self.started) * 1000
            if self.proc.poll() is not None:
                self.drain_until_exit(0)
                haystack = bytes(self.output[since:]) if raw else self.screen.text()
                if text in haystack:
                    return (time.monotonic() - self.started) * 1000
                raise RuntimeError(f'terminal did not show {text!r}; exit={self.proc.poll()}; screen={self.screen.text()[-3000:]!r}')
            if time.monotonic() >= deadline:
                raise RuntimeError(f'terminal did not show {text!r}; exit={self.proc.poll()}; screen={self.screen.text()[-3000:]!r}')
            self.read(.01)
            if not self.samples or time.monotonic() - self.started - self.samples[-1]['at_seconds'] >= .05:
                self.sample()

    def quiet(self, duration=.04, timeout=5):
        deadline = time.monotonic() + timeout
        while self.read(duration):
            if time.monotonic() >= deadline:
                raise RuntimeError('terminal kept writing while idle')

    def input_latency(self, keys):
        self.quiet()
        started = time.monotonic()
        os.write(self.master, keys)
        if not self.read(5):
            raise RuntimeError(f'no terminal feedback for {keys!r}')
        latency = (time.monotonic() - started) * 1000
        self.quiet()
        self.sample()
        return latency

    def send(self, keys):
        os.write(self.master, keys)

    def finish(self, keys=b'\x11', expected=0):
        self.send(keys)
        if not self.drain_until_exit(5):
            raise RuntimeError('terminal failed to quit')
        self.quiet()
        try:
            self.restored = termios.tcgetattr(self.slave) == self.original
        except termios.error:
            if sys.platform != 'darwin':
                raise
            self.restored = None  # macOS can revoke the slave: termios restoration is then unobservable.
        if self.restored is False or b'\x1b[?1049l' not in self.output:
            raise RuntimeError('terminal raw-mode or alternate-screen restoration failed')
        if self.proc.returncode != expected:
            raise RuntimeError(f'exit {self.proc.returncode}, expected {expected}')

    def drain_until_exit(self, seconds):
        deadline = time.monotonic() + seconds
        while self.proc.poll() is None and time.monotonic() < deadline:
            self.read(.02)
        # Drain after exit too: unread final bytes contain the restoration commands.
        drain_deadline = time.monotonic() + .2
        while time.monotonic() < drain_deadline and self.read(0):
            pass
        return self.proc.poll() is not None

    def close(self):
        if self.master is None:
            return
        if self.proc.poll() is None:
            for sig, timeout in [(signal.SIGTERM, 1), (signal.SIGKILL, 2)]:
                with suppress(ProcessLookupError, PermissionError):
                    os.killpg(self.proc.pid, sig)
                if self.drain_until_exit(timeout):
                    break
            if self.proc.poll() is None:
                raise RuntimeError('owned child could not be reaped')
        os.close(self.master)
        os.close(self.slave)
        self.master = self.slave = None


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def identity(binary):
    root = Path(__file__).resolve().parents[1]
    def git(*args):
        return subprocess.run(['git', *args], cwd=root, text=True, capture_output=True).stdout.strip()
    paths = sorted([*root.glob('Cargo.*'), *root.glob('crates/*/Cargo.toml'), *root.glob('crates/**/*.rs')])
    digest = hashlib.sha256()
    for path in paths:
        if path.is_file():
            digest.update(str(path.relative_to(root)).encode() + b'\0' + path.read_bytes())
    return dict(git_commit=git('rev-parse', 'HEAD'), git_status=git('status', '--porcelain'), source_sha256=digest.hexdigest(),
                executable=str(binary), executable_sha256=sha(binary), executable_bytes=binary.stat().st_size,
                executable_mtime_ns=binary.stat().st_mtime_ns, collected_at_utc=time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                collector_sha256=sha(Path(__file__)), system=platform.platform(), machine=platform.machine(), python=sys.version,
                cpu_clock_ticks=os.sysconf('SC_CLK_TCK'),
                rss_semantics='Linux VmRSS/VmHWM' if sys.platform.startswith('linux') else 'ps resident size; peak is sampled RSS')


def exercise(probe, fixture, original, args):
    metrics = {}
    checks = []
    metrics['first_feedback_ms'] = probe.expect(fixture.name)
    metrics['first_content_ms'] = probe.expect('READER_LINE_000000')
    checks.append('first filename feedback and generated content')
    probe.quiet(.1)
    # Opening is a reading viewport with no caret until the user chooses a source location.
    probe.send(b'\x071\r')
    probe.quiet()
    # A filtered disabled command stays in the modal and reports its shared disabled reason.
    probe.send(b'\x1b[21~File Outline')
    probe.expect('No outline for this language')
    probe.send(b'\r')
    probe.quiet()
    if 'Commands' not in probe.screen.text():
        raise RuntimeError('disabled command dismissed its palette')
    checks.append('palette filtering and disabled reason')
    # Go-to/copy keys and text underneath an active palette must stay captured by it.
    probe.send(b'\x07\x03\x1b[6~')
    probe.quiet()
    if 'Commands' not in probe.screen.text():
        raise RuntimeError('palette leaked input')
    probe.send(b'\x1b')
    probe.quiet()
    probe.expect('READER_LINE_000000')
    before = len(probe.output)
    probe.send(b'\x03')
    clip = b'\x1b]52;c;' + base64.b64encode(original.splitlines(keepends=True)[0]) + b'\x07'
    probe.expect(clip, raw=True, since=before)
    checks.append('modal input capture and OSC52 source-line copy')
    target = min(args.lines - 1, max(1, args.lines // 2))
    probe.send(b'\x07' + str(target + 1).encode() + b'\r')
    probe.expect(f'READER_LINE_{target:06d}')
    probe.send(b'\x06READER_LINE_000000\r')
    probe.expect('READER_LINE_000000')
    probe.send(b'\x1b')
    probe.quiet()
    checks.append('goto and find navigate to known lines')
    if fixture.read_bytes() != original:
        raise RuntimeError('reading, navigation or copy changed fixture bytes')
    # New Window creates an independent terminal view; F8 returns to the original document.
    probe.send(b'\x1b[21~New Window\r')
    probe.expect('View 2/2')
    probe.quiet()
    if 'READER_LINE_' in probe.screen.text():
        raise RuntimeError('new independent view inherited the first view document')
    probe.send(b'\x1b[19~')
    probe.expect('View 1/2')
    probe.expect('READER_LINE_000000')
    probe.send(b'\x1b[19~')
    probe.expect('View 2/2')
    probe.send(b'\x11')
    probe.expect('READER_LINE_000000')
    checks.append('New Window and F8 independent sessions; Ctrl-Q closes one view')
    # Deliberate external change followed by explicit reload; every other byte must stay exact.
    updated = original.replace(b'READER_LINE_000000', b'READER_RELOADED_000', 1)
    fixture.write_bytes(updated)
    probe.send(b'\x1b[15~')
    probe.expect('READER_RELOADED_000')
    checks.append('reload exposes external file change')
    probe.quiet(.25)
    baseline = probe.sample()
    before_bytes = len(probe.output)
    idle_started = time.monotonic()
    while time.monotonic() - idle_started < .5:
        probe.read(.02)
    after = probe.sample()
    metrics['idle_seconds'] = time.monotonic() - idle_started
    metrics['idle_output_bytes'] = len(probe.output) - before_bytes
    metrics['idle_cpu_seconds'] = None if baseline['cpu_seconds'] is None or after['cpu_seconds'] is None else after['cpu_seconds'] - baseline['cpu_seconds']
    if metrics['idle_output_bytes']:
        raise RuntimeError(f'idle terminal emitted {metrics["idle_output_bytes"]} bytes')
    metrics['input_first_byte_ms'] = [probe.input_latency(b'\x1b[B' if i % 2 == 0 else b'\x1b[A') for i in range(args.rounds)]
    metrics['resize_first_byte_ms'] = []
    for width, height in [(40, 12), (12, 4), (1, 1), (200, 60), (120, 36)]:
        probe.quiet()
        started = time.monotonic()
        probe.resize(width, height)
        if not probe.read(5):
            raise RuntimeError('resize did not repaint')
        metrics['resize_first_byte_ms'].append(dict(width=width, height=height, ms=(time.monotonic() - started) * 1000))
        probe.quiet()
        probe.sample()
    checks.append('resize including 1x1 without exit')
    # Every turn forces a different viewport; both key handling and actual rendering run under sustained load.
    stress_started = time.monotonic()
    cycles = 0
    stress_samples = []
    while time.monotonic() - stress_started < args.stress_seconds:
        stress_samples.append(probe.input_latency(b'\x1b[6~' if cycles % 2 == 0 else b'\x1b[5~'))
        cycles += 1
    metrics['stress_seconds'] = time.monotonic() - stress_started
    metrics['stress_cycles'] = cycles
    metrics['stress_input_first_byte_ms'] = stress_samples
    probe.quiet(.25)
    settled = probe.sample()
    metrics['settled_rss_mib'] = settled['rss_mib']
    metrics['peak_rss_mib'] = probe.peak
    if fixture.read_bytes() != updated:
        raise RuntimeError('reader changed fixture bytes after reload/stress')
    checks.append('file bytes preserved before and after intentional external modification')
    # Dismiss the palette before requesting reader closure (modal-first since ticket14).
    probe.send(b'\x1b[21~')
    probe.expect('Commands')
    finish_from_palette(probe)
    checks.append('Escape dismisses palette; Ctrl-Q restores alternate screen and observable termios')
    return metrics, checks


def isolated_environment(root):
    # TerminalProbe inherits HOME; isolate application files through XDG only.
    return dict(XDG_CONFIG_HOME=str(root / 'config'), XDG_STATE_HOME=str(root / 'state'),
                XDG_CACHE_HOME=str(root / 'cache'), XDG_DATA_HOME=str(root / 'data'), PATH='/nonexistent',
                LENZ_THEME='darcula', LENZ_APPEARANCE='', LENZ_READER_THEME='', LENZ_READER_APPEARANCE='', NO_COLOR='')


def finish_from_palette(probe):
    probe.send(b'\x1b')
    probe.quiet()
    probe.finish()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', '--app', dest='binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--lines', type=int, default=100000)
    parser.add_argument('--rounds', type=int, default=30)
    parser.add_argument('--stress-seconds', type=float, default=10)
    args = parser.parse_args()
    if args.lines < 2 or args.rounds < 1 or args.stress_seconds <= 0:
        parser.error('lines must be >= 2; rounds and stress-seconds must be positive')
    binary = args.binary.resolve(strict=True)
    args.out.mkdir(parents=True, exist_ok=False)
    result = dict(schema=3, exit_policy='escape-palette-then-ctrl-q', environment_revision='inherited-home-private-xdg-v2', identity=identity(binary), configuration=dict(lines=args.lines, rounds=args.rounds, requested_stress_seconds=args.stress_seconds),
                  latency_semantics='first PTY bytes observed after input/resize; not physical display latency', status='failed')
    probe = None
    try:
        with tempfile.TemporaryDirectory(prefix='reader-tui-bench-') as directory:
            root = Path(directory)
            fixture = root / 'fixture' / 'reader.txt'
            fixture.parent.mkdir()
            original = ''.join(f'READER_LINE_{i:06d} plain reader fixture {i}\n' for i in range(args.lines)).encode()
            fixture.write_bytes(original)
            config = root / 'config' / 'lenz-reader'
            config.mkdir(parents=True)
            (config / 'providers.json').write_text('{"providers": {}}\n')
            env = isolated_environment(root)
            result['fixture'] = dict(original_sha256=sha(fixture), bytes=len(original), lines=args.lines, name=fixture.name, xdg_isolated=True,
                                     providers='plain text document; isolated providers.json; PATH=/nonexistent')
            probe = TerminalProbe([str(binary), str(fixture)], fixture.parent, env)
            result['metrics'], result['checks'] = exercise(probe, fixture, original, args)
            result['fixture']['final_sha256'] = sha(fixture)
            result['status'] = 'passed'
    except BaseException as error:
        result['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        if probe:
            try:
                probe.close()
            except Exception as error:
                result['cleanup_error'] = f'{type(error).__name__}: {error}'
                result['status'] = 'failed'
            finally:
                result['process_samples'] = probe.samples
                result['termios_restored'] = probe.restored
                result['exit_code'] = probe.proc.returncode
                (args.out / 'terminal.raw').write_bytes(probe.output)
                (args.out / 'screen.txt').write_text(probe.screen.text())
        (args.out / 'samples.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps({k: result[k] for k in ('status', 'metrics', 'checks', 'error', 'cleanup_error') if k in result}, indent=2))
        if 'cleanup_error' in result:
            raise RuntimeError(result['cleanup_error'])


if __name__ == '__main__':
    main()

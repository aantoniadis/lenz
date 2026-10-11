#!/usr/bin/env python3
"""Owned-PTY terminal smoke/performance probe (Linux/macOS; no desktop input)."""
from contextlib import suppress
import argparse
import fcntl
import json
import os
from pathlib import Path
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

from perf_support import process_stats, isolate_user_state


class TerminalProbe:
    # Opt-in: answer the terminal queries toolkits block on (primary DA, DECRQM, text-area size),
    # as a real emulator does within milliseconds. Off for the suite's own metrics.
    answer_queries = False

    def __init__(self, command, cwd):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.resize(120, 36)
        self.output = bytearray()
        self.peak = None
        self.started = time.monotonic()
        self.proc = subprocess.Popen(command, cwd=cwd, stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                     start_new_session=True,
                                     # Make the PTY the controlling terminal, as a real emulator does (tools may open /dev/tty).
                                     preexec_fn=lambda: fcntl.ioctl(0, termios.TIOCSCTTY, 0),
                                     env=dict(os.environ, TERM='xterm-256color', GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1',
                                              # a stray `o` must never start the user's editor
                                              LENZ_EDITOR='true', VISUAL='true', EDITOR='true'))

    def resize(self, width, height):
        self.size = (height, width)
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', height, width, 0, 0))
        if hasattr(self, 'proc'):
            self.proc.send_signal(signal.SIGWINCH)

    def read(self, timeout):
        if select.select([self.master], [], [], timeout)[0]:
            try:
                data = os.read(self.master, 65536)
            except OSError:
                data = b''
            self.output.extend(data)
            if self.answer_queries and data:
                self.answer(data)
            return data
        return b''

    def answer(self, data):
        replies = []
        if b'\x1b[c' in data or b'\x1b[0c' in data:
            replies.append(b'\x1b[?62;22c')
        if b'\x1b[18t' in data:
            replies.append(b'\x1b[8;%d;%dt' % self.size)
        replies += [b'\x1b[?%s;0$y' % mode for mode in re.findall(rb'\x1b\[\?(\d+)\$p', data)]
        if replies:
            os.write(self.master, b''.join(replies))

    def sample(self):
        sample = process_stats(self.proc.pid)
        peak = sample['peak_rss_mib'] or sample['rss_mib']
        if peak is not None:
            self.peak = max(self.peak or 0, peak)
        return sample

    def expect(self, text, timeout=15):
        deadline = time.monotonic() + timeout
        while text not in self.output:
            if self.proc.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError(f'terminal did not emit {text!r}; exit={self.proc.poll()}; tail={bytes(self.output[-1000:])!r}')
            self.read(.01)
            self.sample()
        return (time.monotonic() - self.started) * 1000

    def quiet(self, duration=.02, timeout=5):
        deadline = time.monotonic() + timeout
        while self.read(duration):
            if time.monotonic() >= deadline:
                raise RuntimeError('terminal kept writing while idle')

    def input_latency(self, key):
        self.quiet()
        start = time.monotonic()
        os.write(self.master, key)
        if not self.read(5):
            raise RuntimeError(f'no terminal feedback for {key!r}')
        elapsed = (time.monotonic() - start) * 1000
        self.quiet()
        return elapsed

    def finish(self, key=b'q', expected=0):
        os.write(self.master, key)
        deadline = time.monotonic() + 5
        while self.proc.poll() is None:
            self.read(.01)
            if time.monotonic() > deadline:
                raise RuntimeError('terminal failed to quit')
        self.quiet()
        if self.proc.returncode != expected:
            raise RuntimeError(f'exit {self.proc.returncode}, expected {expected}')
        try:
            restored = termios.tcgetattr(self.slave) == self.original
        except termios.error:
            # macOS revokes the slave when the session leader exits and resets its termios,
            # so the restore is unobservable there; Linux keeps the check.
            if sys.platform != 'darwin':
                raise
            restored = True
        if not restored:
            raise RuntimeError('raw-mode/termios settings were not restored')
        if b'\x1b[?1049l' not in self.output:
            raise RuntimeError('alternate screen was not restored')

    def drain_until_exit(self, seconds):
        """Keep reading while waiting: a process exiting with unread terminal output waits in the kernel
        for it to drain, and no signal can end that wait (macOS)."""
        deadline = time.monotonic() + seconds
        while self.proc.poll() is None and time.monotonic() < deadline:
            self.read(0.05)
        return self.proc.poll() is not None

    def close(self):
        if self.proc.poll() is None:
            # The group may already be gone, or macOS may refuse the group kill (EPERM while a member is
            # exiting); the process itself is still ours to kill.
            for sig, wait in ((signal.SIGTERM, 2), (signal.SIGKILL, 5)):
                with suppress(ProcessLookupError, PermissionError):
                    os.killpg(self.proc.pid, sig)
                if self.drain_until_exit(wait):
                    break
            else:
                with suppress(ProcessLookupError):
                    self.proc.kill()
                if not self.drain_until_exit(30):
                    os.close(self.master)  # a closed master hangs up the terminal and ends the drain
                    self.master = None
                    self.proc.wait()
        if self.master is not None:
            os.close(self.master)
        os.close(self.slave)


def record(name, unit, samples):
    print('LENZ_PERF ' + json.dumps({'name': 'tui/pty/' + name, 'unit': unit, 'samples': samples}), flush=True)


def main():
    isolate_user_state()  # never measure with the user's own settings or saved positions
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--lines', type=int, default=100_000)
    parser.add_argument('--rounds', type=int, default=30)
    args = parser.parse_args()
    if args.lines < 1 or args.rounds < 1:
        parser.error('lines and rounds must be positive')
    app = str(args.app.resolve())
    with tempfile.TemporaryDirectory(prefix='lenz-tui-bench-') as directory:
        root = Path(directory)
        for name, word in [('left', 'OLD'), ('right', 'NEW')]:
            with (root/name).open('w') as output:
                for i in range(args.lines):
                    output.write(f'BENCH_{word}_{i} café 👩‍💻\n' if i % 80 == 0 else f'unchanged line {i}\n')
        probe = TerminalProbe([app, '--no-index', 'left', 'right'], root)
        try:
            paint = probe.expect(b'Lenz')
            ready = probe.expect(b'BENCH_NEW_0')
            probe.quiet(.25)
            baseline = probe.sample()
            before_bytes = len(probe.output)
            idle_start = time.monotonic()
            # Read and count, rather than leaving a busy app blocked on a full PTY.
            while time.monotonic() - idle_start < .5:
                probe.read(.05)
            after = probe.sample()
            idle_bytes = len(probe.output) - before_bytes
            if idle_bytes:
                raise RuntimeError(f'idle terminal emitted {idle_bytes} bytes')
            latency = [probe.input_latency(b'\t') for _ in range(args.rounds)]
            resize = []
            for width, height in [(40, 12), (200, 60), (120, 36)]:
                probe.quiet()
                start = time.monotonic()
                probe.resize(width, height)
                if not probe.read(5):
                    raise RuntimeError('resize did not repaint')
                resize.append((time.monotonic() - start) * 1000)
            probe.quiet(.1)
            settled = probe.sample()
            probe.finish()
            record('first-paint', 'ms', [paint])
            record('first-content', 'ms', [ready])
            record('input-first-byte', 'ms', latency)
            record('resize-first-byte', 'ms', resize)
            record('idle-output', 'bytes', [idle_bytes])
            if after['cpu_seconds'] is not None and baseline['cpu_seconds'] is not None:
                record('idle-cpu', 's', [max(0, after['cpu_seconds'] - baseline['cpu_seconds'])])
            if settled['rss_mib'] is not None:
                record('settled-rss', 'MiB', [settled['rss_mib']])
            if probe.peak is not None:
                record('peak-rss', 'MiB', [probe.peak])
        finally:
            probe.close()
        # Error-path cleanup and Ctrl-C while the search prompt owns input.
        for command, key, expected in [([app, '--no-index', 'absent', 'right'], b'q', 2),
                                        ([app, '--no-index', 'left', 'right'], b'/hello\x03', 0)]:
            probe = TerminalProbe(command, root)
            try:
                probe.expect(b'Lenz')
                if expected == 2:
                    probe.expect(b'absent')
                    probe.quiet(.1)
                else:
                    # Find needs the loaded document; typed early, "hello" would run as commands (`o` opens an editor)
                    probe.expect(b'BENCH_NEW_0')
                    probe.quiet(.1)
                probe.finish(key, expected)
            finally:
                probe.close()
    print('Owned PTY: normal/error/Ctrl-C cleanup passed; timings end at PTY bytes, not physical display.')


if __name__ == '__main__':
    main()

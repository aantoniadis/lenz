#!/usr/bin/env python3
"""Deterministic collector checks; only disposable owned PTYs and child processes."""
import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('code_tui_bench', Path(__file__).with_name('code-tui-bench.py'))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)

CHILD = r'''
import os, sys, termios, tty
saved = termios.tcgetattr(0)
tty.setraw(0)
try:
    os.write(1, b'\x1b[?1049hREADY')
    if sys.argv[1] == 'error': raise ValueError('fixture error')
    while os.read(0, 1) != b'q': pass
finally:
    termios.tcsetattr(0, termios.TCSANOW, saved)
    os.write(1, b'\x1b[?1049l')
'''


class CollectorTests(unittest.TestCase):
    def test_identity_skips_corpus_directories_named_like_rust_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            collector = root / 'scripts' / 'code-tui-bench.py'
            collector.parent.mkdir()
            collector.write_text('collector fixture')
            source = root / 'crates' / 'engine' / 'tests' / 'corpus' / 'fixture.rs'
            source.mkdir(parents=True)
            (source / 'input.rs').write_text('fn main() {}')
            with patch.object(bench, '__file__', str(collector)):
                result = bench.identity(Path(sys.executable))
            self.assertEqual(len(result['source_sha256']), 64)

    def probe(self, mode='ok'):
        p = bench.TerminalProbe([sys.executable, '-c', CHILD, mode], Path(tempfile.gettempdir()))
        self.addCleanup(p.close)
        p.expect('READY')
        return p

    def test_success_drains_and_checks_real_terminal_restoration(self):
        p = self.probe()
        p.finish(b'q')
        self.assertTrue(p.restored)
        self.assertIn(b'\x1b[?1049l', p.output)

    def test_error_exit_still_checks_terminal_restoration(self):
        p = self.probe('error')
        p.finish(b'', expected=1)
        self.assertTrue(p.restored)
        self.assertIn(b'fixture error', p.output)

    def test_marker_is_read_even_after_child_already_exited(self):
        p = bench.TerminalProbe([sys.executable, '-c', "print('EXITED-MARKER', flush=True)"], Path(tempfile.gettempdir()))
        self.addCleanup(p.close)
        p.proc.wait(timeout=2)
        p.expect('EXITED-MARKER')

    def test_timeout_is_bounded_and_close_reaps_child(self):
        p = self.probe()
        with self.assertRaisesRegex(RuntimeError, 'never-arrives'):
            p.expect('never-arrives', timeout=.05)
        p.close()
        self.assertIsNotNone(p.proc.returncode)
        self.assertIsNone(p.master)
        self.assertIsNone(p.slave)
        p.close()

    def test_close_drains_a_child_continuously_writing_terminal_output(self):
        child = "import os; os.write(1,b'READY'); exec(\"while True: os.write(1,b'x'*65536)\")"
        p = bench.TerminalProbe([sys.executable, '-c', child], Path(tempfile.gettempdir()))
        self.addCleanup(p.close)
        p.expect('READY')
        p.close()
        self.assertIsNotNone(p.proc.returncode)

    def test_missing_restoration_is_rejected(self):
        child = "import os,tty; tty.setraw(0); os.write(1,b'READY'); os.read(0,1)"
        p = bench.TerminalProbe([sys.executable, '-c', child], Path(tempfile.gettempdir()))
        self.addCleanup(p.close)
        p.expect('READY')
        with self.assertRaisesRegex(RuntimeError, 'restor'):
            p.finish(b'q')

    def test_linux_parser_uses_exact_field_names_and_last_command_parenthesis(self):
        status = 'VmRSSAnon: 999999 kB\nVmRSS: 2048 kB\nVmHWM: 4096 kB\n'
        # Process comm can itself contain spaces and parentheses; fields start after its last ')'.
        fields = ['S'] + ['0'] * 10 + ['150', '25'] + ['0'] * 10
        sample = bench.parse_linux(status, '123 (odd ) command) ' + ' '.join(fields), 100)
        self.assertEqual(sample, {'rss_mib': 2, 'peak_rss_mib': 4, 'cpu_seconds': 1.75})
        self.assertEqual(bench.parse_linux('VmRSSAnon: 99 kB', 'broken', 100),
                         {'rss_mib': None, 'peak_rss_mib': None, 'cpu_seconds': None})
        self.assertIsNone(bench.parse_linux('', '1 (x) S 0', 100)['cpu_seconds'])

    def test_ps_parser_supports_days_and_does_not_invent_absent_memory(self):
        self.assertEqual(bench.parse_ps('2048 01:02.50')['cpu_seconds'], 62.5)
        self.assertEqual(bench.parse_ps('2048 1-02:03:04')['cpu_seconds'], 93784)
        self.assertEqual(bench.parse_ps('2048 01:02')['rss_mib'], 2)
        self.assertEqual(bench.parse_ps(''), {'rss_mib': None, 'peak_rss_mib': None, 'cpu_seconds': None})

    def test_screen_reconstructs_split_escape_and_utf8_sequences(self):
        screen = bench.Screen(8, 3)
        for part in [b'abc\x1b[2;', b'2Hcaf\xc3', b'\xa9\x1b[31m!', b'\x1b[0m\x1b[1;1H\x1b[2K']:
            screen.feed(part)
        self.assertEqual(screen.text().splitlines()[1].rstrip(), ' café!')
        self.assertEqual(screen.text().splitlines()[0].strip(), '')


if __name__ == '__main__':
    unittest.main()

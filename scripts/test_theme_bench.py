import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('theme_bench', Path(__file__).with_name('theme-bench.py'))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)

class ThemeCollectorTests(unittest.TestCase):
    def test_current_cells_not_old_output_determine_transition(self):
        screen = bench.ColorScreen(2, 1)
        screen.feed(b'\x1b[48;2;43;43;43m  ')
        self.assertFalse(bench.matches_palette(screen, 0xeff1f5))
        screen.feed(b'\x1b[1;1H\x1b[48;2;239;241;245m  ')
        self.assertTrue(bench.matches_palette(screen, 0xeff1f5))
        screen.feed(b'\x1b[1;1H\x1b[48;2;43;43;43m  ')
        self.assertFalse(bench.matches_palette(screen, 0xeff1f5))

    def test_split_sgr_repeat_and_erase_preserve_current_colors(self):
        screen = bench.ColorScreen(4, 2)
        for part in [b'\x1b[48;2;239;', b'241;245mX\x1b[', b'3b']:
            screen.feed(part)
        self.assertEqual(screen.background[0], [0xeff1f5]*4)
        screen.feed(b'\x1b[0m\x1b[2J')
        self.assertFalse(bench.matches_palette(screen, 0xeff1f5))
        self.assertEqual(screen.text(), '    \n    ')

    def test_unchanged_byte_source_times_out(self):
        class Fake:
            def read(self, timeout): return b''
            def sample(self): return {}
        screen = bench.ColorScreen(2, 1)
        screen.feed(b'\x1b[48;2;43;43;43mXX')
        with self.assertRaises(TimeoutError):
            bench.wait_transition(Fake(), screen, 0xeff1f5, 'XX', timeout=.01)

"""Reject false readiness and unverifiable Markdown performance evidence."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('markdown_bench', Path(__file__).with_name('markdown-bench.py'))
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


class MarkdownBenchTests(unittest.TestCase):
    def test_gui_tail_requires_formatted_prose_not_source_tail(self):
        self.assertFalse(bench.gui_rendered_tail('Source Rendered MARKDOWN_TAIL_END'))
        self.assertFalse(bench.gui_rendered_tail('Paragraph 999 with **strong**, *emphasis*, `code`\nMARKDOWN_TAIL_END'))
        self.assertTrue(bench.gui_rendered_tail('Paragraph 999 with strong, emphasis, code\nMARKDOWN_TAIL_END'))

    def test_completed_render_requires_tail_and_ready(self):
        self.assertFalse(bench.completed_render('MARKDOWN_TAIL_END', ready=False))
        self.assertFalse(bench.completed_render('loading', ready=True))
        self.assertTrue(bench.completed_render('MARKDOWN_TAIL_END', ready=True))

    def test_source_or_preparing_screen_cannot_complete(self):
        self.assertFalse(bench.terminal_completed('MARKDOWN_TAIL_END\nSource'))
        self.assertFalse(bench.terminal_completed('MARKDOWN_TAIL_END\nPreparing rendered Markdown'))
        self.assertTrue(bench.terminal_completed('MARKDOWN_TAIL_END\nRendered Markdown'))
        self.assertFalse(bench.terminal_completed('MARKDOWN_TAIL_END\nRendered Markdown\nPreparing rendered Markdown'))
        self.assertFalse(bench.terminal_completed('MARKDOWN_TAIL_END\nRendered Markdown exceeds parser events — showing Source'))

    def test_reconstructed_screen_rejects_old_tail(self):
        class Probe:
            output = bytearray(b'MARKDOWN_TAIL_END\x1b[2J\x1b[HRendered Markdown')
        screen = bench.Screen()
        screen.update(Probe())
        self.assertFalse(bench.terminal_completed(screen.text()))

    def test_completion_accepts_sidebar_and_active_find_status(self):
        self.assertTrue(bench.terminal_completed('MARKDOWN_TAIL_END\n│ files │Rendered Markdown'))
        self.assertTrue(bench.terminal_completed('MARKDOWN_TAIL_END\nRendered Markdown · Find: Paragraph 1/100'))

    def test_details_hint_preserves_completed_status_and_rejects_source_fallback(self):
        self.assertTrue(bench.terminal_completed('MARKDOWN_TAIL_END\nF2 details · Rendered Markdown'))
        self.assertTrue(bench.terminal_completed('MARKDOWN_TAIL_END\n│ files │F2 details · Rendered Markdown · Find: Paragraph 1/100'))
        for status in ['Preparing rendered Markdown', 'Source',
                       'Rendered Markdown exceeds parser events — showing Source']:
            self.assertFalse(bench.terminal_completed('MARKDOWN_TAIL_END\nF2 details · ' + status))
        self.assertFalse(bench.terminal_completed('F2 details · Rendered Markdown'))

    def test_changed_fixture_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'input.md'
            path.write_text('original')
            manifest = {'root': directory, 'files': {'input.md': bench.file_identity(path)}}
            bench.verify_fixtures(manifest)
            path.write_text('changed')
            with self.assertRaisesRegex(ValueError, 'fixture'):
                bench.verify_fixtures(manifest)

    def test_incomplete_stress_rejected(self):
        for data in ({'duration_seconds': 59.9, 'cycles': 8, 'timeline': [0, 59.9]},
                     {'duration_seconds': 61, 'cycles': 0, 'timeline': [0, 61]},
                     {'duration_seconds': 61, 'cycles': 8, 'timeline': [0, 20]}):
            with self.assertRaisesRegex(ValueError, 'stress'):
                bench.validate_stress(data, 60)
        bench.validate_stress({'duration_seconds': 60.2, 'cycles': 8, 'timeline': [0, 60.2]}, 60)

    def test_missing_build_identity_rejected(self):
        for identity in ({}, {'binary': {'sha256': 'a'*64, 'bytes': 10}},
                         {'binary': {'sha256': 'a'*64, 'bytes': 10}, 'source': {'commit': 'b'*40}}):
            with self.assertRaisesRegex(ValueError, 'identity'):
                bench.validate_identity(identity)
        bench.validate_identity({'binary': {'sha256': 'a'*64, 'bytes': 10},
                                 'source': {'commit': 'b'*40, 'source': {'file.rs': 'c'*64}}})

    def test_fixture_generator_preserves_version_one_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = bench.generate_fixtures(Path(directory))
            self.assertEqual(manifest['workload_version'], 1)
            self.assertEqual(manifest['files']['small-left.md']['sha256'],
                             '3a8a21059454551697536ed2968d4d69bcb447d3c7b9c0aebe1cb8a6d110d34a')
            self.assertEqual(manifest['files']['medium-right.md']['sha256'],
                             'ab0454e4fb3d01b4f34b17883c98c7f034d81bd9d46844cd871a7951a06b660a')
            self.assertEqual(manifest['files']['large-right.md']['sha256'],
                             'bf436aa213f81c9a92a5f953337d269799f0a9a0700b7b3a2e5f61f834633e29')
            bench.verify_fixtures(manifest)


if __name__ == '__main__':
    unittest.main()

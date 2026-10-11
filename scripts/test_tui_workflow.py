"""Checks for the workflow probe's cell assertions, independent of a live terminal."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest

_spec = importlib.util.spec_from_file_location('tui_workflow', Path(__file__).with_name('tui-workflow-bench.py'))
_workflow = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_workflow)


class ScreenTests(unittest.TestCase):
    def test_repository_filter_uses_the_palette_instead_of_literal_log_search(self):
        calls = []
        workflow = _workflow.Workflow.__new__(_workflow.Workflow)
        workflow.key = lambda data, *texts, **kwargs: calls.append((data, texts, kwargs))
        workflow.history_filter('first', 'Workflow first commit', absent=('Workflow second commit',), metric='history-filter')
        self.assertEqual(calls[0][:2], (b'\x1b[21~', ('Commands:',)))
        self.assertEqual(calls[1][0], b'filter history')
        self.assertEqual(calls[2][:2], (b'\r', ('Log filter:',)))
        self.assertEqual(calls[3], (b'first\r', ('Workflow first commit',),
            {'absent': ('Workflow second commit',), 'metric': 'history-filter'}))
        calls.clear()
        workflow.menu('Find in Change Set', 'Find in Change Set:')
        self.assertEqual(calls[0][:2], (b'\x1b[21~', ('Commands:',)))

    def test_blame_expected_date_uses_author_utc_across_local_midnight(self):
        timestamp = _workflow.datetime.datetime.fromisoformat('2026-10-04T01:00:00+03:00').timestamp()
        self.assertEqual(_workflow.author_date(str(int(timestamp))), '2026-10-03')

    def test_fragmented_cursor_and_utf8_place_wide_cells(self):
        screen = _workflow.Screen(12, 4)
        probe = SimpleNamespace(output=bytearray())
        for fragment in [b'\x1b[2;', b'3H\x1b[31m', '安'.encode()[:1], '安'.encode()[1:], b'X']:
            probe.output.extend(fragment)
            screen.update(probe)
        self.assertEqual(''.join(screen.cells[1]), '  安X       ')
        self.assertEqual(screen.cells[1][3], '')
        self.assertEqual(screen.x, 5)

    def test_reconstructed_cells_discard_previous_popup(self):
        screen = _workflow.Screen(16, 4)
        probe = SimpleNamespace(output=bytearray(b'\x1b[2;2HCommands: old'))
        screen.update(probe)
        self.assertIn('Commands: old', screen.text())
        probe.output.extend(b'\x1b[2;2H\x1b[2Knew')
        screen.update(probe)
        self.assertNotIn('Commands:', screen.text())
        self.assertIn('new', screen.text())
        probe.output.extend(b'\x1b[2J\x1b[1;1HReady')
        screen.update(probe)
        self.assertNotIn('new', screen.text())
        self.assertTrue(screen.text().startswith('Ready'))

    def test_commit_ready_cannot_match_the_background_log_or_loading_modal(self):
        probe = SimpleNamespace(output=bytearray(b'\x1b[1;1HWorkflow Author\x1b[2;1HWorkflow second commit\x1b[3;1HCommit abc\x1b[4;1HLoading commit...'), proc=SimpleNamespace(poll=lambda: None), sample=lambda: {}, reads=0)
        def read(_timeout):
            probe.reads += 1
            probe.output.extend(b'\x1b[6;1HWorkflow Author <workflow@example.invalid>')
        probe.read = read
        workflow = _workflow.Workflow.__new__(_workflow.Workflow)
        workflow.probe, workflow.screen = probe, _workflow.Screen()
        workflow.wait(_workflow.COMMIT_DETAILS_MARKER, 'Commit', 'Workflow second commit')
        self.assertEqual(probe.reads, 1, 'must wait for modal details, not background log text')

    def test_split_osc_is_skipped_before_visible_text(self):
        screen = _workflow.Screen(16, 4)
        probe = SimpleNamespace(output=bytearray(b'\x1b]0;hidden'))
        screen.update(probe)
        self.assertNotIn('hidden', screen.text())
        probe.output.extend(b' title\x1b\\visible')
        screen.update(probe)
        self.assertTrue(screen.text().startswith('visible'))
        self.assertNotIn('hidden', screen.text())


if __name__ == '__main__':
    unittest.main()

class FixtureGitTests(unittest.TestCase):
    def test_setup_retries_only_a_transient_index_lock(self):
        from unittest.mock import patch
        from types import SimpleNamespace
        locked=SimpleNamespace(returncode=128,stdout=b'',stderr=b"Unable to create 'fixture/.git/index.lock': File exists.")
        conflict=SimpleNamespace(returncode=1,stdout=b'conflict',stderr=b'')
        with patch.object(_workflow.subprocess,'run',side_effect=[locked,conflict]) as run, patch.object(_workflow.time,'sleep'):
            self.assertEqual(_workflow.git(Path('/fixture'),'merge','side',expected=1),'conflict')
            self.assertEqual(run.call_count,2)
        failed=SimpleNamespace(returncode=128,stdout=b'',stderr=b'unknown revision')
        with patch.object(_workflow.subprocess,'run',return_value=failed) as run:
            with self.assertRaises(RuntimeError):_workflow.git(Path('/fixture'),'merge','side',expected=1)
            self.assertEqual(run.call_count,1)

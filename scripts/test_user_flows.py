"""Keep the repeated-suite runner from silently accepting missing coverage."""
import importlib.util
from pathlib import Path
import subprocess
import signal
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('user_flows', Path(__file__).with_name('check-user-flows.py'))
flows = importlib.util.module_from_spec(spec)
spec.loader.exec_module(flows)


class RunnerTests(unittest.TestCase):
    def test_native_repository_oracle_ignores_old_pane_and_sidebar_candidates(self):
        spec = importlib.util.spec_from_file_location('native_repository', Path(__file__).with_name('native-user-flows.py'))
        native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native)
        previous = ('files: other.rs  Lenz  1/2\n'
                    'WORKFLOW_1_FILE  ▶ file.rs\n'
                    'WORKFLOW_1_OTHER WORKFLOW_2_FILE\nstatus\nfooter')
        self.assertFalse(native.tui_repository_ready(previous, 'file.rs', 'WORKFLOW_1_FILE', ('WORKFLOW_2_FILE',)))
        self.assertFalse(native.tui_repository_ready(previous, 'other.rs', 'WORKFLOW_1_OTHER', ('WORKFLOW_2_OTHER',)))
        accepted = ('files: other.rs  Lenz  2/2\n'
                    'WORKFLOW_2_FILE  ▶ other.rs\n'
                    'WORKFLOW_2_OTHER WORKFLOW_1_OTHER\nstatus\nfooter')
        self.assertTrue(native.tui_repository_ready(accepted, 'other.rs', 'WORKFLOW_1_OTHER', ('WORKFLOW_2_OTHER',)))
        named=accepted.replace('Lenz', 'main (abc123)')
        self.assertTrue(native.tui_repository_ready(named, 'other.rs', 'WORKFLOW_1_OTHER', ('WORKFLOW_2_OTHER',)))
        self.assertTrue(native.tui_repository_ready('Find: commit · 1/1\nlog row\n'+named, 'other.rs', 'WORKFLOW_1_OTHER', ('WORKFLOW_2_OTHER',)))
        self.assertFalse(native.tui_repository_ready(named.replace('2/2', 'loading'), 'other.rs', 'WORKFLOW_1_OTHER'))

    def test_native_current_pane_rejects_previous_commit_and_wrong_file(self):
        spec = importlib.util.spec_from_file_location('native_pane', Path(__file__).with_name('native-user-flows.py'))
        native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native)
        self.assertTrue(native.visible_ready('other.rs\nWORKFLOW_1 needle', ('other.rs', 'WORKFLOW_1'), ('WORKFLOW_2',)))
        self.assertFalse(native.visible_ready('other.rs\nWORKFLOW_2 needle', ('other.rs', 'WORKFLOW_1'), ('WORKFLOW_2',)))
        self.assertFalse(native.visible_ready('file.rs\nWORKFLOW_1 needle', ('other.rs', 'WORKFLOW_1'), ('WORKFLOW_2',)))

    def test_native_fixture_has_real_history_binary_versions_and_valid_png(self):
        spec = importlib.util.spec_from_file_location('native_fixture', Path(__file__).with_name('native-user-flows.py'))
        native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native.content_fixture(root)
            self.assertEqual((root / 'versions' / 'theirs').read_bytes(), b'theirs\0\xff\r\n')
            self.assertEqual((root / 'merge' / 'ours').read_bytes(), b'common\nours\nend\n')
            self.assertEqual((root / 'left.png').read_bytes()[:8], b'\x89PNG\r\n\x1a\n')
            self.assertIn('Paragraph 0 with **strong**', (root / 'left.md').read_text())
            self.assertEqual(subprocess.check_output(['git', 'log', '--format=%s'], cwd=root / 'repository', text=True).splitlines(),
                             ['Workflow second commit', 'Workflow first commit'])
            for revision, filename, marker in [('HEAD^', 'file.rs', 'WORKFLOW_1_FILE'), ('HEAD^', 'other.rs', 'WORKFLOW_1_OTHER'),
                                               ('HEAD', 'file.rs', 'WORKFLOW_2_FILE'), ('HEAD', 'other.rs', 'WORKFLOW_2_OTHER')]:
                self.assertIn(marker, subprocess.check_output(['git', 'show', revision + ':' + filename], cwd=root / 'repository', text=True))

    def test_native_labels_reject_ambiguous_controls_and_match_whole_words(self):
        spec = importlib.util.spec_from_file_location('native_labels', Path(__file__).with_name('native-user-flows.py'))
        native = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(native)
        header = 'level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n'
        rows = header + '5\t1\t1\t1\t1\t1\t20\t12\t30\t15\t95\tSave\n' + '5\t1\t1\t1\t1\t2\t55\t12\t50\t15\t95\tchanges\n'
        self.assertEqual(native.label_bounds(rows, 'Save changes'), (20, 12, 85, 15))
        with self.assertRaises(AssertionError):
            native.label_bounds(rows, 'Sav')
        with self.assertRaises(AssertionError):
            native.label_bounds(rows + '5\t1\t2\t1\t1\t1\t20\t80\t30\t15\t95\tSave\n', 'Save')
        self.assertEqual(native.label_bounds(rows, 'Save', max_y=32), (20, 12, 30, 15))

    def test_native_save_wait_checks_exact_current_bytes(self):
        native_spec = importlib.util.spec_from_file_location('native_flows', Path(__file__).with_name('native-user-flows.py'))
        native = importlib.util.module_from_spec(native_spec)
        native_spec.loader.exec_module(native)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'result'
            path.write_bytes(b'external\r\n')
            native.wait_bytes(path, b'external\r\n')
            with patch.object(native.time, 'monotonic', side_effect=[0, 11]):
                with self.assertRaises(AssertionError):
                    native.wait_bytes(path, b'external\n')

    def test_failure_empty_suite_and_timeout_are_retained(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'check.log'
            for exit_code, contents, expected in [
                (0, 'test result: ok. 2 passed; 0 failed; 3 ignored;', 0),
                (1, 'test result: FAILED. 2 passed; 1 failed; 3 ignored;', 1),
                (0, 'test result: ok. 0 passed; 0 failed; 0 ignored;', 1),
            ]:
                def execute(*args, **kwargs):
                    Path(args[1]).write_text(contents)
                    return {'status':'passed' if exit_code==0 else 'failed','returncode':exit_code}
                with patch.object(flows, 'run_process', side_effect=execute) as execute_mock:
                    result = flows.run(['cargo', 'test'], log, 1)
                self.assertEqual(execute_mock.call_args.kwargs['cancel_signal'],signal.SIGINT)
                self.assertEqual(result['exit'], expected)
                self.assertEqual(log.read_text(), contents)
                self.assertEqual(result['passed'], int(contents.split()[3]))
            with patch.object(flows, 'run_process', return_value={'status':'timeout','returncode':-2}):
                self.assertEqual(flows.run(['cargo', 'test'], log, 1)['exit'], 124)
            self.assertIn('exceeded 1s', log.read_text())


if __name__ == '__main__':
    unittest.main()

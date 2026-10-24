"""Reader adapters own their budgets; Diff retains shared-dependency coverage."""
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('panic_sites', Path(__file__).with_name('panic-sites.py'))
sites = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sites)


class ScopeTests(unittest.TestCase):
    def test_terminal_budget_excludes_shared_dependency_diagnostics(self):
        diagnostics = '\n'.join(json.dumps({'message': {
            'code': {'code': 'clippy::unwrap_used'},
            'spans': [{'is_primary': True, 'file_name': path, 'line_start': 1, 'column_start': 1}],
        }}) for path in ['crates/content/src/a.rs', 'crates/reader-core/src/a.rs', 'crates/reader-tui/src/app.rs'])
        with patch.object(sites.subprocess, 'run', return_value=SimpleNamespace(stdout=diagnostics)):
            self.assertEqual(sites.current(reader_tui=True)['unwrap_used'], 1)

    def test_reader_budget_counts_both_owned_crates_and_excludes_shared_diagnostics(self):
        paths = ['crates/reader-core/src/session.rs', 'crates/reader-gui/src/lib.rs',
                 'crates/content/src/a.rs', 'crates/gpui/src/a.rs', 'crates/engine/src/a.rs',
                 'crates/reader-tui/src/app.rs']
        def diagnostics(selected):
            return '\n'.join(json.dumps({'message': {
                'code': {'code': 'clippy::unwrap_used'},
                'spans': [{'is_primary': True, 'file_name': path, 'line_start': 1, 'column_start': 1}],
            }}) for path in selected)
        with patch.object(sites.subprocess, 'run', return_value=SimpleNamespace(stdout=diagnostics(paths))):
            self.assertEqual(sites.current(reader=True)['unwrap_used'], 2)
            self.assertEqual(sites.current(reader_tui=True)['unwrap_used'], 1)
            self.assertEqual(sites.current(reader=True, reader_tui=True)['unwrap_used'], 1)
        for path in paths[:2]:
            with self.subTest(owned=path), patch.object(sites.subprocess, 'run', return_value=SimpleNamespace(stdout=diagnostics([path]))):
                self.assertEqual(sites.current(reader=True)['unwrap_used'], 1)
        with patch.object(sites.subprocess, 'run', return_value=SimpleNamespace(stdout=diagnostics(paths[2:5]))):
            self.assertEqual(sites.current(reader=True)['unwrap_used'], 0)
            self.assertEqual(sites.current()['unwrap_used'], 3, 'Diff retains Engine, Content and GPUI coverage')

    def test_package_scopes_keep_separate_budgets(self):
        for flags, packages in [
            ({}, ['--workspace', '--exclude', 'reader-core', '--exclude', 'reader-gui', '--exclude', 'reader-tui']),
            ({'reader': True}, ['-p', 'reader-core', '-p', 'reader-gui']),
            ({'reader_tui': True}, ['-p', 'reader-tui']),
            ({'reader': True, 'reader_tui': True}, ['-p', 'reader-tui']),
        ]:
            with self.subTest(flags=flags), patch.object(sites.subprocess, 'run', return_value=SimpleNamespace(stdout='')) as run:
                self.assertEqual(sites.current(**flags), dict.fromkeys(sites.LINTS, 0))
                command = run.call_args.args[0]
                self.assertEqual(command[3:3 + len(packages)], packages)
                self.assertEqual('--lib' in command, not flags.get('reader_tui', False))
                self.assertIn('--bins', command)


if __name__ == '__main__':
    unittest.main()

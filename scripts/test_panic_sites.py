"""The imported terminal adapter must not consume either existing panic budget."""
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

    def test_package_scopes_keep_separate_budgets(self):
        for flags, packages in [
            ({}, ['--workspace', '--exclude', 'reader-core', '--exclude', 'reader-gui', '--exclude', 'reader-tui']),
            ({'reader': True}, ['-p', 'reader-core', '-p', 'reader-gui']),
            ({'reader_tui': True}, ['-p', 'reader-tui']),
        ]:
            with self.subTest(flags=flags), patch.object(sites.subprocess, 'run', return_value=SimpleNamespace(stdout='')) as run:
                self.assertEqual(sites.current(**flags), dict.fromkeys(sites.LINTS, 0))
                command = run.call_args.args[0]
                self.assertEqual(command[3:3 + len(packages)], packages)
                self.assertEqual('--lib' in command, not flags.get('reader_tui', False))
                self.assertIn('--bins', command)


if __name__ == '__main__':
    unittest.main()

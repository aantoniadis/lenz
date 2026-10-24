"""Pure helper checks only: no X server, application, timed workload or user desktop."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location('foundation_native', Path(__file__).with_name('reader-foundation-native.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class FoundationNativeHelpers(unittest.TestCase):
    def test_close_request_uses_protocol_type_and_delete_payload(self):
        self.assertEqual(native.wm_delete_fields(7, 11, 13), {
            'window': 7, 'client_type': 11, 'data': (32, [13, 0, 0, 0, 0]),
        })

    def test_private_environment_preserves_home_but_removes_desktop_routes(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            original = {'HOME': '/original-home', 'DISPLAY': ':0', 'WAYLAND_DISPLAY': 'user',
                        'DBUS_SESSION_BUS_ADDRESS': 'user-bus', 'XDG_RUNTIME_DIR': '/user/runtime'}
            env = native.private_environment(original, root, ':123')
            self.assertEqual(env['HOME'], '/original-home')
            self.assertEqual(env['DISPLAY'], ':123')
            self.assertNotIn('WAYLAND_DISPLAY', env)
            self.assertNotIn('DBUS_SESSION_BUS_ADDRESS', env)
            self.assertEqual(env['XDG_RUNTIME_DIR'], str(root / 'runtime-dir'))
            self.assertEqual(original['DISPLAY'], ':0')

    def test_state_reader_ignores_only_unfinished_last_line(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'state.jsonl'
            path.write_text('{"label":"a","state":{"closed":false}}\n{"label":')
            self.assertEqual(native.states(path), {'a': {'closed': False}})
            path.write_bytes(b'{"label":"a","state":{"closed":false}}\n{"prefix":"\xc3')
            self.assertEqual(native.states(path), {'a': {'closed': False}})
            path.write_text('invalid\n')
            with self.assertRaises(ValueError):
                native.states(path)

    def test_exact_text_oracle_refuses_a_truncated_prefix(self):
        with self.assertRaisesRegex(RuntimeError, 'truncated'):
            native.exact_text({'source': {'prefix': 'partial', 'prefix_complete': False}})
        self.assertEqual(native.exact_text({'source': {'prefix': 'é', 'prefix_complete': True}}), 'é')

    def test_destroyed_windows_during_native_close_are_ignored_but_other_errors_propagate(self):
        class Gone(Exception):
            pass
        alive = SimpleNamespace(get_attributes=Mock(return_value=SimpleNamespace(map_state=2)), get_wm_name=Mock(return_value='owned'))
        gone_attributes = SimpleNamespace(get_attributes=Mock(side_effect=Gone))
        gone_name = SimpleNamespace(get_attributes=Mock(return_value=SimpleNamespace(map_state=2)), get_wm_name=Mock(side_effect=Gone))
        hidden = SimpleNamespace(get_attributes=Mock(return_value=SimpleNamespace(map_state=0)), get_wm_name=Mock())
        root = SimpleNamespace(query_tree=Mock(return_value=SimpleNamespace(children=[gone_attributes, alive, gone_name, hidden])))
        self.assertEqual(native.visible_windows(root, 2, Gone), {'owned': alive})
        hidden.get_wm_name.assert_not_called()
        alive.get_attributes.side_effect = ValueError('unexpected X error')
        with self.assertRaisesRegex(ValueError, 'unexpected X error'):
            native.visible_windows(root, 2, Gone)

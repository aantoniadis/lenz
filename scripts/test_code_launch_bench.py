"""Launch-marker compatibility checks using owned child pipes, without opening windows."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest

spec = importlib.util.spec_from_file_location('code_launch', Path(__file__).with_name('code-launch-bench.py'))
launch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(launch)


class LaunchMarkerTests(unittest.TestCase):
    def child(self, stderr):
        child = subprocess.Popen([sys.executable, '-c', 'import sys; sys.stderr.write(sys.argv[1]); sys.stderr.flush()', stderr],
                                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.addCleanup(child.stderr.close)
        self.addCleanup(child.wait)
        return child

    def test_new_and_retained_legacy_executables_keep_the_same_marker_endpoint(self):
        for name in ['lenz-code', 'lenz-code']:
            with self.subTest(name=name):
                child = self.child(f'unrelated output\n{name}: tabs ready 12.5 ms\n')
                elapsed, reported, seen = launch.wait_for(child, 'tabs ready', limit=5)
                self.assertGreaterEqual(elapsed, 0)
                self.assertEqual(reported, 12.5)
                self.assertIn(f'{name}: tabs ready', seen)

    def test_unrelated_output_is_not_readiness(self):
        child = self.child('other-app: readable 3.0 ms\nlenz-code: not ready\n')
        with self.assertRaisesRegex(RuntimeError, 'no "readable" marker'):
            launch.wait_for(child, 'readable', limit=5)


if __name__ == '__main__':
    unittest.main()

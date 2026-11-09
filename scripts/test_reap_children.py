"""Real process-lifecycle checks; each supervisor owns only its command's children."""
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest

SUPERVISOR = Path(__file__).with_name('reap-children.py')


@unittest.skipUnless(sys.platform == 'linux', 'Linux subreaper and /proc checks')
class ReapChildren(unittest.TestCase):
    def run_case(self, source, *args):
        return subprocess.run([sys.executable, str(SUPERVISOR), '--grace-seconds', '.1', '--',
                               sys.executable, '-c', source, *map(str, args)],
                              capture_output=True, text=True, timeout=8)

    def summary(self, result):
        rows = [json.loads(line.removeprefix('VALIDATION_CHILDREN '))
                for line in result.stderr.splitlines() if line.startswith('VALIDATION_CHILDREN ')]
        self.assertEqual(len(rows), 1, result.stderr)
        return rows[0]

    def assert_gone(self, pid):
        self.assertFalse(Path(f'/proc/{pid}').exists(), f'owned child {pid} survived or was not reaped')

    def test_success_and_failure_preserve_exit_status(self):
        for code in [0, 7]:
            with self.subTest(code=code):
                result = self.run_case(f'print("output"); raise SystemExit({code})')
                self.assertEqual(result.returncode, code, result.stderr)
                self.assertEqual(result.stdout.strip(), 'output')
                self.assertEqual(self.summary(result)['command_returncode'], code)

    def test_failed_parent_and_detached_grandchild_are_reaped(self):
        source = ('import os,time\n'
                  'first=os.fork()\n'
                  'if first==0:\n'
                  ' os.setsid()\n'
                  ' second=os.fork()\n'
                  ' if second==0:\n'
                  '  print(os.getpid(),flush=True); time.sleep(.02); os._exit(0)\n'
                  ' os._exit(0)\n'
                  'os._exit(7)\n')
        result = self.run_case(source)
        self.assertEqual(result.returncode, 7, result.stderr)
        report = self.summary(result)
        self.assertEqual(report['reaped_orphans'], 2)
        self.assertFalse(report['forced_cleanup'])
        self.assert_gone(int(result.stdout.strip()))

    def test_orphans_are_collected_while_the_command_is_still_running(self):
        source = ('import os,time\nfrom pathlib import Path\n'
                  'for _ in range(20):\n'
                  ' read,write=os.pipe(); parent=os.fork()\n'
                  ' if parent==0:\n'
                  '  os.close(read); child=os.fork()\n'
                  '  if child==0: os._exit(0)\n'
                  '  os.write(write,str(child).encode()); os._exit(0)\n'
                  ' os.close(write); child=int(os.read(read,100)); os.close(read)\n'
                  ' os.waitpid(parent,0); deadline=time.monotonic()+2\n'
                  ' while Path(f"/proc/{child}").exists() and time.monotonic()<deadline: time.sleep(.01)\n'
                  ' assert not Path(f"/proc/{child}").exists(), "orphan accumulated during execution"\n')
        result = self.run_case(source)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.summary(result)['reaped_orphans'], 20)
        self.assertFalse(self.summary(result)['forced_cleanup'])

    def test_lingering_child_is_stopped_and_turns_success_into_failure(self):
        source = ('import os,signal,time\n'
                  'child=os.fork()\n'
                  'if child==0:\n'
                  ' os.setsid(); signal.signal(signal.SIGTERM,signal.SIG_IGN)\n'
                  ' print(os.getpid(),flush=True); time.sleep(60); os._exit(0)\n'
                  'os._exit(0)\n')
        result = self.run_case(source)
        self.assertEqual(result.returncode, 1, result.stderr)
        report = self.summary(result)
        self.assertEqual(report['command_returncode'], 0)
        self.assertTrue(report['forced_cleanup'])
        self.assertEqual(report['reaped_orphans'], 1)
        self.assert_gone(int(result.stdout.strip()))

    def test_interrupt_is_forwarded_and_cleanup_completes(self):
        with tempfile.TemporaryDirectory() as tmp:
            ready, stopped = Path(tmp)/'ready', Path(tmp)/'stopped'
            source = ('import signal,sys,time\nfrom pathlib import Path\n'
                      'def stop(number,frame):\n'
                      ' Path(sys.argv[2]).write_text("handled"); raise SystemExit(0)\n'
                      'signal.signal(signal.SIGTERM,stop)\n'
                      'Path(sys.argv[1]).write_text("ready")\n'
                      'while True:time.sleep(.01)\n')
            process = subprocess.Popen([sys.executable, str(SUPERVISOR), '--', sys.executable,
                                        '-c', source, str(ready), str(stopped)],
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                deadline = time.monotonic() + 4
                while not ready.exists() and process.poll() is None and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertTrue(ready.exists(), 'command did not become ready')
                process.send_signal(signal.SIGTERM)
                stdout, stderr = process.communicate(timeout=5)
                self.assertEqual(process.returncode, 128 + signal.SIGTERM, stderr)
                self.assertEqual(stopped.read_text(), 'handled')
                self.assertFalse(self.summary(subprocess.CompletedProcess([], process.returncode, stdout, stderr))['forced_cleanup'])
            finally:
                if process.poll() is None:
                    process.terminate()
                process.communicate(timeout=5)

    def test_cleanup_does_not_signal_an_unrelated_process(self):
        unrelated = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        try:
            result = self.run_case('import os,time\nif os.fork()==0: time.sleep(60)\nelse: os._exit(0)')
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertTrue(self.summary(result)['forced_cleanup'])
            self.assertIsNone(unrelated.poll(), 'supervisor signalled a process it did not own')
        finally:
            unrelated.terminate()
            unrelated.wait(timeout=5)

    def test_missing_command_fails_without_a_success_report(self):
        result = subprocess.run([sys.executable, str(SUPERVISOR), '--', '/nonexistent/lenz-validation-command'],
                                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('VALIDATION_CHILDREN ', result.stderr)

    def test_local_ci_enters_supervisor_once_and_preserves_a_failed_step(self):
        with tempfile.TemporaryDirectory() as tmp:
            tools = Path(tmp)
            cargo = tools/'cargo'
            cargo.write_text('#!/bin/sh\necho supervised-cargo\nexit 23\n')
            cargo.chmod(0o755)
            env = dict(os.environ, PATH=str(tools)+os.pathsep+os.environ.get('PATH', ''))
            env.pop('LENZ_VALIDATION_SUPERVISOR_PID', None)
            result = subprocess.run(['bash', str(SUPERVISOR.with_name('ci-local.sh').resolve())],
                                    env=env, capture_output=True, text=True, timeout=8)
            self.assertEqual(result.returncode, 23, result.stderr)
            self.assertEqual(result.stdout.count('supervised-cargo'), 1)
            self.assertEqual(self.summary(result)['command_returncode'], 23)


if __name__ == '__main__':
    unittest.main()

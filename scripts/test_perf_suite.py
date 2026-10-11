"""Behavior checks for trustworthy benchmark collection and comparison (stdlib only)."""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest


class PerformanceTools(unittest.TestCase):
    def support(self):
        path = Path(__file__).with_name('perf_support.py')
        self.assertTrue(path.exists(), 'portable benchmark support is missing')
        spec = importlib.util.spec_from_file_location('perf_support', path)
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_benchmarks_never_read_the_users_settings_or_places(self):
        support = self.support()
        saved = {k: os.environ.get(k) for k in ('XDG_CONFIG_HOME', 'XDG_STATE_HOME')}
        try:
            os.environ['XDG_CONFIG_HOME'] = os.environ['XDG_STATE_HOME'] = '/home/someone'
            scratch = support.isolate_user_state()
            for key in ('XDG_CONFIG_HOME', 'XDG_STATE_HOME'):
                self.assertTrue(os.environ[key].startswith(scratch), key)
            self.assertEqual(os.listdir(os.environ['XDG_STATE_HOME']), [])
            # no sample may restore a place an earlier sample saved
            settings = Path(os.environ['XDG_CONFIG_HOME'], 'lenz', 'settings.toml').read_text()
            self.assertIn('restore_place = false', settings)
        finally:
            for k, v in saved.items():
                if v is None: os.environ.pop(k, None)
                else: os.environ[k] = v

    def test_about_oracle_rejects_command_submenu_and_requires_dialog_content(self):
        s = self.support()
        self.assertEqual(s.GUI_ABOUT_ORACLE_VERSION, 3)
        menu = 'Help & appearance\nKeyboard shortcuts\nChange theme\nAbout Lenz\nReport last error\nBack to commands'
        self.assertFalse(s.gui_about_visible(menu))
        dialog = 'About Lenz\nVersion: 1.0\nPlatform: linux\nArchitecture: x86_64\nLicence: MIT; engine Apache-2.0'
        self.assertTrue(s.gui_about_visible(dialog))
        self.assertFalse(s.gui_about_visible(dialog.replace('Licence: MIT; engine Apache-2.0', 'Loading')))

    def test_metrics_reject_bad_samples_instead_of_reporting_success(self):
        s = self.support()
        good = 'test noise\nLENZ_PERF {"name":"find","unit":"ms","samples":[1,3,2]}\n'
        self.assertEqual(s.metrics(good)[0]['samples'], [1,3,2])
        for invalid in ['[]', '[NaN]', '[-1]', '["fast"]']:
            with self.assertRaises(ValueError):
                s.metrics('LENZ_PERF {"name":"find","unit":"ms","samples":'+invalid+'}')
        with self.assertRaises(ValueError):
            s.metrics('LENZ_PERF broken')

    def test_comparison_does_not_mix_units_or_ignore_removed_cases(self):
        s = self.support()
        before = {'a': {'unit':'ms','samples':[10,10,90]}, 'gone': {'unit':'ms','samples':[1]}}
        after = {'a': {'unit':'ms','samples':[12,12,100]}, 'new': {'unit':'MiB','samples':[20]}}
        result = s.compare(before, after, 15)
        self.assertEqual(result['regressions'], ['a'])
        self.assertEqual(result['missing'], ['gone'])
        self.assertEqual(result['added'], ['new'])
        self.assertAlmostEqual(result['changes']['a'], 20)
        with self.assertRaises(ValueError):
            s.compare(before, {'a': {'unit':'bytes','samples':[12]}}, 15)

    def test_comparison_rejects_corrupt_baseline_and_zero_is_not_a_free_pass(self):
        s = self.support()
        with self.assertRaises(ValueError):
            s.compare({'a':{'unit':'ms','samples':[float('nan')]}}, {'a':{'unit':'ms','samples':[1]}}, 10)
        result = s.compare({'a':{'unit':'ms','samples':[0]}}, {'a':{'unit':'ms','samples':[1]}}, 10)
        self.assertEqual(result['uncomparable'], ['a'])

    def test_failed_command_and_timeout_preserve_logs(self):
        s = self.support()
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp)/'case.log'
            result = s.run([sys.executable,'-c','print("evidence",flush=True); raise SystemExit(7)'], log, os.environ.copy(), 5)
            self.assertEqual(result['status'], 'failed')
            self.assertEqual(result['returncode'], 7)
            self.assertIn('evidence', log.read_text())
            result = s.run([sys.executable,'-c','import time; print("waiting",flush=True); time.sleep(20)'], log, os.environ.copy(), .2)
            self.assertEqual(result['status'], 'timeout')
            self.assertIn('waiting', log.read_text())
            self.assertLess(result['wall_seconds'], 4)

    @unittest.skipUnless(Path('/proc/self/task').exists(), 'Linux detached native children')
    def test_cooperative_timeout_reaps_detached_child_even_when_parent_cleans_up(self):
        import signal
        s = self.support()
        with tempfile.TemporaryDirectory() as tmp:
            child = 'import signal,time; signal.signal(signal.SIGINT,signal.SIG_IGN); time.sleep(20)'
            closed=Path(tmp)/'parent-cleaned'
            parent = ('import subprocess,sys,time; from pathlib import Path\n'
                      'p=subprocess.Popen([sys.executable,"-c",' + repr(child) + '],start_new_session=True)\n'
                      'print(p.pid,flush=True)\ntry:time.sleep(20)\nfinally:Path(' + repr(str(closed)) + ').write_text("closed")')
            log=Path(tmp)/'log'
            result=s.run([sys.executable,'-c',parent],log,os.environ.copy(),.2,cancel_signal=signal.SIGINT)
            self.assertEqual(result['status'],'timeout')
            pid=int(log.read_text().splitlines()[0])
            self.addCleanup(lambda: self.kill_running_child(pid))
            self.assertTrue(closed.exists(),'cooperative cancellation bypassed parent finally')
            self.assertFalse(self.child_running(pid), 'detached workload survived timeout')

    @staticmethod
    def child_running(pid):
        try:return Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()[0]!='Z'
        except FileNotFoundError:return False

    @classmethod
    def kill_running_child(cls,pid):
        import signal
        if cls.child_running(pid):os.kill(pid,signal.SIGKILL)

    def test_sustained_growth_ignores_warmup_but_detects_a_rising_tail(self):
        s = self.support()
        self.assertEqual(s.tail_growth([100,200,300,400,401,399,400,400]), 0)
        self.assertEqual(s.tail_growth([100,100,100,100,100,200,300,400]), 200)
        self.assertIsNone(s.tail_growth([100,200,300]))

    def test_bsd_ps_sample_converts_units_and_refuses_missing_process(self):
        s = self.support()
        self.assertEqual(s.parse_ps_stats('  2048  01:02.50\n'),
                         {'rss_mib':2.0,'peak_rss_mib':None,'cpu_seconds':62.5})
        with self.assertRaises(ProcessLookupError):
            s.parse_ps_stats('')

    def test_timeout_kills_owned_descendants(self):
        s = self.support()
        with tempfile.TemporaryDirectory() as tmp:
            marker = Path(tmp)/'leaked-child'
            child = 'import time; from pathlib import Path; time.sleep(.7); Path('+repr(str(marker))+').write_text("leaked")'
            parent = 'import subprocess,sys,time; subprocess.Popen([sys.executable,"-c",'+repr(child)+']); time.sleep(20)'
            result = s.run([sys.executable,'-c',parent],Path(tmp)/'log',os.environ.copy(),.2)
            import time
            time.sleep(.8)
            self.assertEqual(result['status'],'timeout')
            self.assertFalse(marker.exists(), 'a timed-out case left a running child')

    def test_suite_refuses_nonempty_output_before_building(self):
        import subprocess
        with tempfile.TemporaryDirectory() as tmp:
            marker = Path(tmp)/'keep.txt'
            marker.write_text('keep this')
            completed = subprocess.run([sys.executable,'-B',str(Path(__file__).with_name('perf-suite.py')),
                                        '--profile','quick','--output',tmp],capture_output=True,text=True)
            self.assertEqual(completed.returncode,2)
            self.assertEqual(marker.read_text(),'keep this')
            self.assertEqual(list(Path(tmp).iterdir()),[marker])

    def test_suite_refuses_old_merge_workload_before_building(self):
        import subprocess
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp)/'old.json'
            baseline.write_text(json.dumps({'schema':1,'workload_version':1}))
            output = Path(tmp)/'new-run'
            commands = Path(tmp)/'bin'
            commands.mkdir()
            cargo = commands/'cargo'
            cargo.write_text('#!/bin/sh\nexit 91\n')
            cargo.chmod(0o755)
            completed = subprocess.run([sys.executable,'-B',str(Path(__file__).with_name('perf-suite.py')),
                '--profile','quick','--compare',str(baseline),'--output',str(output)],capture_output=True,text=True,
                env=dict(os.environ,PATH=str(commands)+os.pathsep+os.environ['PATH']))
            self.assertEqual(completed.returncode,2)
            self.assertIn('workload version differs',completed.stderr)
            self.assertFalse(output.exists(),'an incompatible baseline must fail before builds or collection')

    def test_current_process_memory_is_measured_not_zero_filled(self):
        s = self.support()
        sample = s.process_stats(os.getpid())
        self.assertGreater(sample['rss_mib'], 0)
        self.assertGreaterEqual(sample['cpu_seconds'], 0)
        self.assertEqual(s.ps_cpu_seconds('02-01:02:03.50'), 176523.5)
        self.assertEqual(s.ps_cpu_seconds('01:02.50'), 62.5)

    @unittest.skipUnless(sys.platform == 'darwin', 'macOS memory accounting')
    def test_macos_sample_is_physical_footprint_not_stale_resident_pages(self):
        # libmalloc marks freed small blocks reusable; ps/task resident size keeps counting them,
        # phys_footprint (what Activity Monitor charges) does not. Linux VmRSS drops when glibc trims.
        import ctypes, subprocess
        s = self.support()
        libc = ctypes.CDLL(None)
        libc.malloc.restype = ctypes.c_void_p
        blocks = [libc.malloc(96) for _ in range(500_000)]
        for block in blocks:
            ctypes.memset(block, 1, 96)
        for block in blocks:
            libc.free(ctypes.c_void_p(block))
        sample = s.process_stats(os.getpid())
        resident = int(subprocess.check_output(['ps', '-o', 'rss=', '-p', str(os.getpid())])) / 1024
        self.assertLess(sample['rss_mib'], resident - 20, sample)
        self.assertGreaterEqual(sample['peak_rss_mib'], sample['rss_mib'], sample)
        self.assertAlmostEqual(sample['resident_mib'], resident, delta=4)
        self.assertGreaterEqual(sample['cpu_seconds'], 0)

    def test_terminal_probe_close_survives_a_refused_process_group_kill(self):
        # macOS refused SIGKILL for a group once in ten launches (EPERM); the launch must still be torn down.
        import signal, subprocess
        spec = importlib.util.spec_from_file_location('tui_bench', Path(__file__).with_name('tui-bench.py'))
        bench = importlib.util.module_from_spec(spec); spec.loader.exec_module(bench)
        probe = bench.TerminalProbe(['sh', '-c', 'trap "" TERM; while :; do sleep 1; done'], None)
        real = os.killpg
        def refuse(pgid, sig):
            if sig == signal.SIGKILL:
                raise PermissionError(1, 'Operation not permitted')
            real(pgid, sig)
        try:
            os.killpg = refuse
            probe.close()
        finally:
            os.killpg = real
            try:
                os.kill(probe.proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.assertIsNotNone(probe.proc.poll(), 'the launched process must be gone after close')

    def test_terminal_probe_close_drains_output_a_dying_child_still_owes(self):
        # A macOS process exiting with unread terminal output waits in the kernel for it to drain; a probe
        # that stopped reading then waited forever on an unkillable child (2026-10-11, 30 minutes).
        import time
        spec = importlib.util.spec_from_file_location('tui_bench', Path(__file__).with_name('tui-bench.py'))
        bench = importlib.util.module_from_spec(spec); spec.loader.exec_module(bench)
        probe = bench.TerminalProbe(['sh', '-c', 'head -c 2000000 /dev/zero | tr "\\0" x; sleep 30'], None)
        time.sleep(0.5)  # the child fills the terminal's buffer and blocks
        started = time.monotonic()
        probe.close()
        self.assertIsNotNone(probe.proc.poll(), 'the child must be reaped')
        self.assertLess(time.monotonic() - started, 10, 'close must not wait on an undrained terminal')

    def test_four_file_merge_probe_is_isolated_from_git_and_original_output(self):
        import subprocess
        s = self.support()
        with tempfile.TemporaryDirectory() as tmp:
            repo = Path(tmp)/'repo'
            repo.mkdir()
            env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1')
            subprocess.run(['git','init','-q',str(repo)],env=env,check=True)
            paths = [repo/f'{side}.rs' for side in ('base','ours','theirs','merged')]
            for path in paths:
                path.write_text(path.stem+'\n')
            scratch = Path(tmp)/'scratch'
            scratch.mkdir()
            args, cwd = s.prepare_gui_launch(['merge',*[str(p) for p in paths]],scratch)
            self.assertEqual(Path(cwd).resolve(),scratch.resolve())  # macOS resolves /var through /private
            self.assertEqual(args[0],'merge')
            self.assertEqual([Path(p).read_bytes() for p in args[1:]], [p.read_bytes() for p in paths])
            Path(args[-1]).write_text('benchmark saved result\n')
            self.assertEqual(paths[-1].read_text(),'merged\n')
            with self.assertRaises(ValueError):
                s.prepare_gui_launch(['merge',*[str(p) for p in paths]],repo/'scratch')
            ordinary = ['log']
            self.assertEqual(s.prepare_gui_launch(ordinary,scratch),(ordinary,None))


if __name__ == '__main__':
    unittest.main()

class CargoArtifactSelection(unittest.TestCase):
    def test_examples_and_cli_cannot_replace_library_test_harness(self):
        spec=importlib.util.spec_from_file_location('perf_support',Path(__file__).with_name('perf_support.py'))
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        def artifact(kind,name,path,test=True):return json.dumps({'reason':'compiler-artifact','target':{'kind':[kind],'name':name},'profile':{'test':test},'executable':path})
        lines='\n'.join([artifact('lib','lenz','/tmp/tests'),artifact('bin','lenz','/tmp/cli'),artifact('example','crash_probe','/tmp/crash',False)])
        self.assertEqual(module.cargo_executable(lines,'lib','lenz',True),'/tmp/tests')
        with self.assertRaises(ValueError):module.cargo_executable(lines,'bench','engine',False)
        with self.assertRaises(ValueError):module.cargo_executable(lines+'\n'+artifact('lib','lenz','/tmp/other'),'lib','lenz',True)

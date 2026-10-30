"""Collection helpers shared by the portable suite and startup probe (Python stdlib)."""
from contextlib import suppress
import ctypes
import json
import math
import os
from pathlib import Path
import platform
import re
import selectors
import signal
import shutil
import statistics
import subprocess
import time

GUI_ABOUT_ORACLE_VERSION = 3


def read_display_number(fd, timeout=10):
    """Read the complete Xvfb readiness line before the caller closes its pipe."""
    deadline = time.monotonic() + timeout
    data = bytearray()
    with selectors.DefaultSelector() as events:
        events.register(fd, selectors.EVENT_READ)
        while b'\n' not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not events.select(remaining):
                raise TimeoutError('Xvfb did not publish a complete display number')
            chunk = os.read(fd, 32 - len(data))
            if not chunk:
                raise RuntimeError('Xvfb returned an incomplete display number')
            data.extend(chunk)
            if len(data) >= 32:
                raise RuntimeError('Xvfb returned an oversized display number')
    number = data.removesuffix(b'\n')
    if not number.isdigit():
        raise RuntimeError('Xvfb returned an invalid display number')
    return number.decode('ascii')


def gui_about_visible(text):
    """Require dialog-only content, never merely its command-menu label."""
    text = ' '.join(text.casefold().split())
    return all(marker in text for marker in (
        'about lenz', 'version', 'platform', 'architecture', 'licence',
    ))


def isolate_user_state():
    """Point XDG_CONFIG_HOME and XDG_STATE_HOME at a scratch directory for this run and its children, so the
    user's settings.toml and saved positions never shape a measurement, and switch place restore off so samples stay
    independent. Returns that directory."""
    import tempfile
    scratch = tempfile.mkdtemp(prefix='lenz-bench-xdg-')
    for name in ('config', 'state'):
        os.makedirs(os.path.join(scratch, name))
    os.environ['XDG_CONFIG_HOME'] = os.path.join(scratch, 'config')
    os.environ['XDG_STATE_HOME'] = os.path.join(scratch, 'state')
    # every sample measures a fresh launch: none restores the place an earlier sample saved
    os.makedirs(os.path.join(scratch, 'config', 'lenz'))
    with open(os.path.join(scratch, 'config', 'lenz', 'settings.toml'), 'w') as f:
        f.write('[session]\nrestore_place = false\n')
    return scratch


def prepare_gui_launch(args, scratch):
    """Isolate four-file merge probes from Git resume and original output files."""
    if len(args) != 5 or args[0] != 'merge':
        return args, None
    scratch = Path(scratch).resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1')
    check = subprocess.run(['git','-C',str(scratch),'rev-parse','--show-toplevel'],
                           env=env, capture_output=True)
    if check.returncode == 0:
        raise ValueError('merge benchmark scratch directory must be outside Git; choose another TMPDIR')
    copied = ['merge']
    for i, value in enumerate(args[1:]):
        source = Path(value).resolve()
        target = scratch/f'merge-{i}{source.suffix}'
        if source.exists():
            shutil.copyfile(source, target)
        elif i != 3:
            raise FileNotFoundError(source)
        copied.append(str(target))
    return copied, scratch


def ps_cpu_seconds(value):
    days, clock = value.split('-', 1) if '-' in value else ('0', value)
    parts = [float(p) for p in clock.split(':')]
    total = 0.0
    for part in parts:
        total = total * 60 + part
    return int(days) * 86400 + total


def process_stats(pid):
    if platform.system() == 'Linux':
        status = Path(f'/proc/{pid}/status').read_text()
        stat = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
        def mib(field):
            found = re.search(rf'{field}:\s+(\d+) kB', status)
            return int(found[1]) / 1024 if found else None
        return {'rss_mib': mib('VmRSS'), 'peak_rss_mib': mib('VmHWM'),
                'cpu_seconds': (int(stat[11]) + int(stat[12])) / os.sysconf('SC_CLK_TCK')}
    # BSD ps reports RSS in KiB and accumulated CPU as [[dd-]hh:]mm:ss.xx.
    stats = parse_ps_stats(subprocess.check_output(['ps', '-o', 'rss=', '-o', 'time=', '-p', str(pid)], text=True))
    if platform.system() == 'Darwin':
        stats.update(macos_footprint(pid))
    return stats


class _RusageInfoV4(ctypes.Structure):
    _fields_ = [('ri_uuid', ctypes.c_uint8 * 16)] + [(name, ctypes.c_uint64) for name in (
        'ri_user_time', 'ri_system_time', 'ri_pkg_idle_wkups', 'ri_interrupt_wkups', 'ri_pageins', 'ri_wired_size',
        'ri_resident_size', 'ri_phys_footprint', 'ri_proc_start_abstime', 'ri_proc_exit_abstime', 'ri_child_user_time',
        'ri_child_system_time', 'ri_child_pkg_idle_wkups', 'ri_child_interrupt_wkups', 'ri_child_pageins',
        'ri_child_elapsed_abstime', 'ri_diskio_bytesread', 'ri_diskio_byteswritten', 'ri_cpu_time_qos_default',
        'ri_cpu_time_qos_maintenance', 'ri_cpu_time_qos_background', 'ri_cpu_time_qos_utility', 'ri_cpu_time_qos_legacy',
        'ri_cpu_time_qos_user_initiated', 'ri_cpu_time_qos_user_interactive', 'ri_billed_system_time',
        'ri_serviced_system_time', 'ri_logical_writes', 'ri_lifetime_max_phys_footprint', 'ri_instructions', 'ri_cycles',
        'ri_billed_energy', 'ri_serviced_energy', 'ri_interval_max_phys_footprint', 'ri_runnable_time')]


def macos_footprint(pid):
    """Memory the kernel charges to the process (phys_footprint, Activity Monitor's "Memory") and its lifetime peak.

    Resident size (ps RSS) also counts freed pages libmalloc marked reusable that the kernel has not reclaimed;
    they never leave that count, while Linux VmRSS drops when glibc returns such pages. `rss_mib` is therefore
    the footprint on both platforms; `resident_mib` keeps the raw macOS resident size.
    """
    libproc = ctypes.CDLL('/usr/lib/libproc.dylib', use_errno=True)
    libproc.proc_pid_rusage.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
    info = _RusageInfoV4()
    if libproc.proc_pid_rusage(pid, 4, ctypes.byref(info)) != 0:  # RUSAGE_INFO_V4
        raise ProcessLookupError(f'proc_pid_rusage failed for {pid}: errno {ctypes.get_errno()}')
    mib = 1024 * 1024
    return {'rss_mib': info.ri_phys_footprint / mib, 'peak_rss_mib': info.ri_lifetime_max_phys_footprint / mib,
            'resident_mib': info.ri_resident_size / mib}


def parse_ps_stats(output):
    fields = output.split()
    if len(fields) != 2:
        raise ProcessLookupError('process is absent from ps output')
    return {'rss_mib': int(fields[0]) / 1024, 'peak_rss_mib': None, 'cpu_seconds': ps_cpu_seconds(fields[1])}


def validate_metric(item):
    samples = item.get('samples')
    if (not isinstance(item.get('name'), str) or not isinstance(item.get('unit'), str)
            or not isinstance(samples, list) or not samples
            or any(type(v) not in (int, float) or not math.isfinite(v) or v < 0 for v in samples)):
        raise ValueError(f'invalid benchmark record: {item}')


def metrics(output):
    records = []
    for line in output.splitlines():
        if 'LENZ_PERF ' not in line:
            continue
        item = json.loads(line.split('LENZ_PERF ', 1)[1])
        validate_metric(item)
        records.append(item)
    return records


def compare(before, after, threshold):
    for dataset in (before, after):
        for name, item in dataset.items():
            validate_metric(dict(item, name=name))
    changes = {}
    uncomparable = []
    for key in sorted(before.keys() & after.keys()):
        if before[key]['unit'] != after[key]['unit']:
            raise ValueError(f'unit mismatch for {key}')
        old, new = (statistics.median(rows[key]['samples']) for rows in (before, after))
        if old > 0:
            changes[key] = (new / old - 1) * 100
        elif new == 0:
            changes[key] = 0
        else:
            uncomparable.append(key)
    return {'changes': changes, 'regressions': [k for k,v in changes.items() if v > threshold],
            'missing': sorted(before.keys() - after.keys()), 'added': sorted(after.keys() - before.keys()),
            'uncomparable': uncomparable}


def owned_descendants(pid):
    # Linux native probes launch isolated sessions; retain birth ticks so a reused
    # PID cannot make cancellation touch an unrelated process.
    # Some Linux kernels omit task/*/children. A PPID snapshot also includes
    # children started by any thread, rather than just the main thread.
    children = {}
    for entry in Path('/proc').glob('[0-9]*'):
        try:
            stat = (entry / 'stat').read_text().rsplit(')', 1)[1].split()
            children.setdefault(int(stat[1]), []).append((int(entry.name), stat[19]))
        except (OSError, ValueError, IndexError):
            continue  # A process may exit while the snapshot is read.
    pending=[pid];owned={}
    while pending:
        parent=pending.pop()
        for child,birth in children.get(parent, []):
            if child not in owned:owned[child]=birth;pending.append(child)
    return owned


def owned_running(pid,birth):
    try:stat=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
    except OSError:return False
    return stat[19]==birth and stat[0]!='Z'


def run(command, log, env, timeout, cwd=None, cancel_signal=signal.SIGTERM):
    """Run sequentially; preserve output, sample child RSS, kill its group on timeout/cancel."""
    start = time.monotonic()
    peak = None
    cpu = None
    timed_out = False
    with Path(log).open('w') as output:
        proc = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT, env=env, cwd=cwd, start_new_session=True)
        try:
            while proc.poll() is None:
                if time.monotonic() - start >= timeout:
                    timed_out = True
                    break
                try:
                    sample = process_stats(proc.pid)
                    value = sample['peak_rss_mib'] or sample['rss_mib']
                    if value is not None:
                        peak = max(peak or 0, value)
                    cpu = sample['cpu_seconds']
                except (OSError, subprocess.SubprocessError):
                    pass  # Process can exit between poll and sampling.
                time.sleep(.05)
        finally:
            if proc.poll() is None:
                # Process discovery can be slow on a busy host. Stop the owned group first, retaining its
                # parent links for detached-child discovery without letting timed-out work continue.
                with suppress(ProcessLookupError):os.killpg(proc.pid,signal.SIGSTOP)
                try:
                    owned=owned_descendants(proc.pid)
                finally:
                    with suppress(ProcessLookupError):
                        if cancel_signal==signal.SIGINT:
                            proc.send_signal(signal.SIGINT)  # Python finally closes native apps/displays.
                        else:os.killpg(proc.pid,cancel_signal)
                    with suppress(ProcessLookupError):os.killpg(proc.pid,signal.SIGCONT)
                try:proc.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    with suppress(ProcessLookupError):os.killpg(proc.pid,signal.SIGKILL)
                    proc.wait()
                # Cargo children share the group; native app children may own a
                # separate session. Cover both if cooperative cleanup was bypassed.
                with suppress(ProcessLookupError):os.killpg(proc.pid,signal.SIGTERM)
                for pid,birth in owned.items():
                    if owned_running(pid,birth):
                        with suppress(ProcessLookupError):os.kill(pid,signal.SIGTERM)
                deadline=time.monotonic()+2
                while any(owned_running(pid,birth) for pid,birth in owned.items()) and time.monotonic()<deadline:time.sleep(.02)
                for pid,birth in owned.items():
                    if owned_running(pid,birth):
                        with suppress(ProcessLookupError):os.kill(pid,signal.SIGKILL)
    return {'status': 'timeout' if timed_out else 'passed' if proc.returncode == 0 else 'failed',
            'returncode': proc.returncode, 'wall_seconds': round(time.monotonic() - start, 3),
            'sampled_process_peak_mib': peak, 'sampled_process_cpu_seconds': cpu,
            'command': [str(v) for v in command], 'log': str(log)}


def tail_growth(samples):
    """Compare the last two quarters, after the initial working set/allocator warmup."""
    if len(samples) < 4:
        return None
    width = len(samples) // 4
    return statistics.median(samples[-width:]) - statistics.median(samples[-2*width:-width])


def cargo_executable(output, kind, name, test):
    """Select the requested Cargo target, rejecting missing/ambiguous harnesses."""
    candidates = set()
    for line in output.splitlines():
        try:
            item = json.loads(line)
        except ValueError:
            continue
        if (item.get('reason') == 'compiler-artifact' and item.get('executable')
                and kind in item.get('target', {}).get('kind', [])
                and item.get('target', {}).get('name') == name
                and item.get('profile', {}).get('test') == test):
            candidates.add(item['executable'])
    if len(candidates) != 1:
        raise ValueError(f'Expected one {kind} {name} test={test} executable, got {sorted(candidates)}')
    return candidates.pop()

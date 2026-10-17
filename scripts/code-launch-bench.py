#!/usr/bin/env python3
"""Reader launch and memory acceptance: on Linux an owned private Xvfb display (never the user's desktop); with
`--native` (macOS) real windows on the Mac's display, memory as physical footprint with resident size beside it.

Needs a release `lenz-reader` built with `--features perf-probe` (benchmark markers on stderr), Xvfb and a CPU
Vulkan driver (Mesa lavapipe: `lvp_icd.json`). Every launch opens the file under the corpus as its root, with its own
XDG config/state; "warm" reuses one reader cache folder across launches, "fresh" gives each launch an empty one. Mesa's
shader cache is one warmed folder for both (it is the renderer's, not the reader's). Output: JSON samples and a
summary.

  scripts/code-launch-bench.py --binary target/release/lenz-reader --file <file> --corpus <dir> --out <dir>
"""
import argparse
import ctypes
import platform
import json
import os
from pathlib import Path
import re
import shutil
import signal
import statistics
import subprocess
import tempfile
import time

ICD = '/usr/share/vulkan/icd.d/lvp_icd.json'


class _RusageInfoV4(ctypes.Structure):
    _fields_ = [('ri_uuid', ctypes.c_uint8 * 16)] + [(f'f{i}', ctypes.c_uint64) for i in range(40)]


def macos_memory(pid):
    """Physical footprint, its lifetime peak and resident size in MiB (proc_pid_rusage, RUSAGE_INFO_V4)."""
    info = _RusageInfoV4()
    if ctypes.CDLL('/usr/lib/libproc.dylib').proc_pid_rusage(pid, 4, ctypes.byref(info)) != 0:
        return None
    mib = 1024 * 1024
    return info.f7 / mib, info.f28 / mib, info.f6 / mib  # ri_phys_footprint, ri_lifetime_max_phys_footprint, ri_resident_size


def status(pid):
    """Memory and its peak in MiB (Linux VmRSS/VmHWM; macOS physical footprint and its peak), or None once gone."""
    if platform.system() == 'Darwin':
        m = macos_memory(pid)
        return m and m[:2]
    try:
        text = Path(f'/proc/{pid}/status').read_text()
    except OSError:
        return None
    field = lambda name: int(re.search(rf'^{name}:\s+(\d+)', text, re.M)[1]) / 1024
    return field('VmRSS'), field('VmHWM')


def descendants(pid):
    """RSS in MiB of every process below `pid` (language servers and their helpers)."""
    out = subprocess.run(['ps', '-A', '-o', 'pid=,ppid=,rss='], capture_output=True, text=True).stdout
    rows = [tuple(int(x) for x in line.split()) for line in out.splitlines() if line.strip()]
    children, rss = {}, {}
    for p, pp, r in rows:
        children.setdefault(pp, []).append(p)
        rss[p] = r
    total, stack = 0, list(children.get(pid, []))
    while stack:
        p = stack.pop()
        total += rss.get(p, 0)
        stack.extend(children.get(p, []))
    return total / 1024


def launch(binary, args, display, home, cache, extra_env=None, shaders=None):
    env = {k: v for k, v in os.environ.items() if k not in ('WAYLAND_DISPLAY', 'XDG_SESSION_TYPE')}
    env.update(XDG_CONFIG_HOME=str(home / 'config'), XDG_STATE_HOME=str(home / 'state'), XDG_CACHE_HOME=str(cache), XDG_DATA_HOME=str(home / 'data'))
    if display:
        env.update(DISPLAY=display, VK_ICD_FILENAMES=ICD)
    if shaders:
        env['MESA_SHADER_CACHE_DIR'] = str(shaders)
    env.update(extra_env or {})
    return subprocess.Popen([binary, *args], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)


def wait_for(proc, marker, limit=30):
    """Seconds from now until `marker` appears on stderr (the reader's own launch-relative ms too)."""
    started = time.monotonic()
    os.set_blocking(proc.stderr.fileno(), False)
    seen = ''
    while time.monotonic() - started < limit:
        try:
            seen += os.read(proc.stderr.fileno(), 65536).decode(errors='replace')
        except BlockingIOError:
            pass
        m = re.search(rf'lenz-reader: {marker} ([\d.]+) ms', seen)
        if m:
            return (time.monotonic() - started) * 1000, float(m[1]), seen
        if proc.poll() is not None:
            break
        time.sleep(0.002)
    raise RuntimeError(f'no "{marker}" marker: {seen[-2000:]}')


def stop(proc):
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait()


def p95(samples):
    s = sorted(samples)
    return s[round((len(s) - 1) * 0.95)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--binary', required=True)
    ap.add_argument('--file', required=True, help='an ordinary source file to open at launch')
    ap.add_argument('--corpus', required=True, help='a project folder for the ten-tab memory run')
    ap.add_argument('--out', required=True)
    ap.add_argument('--trials', type=int, default=100)
    ap.add_argument('--display', default=':91')
    ap.add_argument('--geometry', default='1600x1000x24')
    ap.add_argument('--native', action='store_true', help='macOS: real windows on this display (no Xvfb)')
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    xvfb = None
    if a.native:
        a.display = None
    else:
        if not Path(ICD).exists():
            raise SystemExit(f'missing {ICD}: install Mesa lavapipe (vulkan-swrast)')
        xvfb = subprocess.Popen(['Xvfb', a.display, '-screen', '0', a.geometry, '-nolisten', 'tcp'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(1)
    scratch = Path(tempfile.mkdtemp(prefix='reader-launch-'))
    results = {'binary': a.binary, 'file': a.file, 'corpus': a.corpus, 'platform': platform.platform(), 'display': a.display or 'native', 'geometry': None if a.native else a.geometry, 'icd': None if a.native else ICD, 'memory': 'macOS physical footprint (resident beside it)' if platform.system() == 'Darwin' else 'Linux VmRSS/VmHWM'}
    try:
        for kind in ('warm', 'fresh'):
            outside, inside = [], []
            warm_cache = scratch / 'warm-cache'
            for i in range(a.trials + 3):
                home = scratch / f'{kind}-{i}'
                cache = warm_cache if kind == 'warm' else home / 'cache'
                proc = launch(a.binary, [a.file], a.display, home, cache, {'READER_PROBE_ROOT': a.corpus}, scratch / 'shaders')
                try:
                    wall, own, _ = wait_for(proc, 'readable')
                finally:
                    stop(proc)
                shutil.rmtree(home, ignore_errors=True)
                if i >= 3:  # the first launches warm the OS page cache and, for "warm", the reader's cache
                    outside.append(wall)
                    inside.append(own)
            target = 250 if kind == 'warm' else 1000
            within = sum(x <= target for x in outside) / len(outside)
            results[f'launch-{kind}'] = {'unit': 'ms', 'samples': outside, 'reader_reported': inside, 'p95': p95(outside), 'within_target': within, 'target': target}
            print(f'{kind} launch to readable: median {statistics.median(outside):.1f} ms, p95 {p95(outside):.1f} ms, {within:.0%} within {target} ms over {len(outside)} ({"PASS" if within >= 0.95 else "FAIL"})')
        # Ten kept tabs of ordinary files (≤1 MiB) with the corpus inventory loaded.
        # The ten largest source files up to 1 MiB (Rust: its language server runs, accounted separately).
        files = sorted((p for p in Path(a.corpus).rglob('*.rs') if p.is_file() and '.git' not in p.parts and 0 < p.stat().st_size <= 1 << 20), key=lambda p: -p.stat().st_size)
        picks = files[:10]
        home = scratch / 'memory'
        proc = launch(a.binary, [], a.display, home, home / 'cache', {'READER_PROBE_ROOT': a.corpus, 'READER_PROBE_OPEN': ':'.join(map(str, picks))}, scratch / 'shaders')
        try:
            wait_for(proc, 'tabs ready', 60)
            samples, peak, servers, resident = [], 0.0, [], []
            for _ in range(120):  # 60 s: 30 s to settle (indexing, the language server), 30 s settled
                rss, hwm = status(proc.pid)
                samples.append(rss)
                if platform.system() == 'Darwin':
                    resident.append(macos_memory(proc.pid)[2])
                peak = max(peak, hwm)
                servers.append(descendants(proc.pid))
                time.sleep(0.5)
        finally:
            stop(proc)
        settled = statistics.median(samples[-60:])
        results['ten-tabs'] = {'files': list(map(str, picks)), 'bytes': sum(p.stat().st_size for p in picks), 'rss_mib': samples, 'resident_mib': resident, 'settled_mib': settled, 'peak_mib': peak, 'children_mib': servers}
        print(f'ten tabs + inventory: settled {settled:.1f} MiB (≤250: {"PASS" if settled <= 250 else "FAIL"}), peak {peak:.1f} MiB (≤512: {"PASS" if peak <= 512 else "FAIL"}); language servers and helpers {max(servers):.1f} MiB, accounted separately')
    finally:
        if xvfb:
            xvfb.terminate()
        shutil.rmtree(scratch, ignore_errors=True)
    (out / 'launch-bench.json').write_text(json.dumps(results, indent=1))


if __name__ == '__main__':
    main()

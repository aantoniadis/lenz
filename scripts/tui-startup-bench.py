#!/usr/bin/env python3
"""TUI startup matrix over the perf-suite GUI startup fixtures (`<suite output>/fixtures`), in an owned 120x36 PTY.

Pairs with scripts/startup-bench.py. First frame is the first bytes drawn after entering the
alternate screen (usually "Loading comparison"). First content is when a fixture marker from
the opened comparison reaches the PTY, the analogue of the GUI first paint, which follows load
and line diff. RSS is sampled `--seconds` after first frame. Settled is the last output byte
before `--quiet` seconds of silence, after background syntax/refinement repaints.
After settling, `--inputs` key presses measure first PTY byte after each key (first feedback,
not completed presentation), matching the GUI interaction probe's first-damage boundary.
Prints one JSON line per measured launch.
"""
import argparse, importlib.util, json, subprocess, tempfile, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('tui_bench', Path(__file__).with_name('tui-bench.py'))
bench = importlib.util.module_from_spec(spec); spec.loader.exec_module(bench)
from perf_support import prepare_gui_launch, process_stats, isolate_user_state

ALT = b'\x1b[?1049h'
PAGE_DOWN, DOWN = b'\x1b[6~', b'\x1b[B'


def tree_rss_mib(root):
    """RSS summed over the process and its descendants (nvim runs its UI and core as two processes)."""
    parents = {}
    if Path('/proc').is_dir():
        for proc in Path('/proc').iterdir():
            if proc.name.isdigit():
                try:
                    parents[int(proc.name)] = int((proc/'stat').read_text().rsplit(')', 1)[1].split()[1])
                except (OSError, ValueError, IndexError):
                    pass
    else:  # macOS has no /proc; BSD ps lists every pid with its parent.
        for line in subprocess.check_output(['ps', '-A', '-o', 'pid=', '-o', 'ppid='], text=True).splitlines():
            pid, parent = line.split()
            parents[int(pid)] = int(parent)
    if not parents:
        return None
    tree, grew = {root}, True
    while grew:
        children = {pid for pid, parent in parents.items() if parent in tree} - tree
        tree |= children
        grew = bool(children)
    total = 0
    for pid in tree:
        try:
            total += process_stats(pid)['rss_mib'] or 0
        except (OSError, ProcessLookupError):
            pass
    return round(total, 2)


def launch(app, args, cwd, seconds, quiet, marker, key=None, inputs=0, allow_unsettled=False):
    probe = bench.TerminalProbe([app, *args], cwd)
    try:
        first = None
        while first is None:
            if probe.proc.poll() is not None or time.monotonic() - probe.started > 30:
                raise RuntimeError(f'no frame; exit={probe.proc.poll()} tail={bytes(probe.output[-400:])!r}')
            probe.read(.005)
            at = probe.output.find(ALT)
            if at >= 0 and len(probe.output) > at + len(ALT):
                first = (time.monotonic() - probe.started) * 1000
        content = (probe.expect(marker, 30) if marker not in probe.output
                   else first)  # marker already in the first frame
        last = content
        deadline = time.monotonic() + 30
        rss = peak = tree = resident = None
        while time.monotonic() < deadline:
            if probe.read(.01):
                last = (time.monotonic() - probe.started) * 1000
            now = (time.monotonic() - probe.started) * 1000
            if rss is None and now - first >= seconds * 1000:
                s = process_stats(probe.proc.pid); rss, peak, resident = s['rss_mib'], s['peak_rss_mib'], s.get('resident_mib')
                tree = tree_rss_mib(probe.proc.pid)
            if rss is not None and now - last >= quiet * 1000:
                break
        else:
            if not allow_unsettled:
                raise RuntimeError('terminal never settled')
            last = None  # still writing after 30 s (e.g. loading a huge history); content time stands
        cpu = process_stats(probe.proc.pid)['cpu_seconds']
        feedback = [round(probe.input_latency(key), 3) for _ in range(inputs if key else 0)]
        return {'first_frame_ms': round(first, 2), 'first_content_ms': round(content, 2), 'settled_ms': round(last, 2) if last is not None else None, 'rss_mib': round(rss, 2),
                'peak_rss_mib': round(peak, 2) if peak else None, 'resident_mib': round(resident, 2) if resident else None,
                'tree_rss_mib': tree, 'cpu_seconds': cpu, 'bytes': len(probe.output),
                **({'key': key.decode(), 'input_first_byte_ms': feedback} if feedback else {})}
    finally:
        probe.close()


def main():
    isolate_user_state()  # never measure with the user's own settings or saved positions
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--app', required=True)
    cli.add_argument('--fixtures', required=True, type=Path)
    cli.add_argument('--runs', type=int, default=3)
    cli.add_argument('--warmups', type=int, default=1)
    cli.add_argument('--seconds', type=float, default=3)
    cli.add_argument('--quiet', type=float, default=.5)
    cli.add_argument('--inputs', type=int, default=30, help='Key presses per measured launch where a key is defined')
    opt = cli.parse_args()
    f = opt.fixtures
    pair = lambda n, l, r, marker: (n, [str(f/n/l), str(f/n/r)], ROOT, marker)
    head = subprocess.run(['git', '-C', str(f/'history'), 'log', '-1', '--format=%h'], capture_output=True,
                          text=True, check=True).stdout.strip().encode()
    # Markers avoid spaces: an incremental frame may move the cursor over unchanged blank cells.
    cases = [pair('small', 'left.txt', 'right.txt', b'TWO'), pair('generated-100k', 'left.rs', 'right.rs', b'compute_1') + (PAGE_DOWN,),
             pair('different-100k', 'left.rs', 'right.rs', b'right0'), pair('images-4k', 'left.png', 'right.png', b'regions'),
             ('merge', ['merge', *[str(f/'merge'/f'{s}.rs') for s in ('base', 'ours', 'theirs', 'merged')]], ROOT, b'ours', PAGE_DOWN),
             ('repository', ['log'], f/'history', head, DOWN),
             ('many-small', [str(f/'many-small'/'left'), str(f/'many-small'/'right')], ROOT, b'file-0000'),
             ('about-standalone', ['--about'], Path(tempfile.gettempdir()), b'Lenz')]
    app = str(Path(opt.app).resolve())
    for name, args, cwd, marker, *key in cases:
        key = key[0] if key else None
        with tempfile.TemporaryDirectory(prefix='lenz-tui-startup-') as scratch:
            run_args, merge_cwd = prepare_gui_launch(args, Path(scratch)/'merge-inputs')
            where = merge_cwd or cwd
            for _ in range(opt.warmups):
                launch(app, run_args, where, opt.seconds, opt.quiet, marker)
            for run in range(1, opt.runs + 1):
                print(json.dumps({'case': name, 'run': run, **launch(app, run_args, where, opt.seconds, opt.quiet, marker, key, opt.inputs)}), flush=True)


if __name__ == '__main__':
    main()

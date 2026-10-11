#!/usr/bin/env python3
"""Long-session memory probe for lenz-tui in an owned PTY (Linux).

Opens a directory pair, then for each round moves forward through every file with `]` and back
with `[`, waiting for the terminal to go quiet after each key. Prints one JSON line per round with
the process tree's RSS, so growth across a long session is visible. Use disposable fixtures, e.g.
`<perf-suite output>/fixtures/files/{left,right}` (12 large Rust files: each first visit parses syntax).
"""
import argparse, importlib.util, json, sys, time
from pathlib import Path
from perf_support import isolate_user_state

SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPTS))
spec = importlib.util.spec_from_file_location('tui_bench', SCRIPTS/'tui-bench.py')
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
spec = importlib.util.spec_from_file_location('tui_startup', SCRIPTS/'tui-startup-bench.py')
argv, sys.argv = sys.argv, [sys.argv[0], '--app', 'x', '--fixtures', '.']
startup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(startup)
sys.argv = argv

isolate_user_state()  # never measure with the user's own settings or saved positions
cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument('--app', type=Path, required=True)
cli.add_argument('--rounds', type=int, default=50)
cli.add_argument('--files', type=int, default=12)
cli.add_argument('left', type=Path)
cli.add_argument('right', type=Path)
opt = cli.parse_args()

bench.TerminalProbe.answer_queries = True
probe = bench.TerminalProbe([str(opt.app.resolve()), str(opt.left.resolve()), str(opt.right.resolve())], '.')
try:
    probe.quiet(.5, timeout=60)
    start = time.monotonic()
    print(json.dumps({'round': 0, 'elapsed_s': 0, 'tree_rss_mib': startup.tree_rss_mib(probe.proc.pid)}), flush=True)
    for round_ in range(1, opt.rounds + 1):
        latency = [probe.input_latency(key) for key in [b']'] * (opt.files - 1) + [b'['] * (opt.files - 1)]
        probe.quiet(.3, timeout=60)
        print(json.dumps({'round': round_, 'elapsed_s': round(time.monotonic() - start, 1),
                          'tree_rss_mib': startup.tree_rss_mib(probe.proc.pid),
                          'median_key_ms': round(sorted(latency)[len(latency) // 2], 3)}), flush=True)
    probe.finish(key=b'\x11')
finally:
    probe.close()

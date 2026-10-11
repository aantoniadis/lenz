#!/usr/bin/env python3
"""Repeat all registered workflows, retaining failures and coverage counts.

GPUI uses its private test platform; TUI uses owned TestBackend instances.
Optional PTY replay owns its terminal and disposable Git repositories.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import signal
from perf_support import run as run_process
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
RESULT = re.compile(r'test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored;')


def run(command, log, timeout):
    started = time.monotonic()
    print(' '.join(command), flush=True)
    result=run_process(command,log,dict(os.environ),timeout,cwd=ROOT,cancel_signal=signal.SIGINT)
    code=result['returncode']
    if result['status']=='timeout':
        with log.open('a') as output:output.write(f'\nWorkflow check exceeded {timeout}s\n')
        code=124
    counts = [tuple(map(int, match)) for match in RESULT.findall(log.read_text())]
    # An accidentally filtered-out Rust suite must not count as coverage.
    if command[0] == 'cargo' and code == 0 and not any(passed for passed, _, _ in counts):
        code = 1
    return {'command': command, 'exit': code, 'seconds': round(time.monotonic() - started, 3),
            'log': str(log), 'passed': sum(c[0] for c in counts),
            'failed': sum(c[1] for c in counts), 'ignored': sum(c[2] for c in counts)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--timeout', type=int, default=600, help='Seconds per check')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/user-flow-checks')
    parser.add_argument('--tui-app', type=Path, help='Also replay the real production TUI in a private PTY')
    parser.add_argument('--gui-app', type=Path, help='Also replay matching production GUI/TUI disk boundaries')
    parser.add_argument('--xvfb', type=Path, help='Private Xvfb executable for --gui-app')
    args = parser.parse_args()
    if args.rounds < 1 or args.timeout < 1:
        parser.error('rounds and timeout must be positive')
    if args.gui_app and (not args.tui_app or not args.xvfb):
        parser.error('--gui-app requires --tui-app and --xvfb')
    args.output.mkdir(parents=True, exist_ok=True)
    commands = [(f'workspace-{i + 1}', ['cargo', 'test', '--locked', '--workspace', '--features', 'gui-test'])
                for i in range(args.rounds)]
    commands += [
        ('gui-adapter', ['cargo', 'test', '--locked', '-p', 'lenz', '--features', 'gui-test', 'gui_']),
        ('core-without-gui', ['cargo', 'test', '--locked', '-p', 'lenz', '--no-default-features']),
        ('collectors', [sys.executable, '-m', 'unittest', 'discover', '-s', 'scripts', '-p', 'test_*.py']),
    ]
    if args.tui_app:
        commands.append(('pty', [sys.executable, 'scripts/tui-workflow-bench.py', '--app',
                                str(args.tui_app.resolve()), '--lines', '300', '--rounds', str(args.rounds)]))
    if args.gui_app:
        commands.append(('native', [sys.executable, 'scripts/native-user-flows.py', '--gui-app', str(args.gui_app.resolve()),
                                   '--tui-app', str(args.tui_app.resolve()), '--xvfb', str(args.xvfb.resolve()),
                                   '--rounds', str(args.rounds), '--output', str(args.output.resolve() / 'native')]))
    report = {'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'rounds': args.rounds, 'platform': sys.platform,
              'random_seed_override': os.environ.get('LENZ_FLOW_SEED'), 'checks': [],
              'limits': ['Registered tests are repeated, not exhaustive state exploration.',
                         'Ignored stress tests and live GitHub are not run by this command.',
                         'Native GUI painting is checked only when --gui-app is supplied; screenshots need visual review.']}
    for name, command in commands:
        check = run(command, args.output / f'{name}.log', args.timeout)
        report['checks'].append(check)
        (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
        print(f"{name}: exit {check['exit']}; {check['passed']} Rust tests passed; log: {check['log']}", flush=True)
        if check['exit']:
            return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())

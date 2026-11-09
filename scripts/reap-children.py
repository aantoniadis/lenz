#!/usr/bin/env python3
"""Own and reap a validation command's orphaned descendants on Linux.

    python3 scripts/reap-children.py -- cargo test --workspace --locked

Other platforms exec the command directly. On Linux, preserve its exit status;
interruption returns 128 + signal, and forced cleanup makes a successful command
fail. This cannot reap processes already adopted by another parent.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def supervise(command, grace):
    if sys.platform != 'linux':
        os.execvp(command[0], command)
    children_path = Path(f'/proc/self/task/{os.getpid()}/children')
    Path('/proc/self/stat').read_text()  # Required for safe cleanup; check before starting a command.

    def direct_children():
        # Some kernels omit task/*/children (CONFIG_CHECKPOINT_RESTORE).
        if children_path.exists():
            return list(map(int, children_path.read_text().split()))
        found = []
        for entry in Path('/proc').iterdir():
            if entry.name.isdigit():
                try:
                    fields = (entry/'stat').read_text(errors='replace').rsplit(')', 1)[1].split()
                    if int(fields[1]) == os.getpid():
                        found.append(int(entry.name))
                except (OSError, IndexError, ValueError):
                    continue
        return found
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))
    env = dict(os.environ, LENZ_VALIDATION_SUPERVISOR_PID=str(os.getpid()))
    pending = interrupted = None

    def received(number, _frame):
        nonlocal pending, interrupted
        pending = interrupted = number

    previous = {number: signal.signal(number, received) for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
    started = time.monotonic()
    try:
        process = subprocess.Popen(command, env=env, start_new_session=True)
        code = None
        reaped = 0
        shutdown = None
        forced = False
        while True:
            if pending is not None:
                # The unreaped command pins its PID and original process-group ID.
                if code is None:
                    try:
                        os.killpg(process.pid, pending)
                    except ProcessLookupError:
                        pass
                pending = None
                if shutdown is None:
                    shutdown = time.monotonic()
            while True:
                try:
                    pid, status = os.waitpid(-1, os.WNOHANG)
                except ChildProcessError:
                    alive = False
                    break
                if not pid:
                    alive = True
                    break
                if pid == process.pid:
                    process.returncode = code = os.waitstatus_to_exitcode(status)
                    if shutdown is None:
                        shutdown = time.monotonic()
                else:
                    reaped += 1
            if not alive:
                break
            if shutdown is not None:
                elapsed = time.monotonic() - shutdown
                if elapsed >= grace:
                    forced = True
                    number = signal.SIGTERM if elapsed < grace + 1 else signal.SIGKILL
                    # Only direct children: their PIDs stay reserved until this
                    # supervisor reaps them. Killed parents transfer their children
                    # here, so later passes also cover detached grandchildren.
                    for pid in direct_children():
                        try:
                            os.kill(pid, number)
                        except ProcessLookupError:
                            pass
                if elapsed > grace + 4:
                    raise RuntimeError('owned children could not be stopped and reaped')
            time.sleep(.01)
        result = 128 + interrupted if interrupted else (128 - code if code is not None and code < 0 else code)
        if result is None:
            raise RuntimeError('command status was not collected')
        if forced and result == 0:
            result = 1
        print('VALIDATION_CHILDREN ' + json.dumps(dict(returncode=result, command_returncode=code,
              reaped_orphans=reaped, forced_cleanup=forced, interrupted=interrupted,
              seconds=time.monotonic() - started)), file=sys.stderr, flush=True)
        return result
    finally:
        for number, handler in previous.items():
            signal.signal(number, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--grace-seconds', type=float, default=2)
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command or not 0 <= args.grace_seconds <= 30:
        parser.error('a command and a finite grace period between 0 and 30 seconds are required')
    try:
        return supervise(command, args.grace_seconds)
    except (OSError, RuntimeError) as error:
        print(f'validation supervisor: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())

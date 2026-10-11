#!/usr/bin/env python3
"""macOS: cancelling the GUI under `git mergetool` must leave the conflict unresolved.

With `mergetool.lenz.trustExitCode true`, git stages the merged file when Lenz exits 0. A cancelled
or quit Merge View must exit 1 (README, "git mergetool"), so git restores its backup and keeps the
path unmerged. AppKit ends the process with exit(0) on quit, which once turned every cancel into a
success here. This quits the window the way Cmd-Q does (NSRunningApplication.terminate) in a
disposable repository, with git configured for that repository only. It opens one native window.

    python3 scripts/check-mergetool-cancel.py --app target/release/lenz
"""
import argparse
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import tempfile
import time

CONFLICT = b'a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nz\n'


def git(repo, *args, env):
    return subprocess.run(['git', *args], cwd=repo, env=env, check=True, capture_output=True).stdout


def quit_like_cmd_q(pid):
    """Ask the app to quit, as Cmd-Q or the Dock's Quit does (no Accessibility or key events)."""
    script = (f"ObjC.import('AppKit'); var app = $.NSRunningApplication."
              f"runningApplicationWithProcessIdentifier({pid}); app.isNil() ? 'gone' : app.terminate")
    return subprocess.run(['osascript', '-l', 'JavaScript', '-e', script], capture_output=True, text=True).stdout.strip()


def main():
    cli = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    cli.add_argument('--app', required=True, type=Path, help='the lenz GUI executable')
    cli.add_argument('--settle', type=float, default=3.0, help='seconds for the window to open before quitting')
    opt = cli.parse_args()
    if platform.system() != 'Darwin':
        sys.exit('macOS only: on Linux the event loop returns and the exit code is covered by the GUI tests')
    app = opt.app.resolve()
    with tempfile.TemporaryDirectory(prefix='lenz-mergetool-') as scratch:
        repo, pidfile = Path(scratch, 'repo'), Path(scratch, 'lenz.pid')
        env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', GIT_AUTHOR_NAME='t',
                   GIT_AUTHOR_EMAIL='t@example.invalid', GIT_COMMITTER_NAME='t', GIT_COMMITTER_EMAIL='t@example.invalid',
                   XDG_CONFIG_HOME=str(Path(scratch, 'config')), XDG_STATE_HOME=str(Path(scratch, 'state')))
        repo.mkdir()
        git(repo, 'init', '-q', '-b', 'main', env=env)
        for branch, text in (('main', b'a\nbase\nz\n'),):
            (repo / 'f.txt').write_bytes(text); git(repo, 'add', 'f.txt', env=env); git(repo, 'commit', '-qm', 'base', env=env)
        git(repo, 'switch', '-qc', 'other', env=env)
        (repo / 'f.txt').write_bytes(b'a\ntheirs\nz\n'); git(repo, 'commit', '-qam', 'theirs', env=env)
        git(repo, 'switch', '-q', 'main', env=env)
        (repo / 'f.txt').write_bytes(b'a\nours\nz\n'); git(repo, 'commit', '-qam', 'ours', env=env)
        subprocess.run(['git', 'merge', 'other'], cwd=repo, env=env, capture_output=True)
        assert git(repo, 'ls-files', '-u', env=env), 'fixture must start with an unmerged path'
        for key, value in (('merge.tool', 'lenz'), ('mergetool.prompt', 'false'), ('mergetool.lenz.trustExitCode', 'true'),
                           # git evals the command in a subshell, whose $$ is its parent's; a child sh's $$ is the lenz it execs
                           ('mergetool.lenz.cmd', f'sh -c \'echo $$ > "{pidfile}"; exec "$0" merge "$1" "$2" "$3" "$4"\' '
                                                  f'"{app}" "$BASE" "$LOCAL" "$REMOTE" "$MERGED"')):
            git(repo, 'config', key, value, env=env)
        tool = subprocess.Popen(['git', 'mergetool'], cwd=repo, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                start_new_session=True)
        try:
            deadline = time.monotonic() + 30
            while not (pidfile.exists() and pidfile.read_text().strip()):
                if time.monotonic() > deadline or tool.poll() is not None:
                    sys.exit(f'lenz never started: {tool.communicate()[0].decode(errors="replace")}')
                time.sleep(0.1)
            pid = int(pidfile.read_text())
            time.sleep(opt.settle)
            asked = quit_like_cmd_q(pid)
            output = tool.communicate(timeout=30)[0].decode(errors='replace')
        finally:
            if tool.poll() is None:
                os.killpg(tool.pid, signal.SIGTERM)
                tool.wait()
        unmerged = git(repo, 'ls-files', '-u', env=env)
        merged = (repo / 'f.txt').read_bytes()
        print(f'quit requested: {asked}; git mergetool exit {tool.returncode}; unmerged entries {len(unmerged.splitlines())}')
        failures = []
        if tool.returncode == 0:
            failures.append('git mergetool reported success for a cancelled merge')
        if not unmerged:
            failures.append('the path was staged as resolved')
        if merged != CONFLICT:
            failures.append(f'the merged file changed: {merged!r}')
        if failures:
            print(output)
            sys.exit('FAIL: ' + '; '.join(failures))
        print('ok: the cancelled merge stays unresolved')


if __name__ == '__main__':
    main()

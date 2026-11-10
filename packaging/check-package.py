#!/usr/bin/env python3
"""Check an unpacked, owned release package without opening windows or a terminal."""
import argparse
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('version')
    args = parser.parse_args()
    directory = args.directory.resolve(strict=True)

    def run(name, flags, status=0, text=None):
        command = [str(directory / name), *flags]
        result = subprocess.run(command, capture_output=True, text=True, timeout=20)
        output = result.stdout + result.stderr
        if result.returncode != status or text is not None and text not in output:
            raise RuntimeError(f'{command}: exit {result.returncode}, expected {status}; {output}')
        print(f'passed: {name} {" ".join(flags)}')

    for name in ['lenz', 'lenz-code', 'lenz-code-tui', 'lenz-diff', 'lenz-diff-tui']:
        run(name, ['--version'], text=args.version)
    run('lenz', ['--help'], text='Usage: lenz [file-or-folder]')
    run('lenz', ['--tui', '--help'], text='Usage: lenz --tui')
    run('lenz', ['diff', '--bogus'], status=2, text='usage: lenz diff')
    run('lenz', ['diff', '--tui', '--bogus'], status=2, text='usage: lenz diff --tui')
    if sys.platform == 'darwin':
        subprocess.run(['codesign', '-v', '--strict', str(directory / 'Lenz.app')], check=True)
        print('passed: strict macOS bundle signature')


if __name__ == '__main__':
    main()

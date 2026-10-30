#!/usr/bin/env python3
"""Focused ordinary Linux TUI input/Save/close reachability; no timing or stress workload."""
import argparse
import base64
import importlib.util
import json
from pathlib import Path
import time

spec = importlib.util.spec_from_file_location('reader_probe', Path(__file__).with_name('code-tui-bench.py'))
probe_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe_module)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--source', required=True)
    args = parser.parse_args()
    root = args.out.resolve()
    root.mkdir(parents=True, exist_ok=False)
    fixture = root / 'default.txt'
    original = b'saved\r\n'
    expected = 'qλ😀\r\npasted'.encode()
    fixture.write_bytes(original)
    config = root / 'config/lenz-code'
    config.mkdir(parents=True)
    (config / 'providers.json').write_text('{"providers":{}}\n')
    binary = args.binary.resolve(strict=True)
    result = dict(schema=1, source=args.source, binary=str(binary), binary_sha256=probe_module.sha(binary),
                  driver_sha256=probe_module.sha(Path(__file__)), explicit_test_opt_in=False,
                  scope='owned PTY ordinary Linux executable; correctness only, no physical-display/GUI/IME latency claim',
                  status='failed', checks=[])
    probe = None
    try:
        probe = probe_module.TerminalProbe([str(binary), str(fixture)], root, probe_module.isolated_environment(root))
        probe.expect('saved')
        probe.quiet()
        probe.send(b'\x01' + 'qλ'.encode() + b'\x1b[200~' + '😀\r\npasted'.encode() + b'\x1b[201~')
        probe.quiet()
        def copied(text):
            before = len(probe.output)
            probe.send(b'\x01\x03')
            probe.expect(b'\x1b]52;c;' + base64.b64encode(text) + b'\x07', raw=True, since=before)
            probe.quiet()
        copied(expected)
        assert fixture.read_bytes() == original
        result['checks'].append('ordinary constructor types q/Unicode and bracketed-pastes exact CRLF; Copy returns draft, disk unchanged')
        probe.send(b'\x13')
        deadline = time.monotonic() + 5
        while fixture.read_bytes() != expected:
            probe.read(.01)
            if time.monotonic() > deadline:
                raise RuntimeError('normal Ctrl-S did not save exact draft')
        probe.quiet()
        probe.send(b'!')
        probe.quiet()
        copied(b'!')
        probe.send(b'\x11')
        probe.expect('Unsaved changes')
        probe.send(b'\x13\x11x')
        probe.quiet()
        assert probe.proc.poll() is None and fixture.read_bytes() == expected
        probe.send(b'\x1b')
        probe.quiet()
        copied(b'!')
        probe.send(b'\x11')
        probe.expect('Unsaved changes')
        probe.finish(b'd')
        assert fixture.read_bytes() == expected
        result['checks'].append('Ctrl-S saves exact bytes; dirty Ctrl-Q veto/modal precedence/Cancel preserves draft; fresh Discard exits normally')
        result.update(status='passed', exit_code=probe.proc.returncode, termios_restored=probe.restored,
                      saved_utf8=expected.decode(), discarded_draft='!')
    except BaseException as error:
        result['error'] = repr(error)
        raise
    finally:
        try:
            if probe:
                probe.close()
        finally:
            if probe:
                (root / 'terminal.raw').write_bytes(probe.output)
                (root / 'screen.txt').write_text(probe.screen.text())
            (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()

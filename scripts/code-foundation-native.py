#!/usr/bin/env python3
"""Owned Linux X11 close/compose acceptance. No native or timed workload runs on import.

Requires the test-support-only foundation-native example and an explicitly released cohort campaign.
The example alone opts in; shipping application defaults/flags are unchanged. This instrumented
correctness flow is not a production typing latency measurement.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def wm_delete_fields(window, protocols, delete):
    """ICCCM protocol request, not an XDestroyWindow/XKillClient operation."""
    return dict(window=window, client_type=protocols, data=(32, [delete, 0, 0, 0, 0]))


def private_environment(original, root, display):
    env = dict(original)
    for key in ('WAYLAND_DISPLAY', 'XDG_SESSION_TYPE', 'DBUS_SESSION_BUS_ADDRESS'):
        env.pop(key, None)
    env.update(DISPLAY=display, XDG_CONFIG_HOME=str(root / 'config'), XDG_STATE_HOME=str(root / 'state'),
               XDG_CACHE_HOME=str(root / 'cache'), XDG_DATA_HOME=str(root / 'data'),
               XDG_RUNTIME_DIR=str(root / 'runtime-dir'), MESA_SHADER_CACHE_DIR=str(root / 'shaders'),
               VK_ICD_FILENAMES=original.get('VK_ICD_FILENAMES', '/usr/share/vulkan/icd.d/lvp_icd.json'), XMODIFIERS='',
               LC_ALL='C.UTF-8', XCOMPOSEFILE=str(root / 'Compose'))
    return env


def visible_windows(root, viewable, bad_window):
    result = {}
    for window in root.query_tree().children:
        try:
            if window.get_attributes().map_state == viewable:
                name = window.get_wm_name()
                if name:
                    result[name] = window
        except bad_window:
            continue  # A closing window can disappear after the server enumerates it.
    return result


def states(path):
    if not path.exists():
        return {}
    result = {}
    for line in path.read_bytes().splitlines(keepends=True):
        if not line.endswith(b'\n'):
            break
        row = json.loads(line)
        result[row['label']] = row['state']
    return result


def exact_text(state):
    source = state.get('source') or {}
    if not source.get('prefix_complete'):
        raise RuntimeError('exact text oracle refuses a truncated source prefix')
    return source['prefix']


def wait_for(predicate, label, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.01)
    raise RuntimeError('native acceptance timed out: ' + label)


def reap(proc):
    if proc is None:
        return
    if proc.poll() is None:
        proc.terminate()
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True, help='NEW directory; never reused')
    parser.add_argument('--source', required=True, help='cohort source identity, paired with executable hash')
    parser.add_argument('--scenario', choices=('close', 'compose', 'both'), default='both')
    parser.add_argument('--default-input', action='store_true', help='verify actual production capability without the explicit test opt-in')
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    args.out.mkdir()  # Refuse to overwrite prior evidence or fixtures.
    root = args.out.resolve()
    (root / 'runtime-dir').mkdir(mode=0o700)
    (root / 'Compose').write_text('include "%L"\n<dead_acute> <e> : "é"\n')
    report = {'schema': 1, 'source_claim': args.source, 'binary': str(binary), 'binary_sha256': sha(binary),
              'driver_sha256': sha(Path(__file__)), 'scenario': args.scenario, 'explicit_test_opt_in': not args.default_input,
              'scope': 'owned Xvfb/X11 native protocol and XTest delivery with test-support observation; input policy recorded separately; no physical display/Wayland/Mac or production latency claim',
              'checks': [], 'status': 'running'}
    output = root / 'result.json'
    def save():
        output.write_text(json.dumps(report, indent=2) + '\n')
    save()
    xvfb = app = connection = None
    readfd = None
    started = time.monotonic()
    with (root / 'xvfb.log').open('wb') as server_log, (root / 'app.stderr').open('wb') as app_log:
        try:
            # Native dependencies load only inside a released real run, never pure unit self-checks.
            from Xlib import X, XK, display, protocol, error
            from Xlib.ext import xtest
            from PIL import Image
            from perf_support import read_display_number
            readfd, writefd = os.pipe()
            try:
                xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(writefd), '-screen', '0', '1600x1000x24', '-nolisten', 'tcp'],
                                        pass_fds=[writefd], stdout=subprocess.DEVNULL, stderr=server_log)
            finally:
                os.close(writefd)
            try:
                name = ':' + read_display_number(readfd)
            finally:
                os.close(readfd)
                readfd = None
            report.update(display=name, xvfb_pid=xvfb.pid)
            connection = display.Display(name)
            # Configure ONLY this owned server, before GPUI reads its initial XKB map.
            first, last = connection.display.info.min_keycode, connection.display.info.max_keycode
            mappings = connection.get_keyboard_mapping(first, last - first + 1)
            compose_code = next((first + i for i, values in enumerate(mappings) if not any(values)), last)
            old_mapping = connection.get_keyboard_mapping(compose_code, 1)
            connection.change_keyboard_mapping(compose_code, [(0xfe51, 0xfe51)])  # dead_acute has a visible native preedit prefix
            connection.sync()
            env = private_environment(os.environ, root, name)
            app = subprocess.Popen([str(binary), str(root / 'app'), *(['--default-input'] if args.default_input else [])], cwd=root, env=env,
                                   stdout=subprocess.DEVNULL, stderr=app_log)
            report['app_pid'] = app.pid
            trace = root / 'app/state.jsonl'
            def state(label):
                if app.poll() is not None:
                    raise RuntimeError('acceptance app exited before approval: ' + str(app.returncode))
                return states(trace).get(label, {})
            wait_for(lambda: all(state(k).get('source') for k in ('a', 'b')), 'both production windows loaded')
            if args.default_input:
                assert all(state(k).get('text_input_enabled') is True for k in ('a', 'b')), 'ordinary X11 constructor must enable input'
            def windows():
                return visible_windows(connection.screen().root, X.IsViewable, error.BadWindow)
            found = wait_for(lambda: windows() if all('Lenz foundation ' + k in windows() for k in ('a', 'b')) else None,
                             'owned native windows')
            a, b = (found['Lenz foundation ' + k] for k in ('a', 'b'))
            report['windows'] = {'a': a.id, 'b': b.id}
            def key(symbol, *modifiers):
                codes = [connection.keysym_to_keycode(XK.string_to_keysym(k)) for k in (*modifiers, symbol)]
                if not all(codes):
                    raise RuntimeError('unmapped native key: ' + repr((symbol, modifiers)))
                for code in codes:
                    xtest.fake_input(connection, X.KeyPress, code)
                for code in reversed(codes):
                    xtest.fake_input(connection, X.KeyRelease, code)
                connection.sync()
            def focus(window):
                window.configure(stack_mode=X.Above)
                window.set_input_focus(X.RevertToParent, X.CurrentTime)
                connection.sync()
                time.sleep(.05)
            def screenshot(window, label):
                geometry = window.get_geometry()
                image = window.get_image(0, 0, geometry.width, geometry.height, X.ZPixmap, 0xffffffff)
                Image.frombytes('RGB', (geometry.width, geometry.height), image.data, 'raw', 'BGRX').save(root / (label + '.png'))
            originals = {k: (root / 'app' / (k + '.txt')).read_bytes() for k in ('a', 'b')}
            for label, window in [('a', a), ('b', b)]:
                focus(window)
                key('a', 'Control_L')
                for character in 'draft-' + label:
                    key('minus' if character == '-' else character)
                wait_for(lambda: exact_text(state(label)) == 'draft-' + label, 'actual OS typing in ' + label)
                assert state(label)['source']['dirty']
            before = {k: state(k)['source'] for k in ('a', 'b')}
            delete = connection.intern_atom('WM_DELETE_WINDOW')
            protocols = connection.intern_atom('WM_PROTOCOLS')
            def request_delete(window):
                assert delete in window.get_wm_protocols(), 'production native window does not advertise close protocol'
                window.send_event(protocol.event.ClientMessage(**wm_delete_fields(window.id, protocols, delete)),
                                  event_mask=X.NoEventMask, propagate=False)
                connection.flush()
            def request_close():
                request_delete(a)
            if args.scenario in ('close', 'both'):
                focus(a)
                request_close()
                wait_for(lambda: state('a').get('close_prompt'), 'WM_DELETE delivered to shared dirty guard')
                screenshot(a, 'close-veto')
                request_close()
                key('s', 'Control_L'); key('q', 'Control_L'); key('x')
                assert all('Lenz foundation ' + k in windows() for k in ('a', 'b'))
                key('Escape')
                wait_for(lambda: not state('a').get('close_prompt'), 'Cancel leaves native window alive')
                for label in ('a', 'b'):
                    after = state(label)['source']
                    assert after == before[label], (label, before[label], after)
                    assert (root / 'app' / (label + '.txt')).read_bytes() == originals[label]
                report['checks'].append('WM_DELETE traverses native veto; repeat/global keys cannot bypass; Cancel preserves both exact drafts/positions/files')
            if args.scenario in ('compose', 'both'):
                focus(a); key('a', 'Control_L')
                wait_for(lambda: state('a')['source']['selection'] == [0, 7], 'capture selected draft before preedit')
                captured = state('a')['source']
                marked_phases = []
                for phase, expected_before, expected_after in [(1, 'draft-a', 'é'), (2, 'é', 'éé')]:
                    captured = state('a')['source']
                    xtest.fake_input(connection, X.KeyPress, compose_code)
                    xtest.fake_input(connection, X.KeyRelease, compose_code)
                    connection.sync()
                    marked = wait_for(lambda: state('a').get('marked'), f'positive native preedit phase {phase}')
                    assert marked['text'], marked
                    assert state('a')['source'] == captured
                    assert exact_text(state('a')) == expected_before
                    screenshot(a, f'native-preedit-{phase}')
                    marked_phases.append(marked)
                    key('e')
                    wait_for(lambda: exact_text(state('a')) == expected_after and not state('a').get('marked'),
                             f'native Unicode commit phase {phase}')
                # No navigation or Undo between compositions: exercise the same ended input context.
                for expected in ['é', 'draft-a']:
                    key('z', 'Control_L')
                    wait_for(lambda: exact_text(state('a')) == expected, f'one transaction Undo to {expected!r}')
                assert (root / 'app/a.txt').read_bytes() == originals['a']
                report['compose'] = {'marked_phases': marked_phases, 'commit': 'éé', 'undos': ['é', 'draft-a'],
                                     'native_cancel': 'not exercised; platform Cancelled can commit a dead-key prefix'}
                report['checks'].append('two consecutive native preedits preserve authoritative draft/history; exact éé commits undo independently; disk untouched')
            if args.default_input:
                focus(a); key('s', 'Control_L')
                wait_for(lambda: (root / 'app/a.txt').read_bytes() == b'draft-a' and not state('a')['source']['dirty'],
                         'ordinary GUI Ctrl-S saves the exact draft')
                key('a', 'Control_L'); key('x')
                wait_for(lambda: exact_text(state('a')) == 'x' and state('a')['source']['dirty'], 'ordinary GUI remains writable after Save')
                report['checks'].append('actual default GUI Ctrl-S saves exact draft; later typing stays dirty until explicit close decision')
            if args.scenario in ('close', 'both'):
                focus(a); request_close()
                wait_for(lambda: state('a').get('close_prompt'), 'fresh close decision')
                key('d')
                wait_for(lambda: 'Lenz foundation a' not in windows(), 'only approved window removed')
                assert 'Lenz foundation b' in windows() and exact_text(state('b')) == 'draft-b'
                report['checks'].append('fresh explicit Discard removes only requested native window after core commit')
            # The other disposable draft survives the asserted close; explicitly discard it for normal cleanup.
            for label, window in [('a', a), ('b', b)]:
                if 'Lenz foundation ' + label in windows():
                    focus(window); request_delete(window)
                    wait_for(lambda: state(label).get('close_prompt'), 'final explicit fixture cleanup ' + label)
                    key('d')
                    wait_for(lambda: 'Lenz foundation ' + label not in windows(), 'approved fixture window closes ' + label)
            wait_for(lambda: app.poll() == 0, 'normal last-window process exit')
            assert (root / 'app/a.txt').read_bytes() == (b'draft-a' if args.default_input else originals['a'])
            assert (root / 'app/b.txt').read_bytes() == originals['b']
            report['normal_exit'] = app.returncode
            report['status'] = 'passed'
            connection.change_keyboard_mapping(compose_code, old_mapping)
            connection.sync()
        except BaseException as error:
            report.update(status='failed', error=repr(error))
            raise
        finally:
            report['actual_seconds'] = time.monotonic() - started
            save()
            if readfd is not None:
                os.close(readfd)
            reap(app)
            if connection is not None:
                connection.close()
            reap(xvfb)


if __name__ == '__main__':
    main()

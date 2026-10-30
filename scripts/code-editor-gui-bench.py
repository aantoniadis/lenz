#!/usr/bin/env python3
"""Production Linux Reader editing on an owned private Xvfb display.

Latency ends at the first XDamage notification after input, not physical display or completed
syntax/marker work. Exact Save/Undo bytes verify each cycle. No user files or display are used.
"""
import argparse
import ctypes as C
import ctypes.util
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from perf_support import read_display_number

# Open only the private server created below; never connect to the user's DISPLAY.
x=C.CDLL(ctypes.util.find_library('X11'))
damage=C.CDLL(ctypes.util.find_library('Xdamage'))
xtest=C.CDLL(ctypes.util.find_library('Xtst'))
def signature(lib,name,restype,*args):
    f=getattr(lib,name); f.restype=restype; f.argtypes=list(args); return f
P=C.c_void_p; U=C.c_ulong; I=C.c_int
class XError(C.Structure):
    _fields_=[('type',I),('display',P),('resourceid',U),('serial',U),('error_code',C.c_ubyte),('request_code',C.c_ubyte),('minor_code',C.c_ubyte)]
errors=[]
ErrorHandler=C.CFUNCTYPE(I,P,C.POINTER(XError))
@ErrorHandler
def record_error(_display,error):
    errors.append((error.contents.error_code,error.contents.request_code,error.contents.minor_code))
    return 0
signature(x,'XSetErrorHandler',P,ErrorHandler)(record_error)
signature(x,'XOpenDisplay',P,C.c_char_p)
signature(x,'XDefaultRootWindow',U,P)
signature(x,'XQueryTree',I,P,U,C.POINTER(U),C.POINTER(U),C.POINTER(C.POINTER(U)),C.POINTER(C.c_uint))
signature(x,'XFetchName',I,P,U,C.POINTER(C.c_char_p))
signature(x,'XFree',I,P)
signature(x,'XSetInputFocus',I,P,U,I,U)
signature(x,'XStringToKeysym',U,C.c_char_p)
signature(x,'XKeysymToKeycode',C.c_ubyte,P,U)
signature(x,'XPending',I,P)
signature(x,'XNextEvent',I,P,P)
signature(x,'XSync',I,P,I)
signature(x,'XFlush',I,P)
signature(x,'XCloseDisplay',I,P)
signature(damage,'XDamageQueryExtension',I,P,C.POINTER(I),C.POINTER(I))
signature(damage,'XDamageCreate',U,P,U,I)
signature(damage,'XDamageSubtract',None,P,U,U,U)
signature(xtest,'XTestFakeKeyEvent',I,P,C.c_uint,I,U)


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--binary', type=Path, required=True)
    cli.add_argument('--source', required=True)
    cli.add_argument('--out', type=Path, required=True)
    cli.add_argument('--rounds', type=int, default=30)
    args = cli.parse_args()
    if args.rounds < 1 or not shutil.which('Xvfb'):
        cli.error('positive rounds and Xvfb required')
    root = args.out.resolve()
    root.mkdir(parents=True, exist_ok=False)
    binary = args.binary.resolve(strict=True)
    original = ''.join(f'READER_LINE_{i:06d} α theme\r\n' for i in range(100000)).encode()
    fixture = root / 'large.txt'
    fixture.write_bytes(original)
    env = dict(os.environ)
    for key in ('WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'XDG_SESSION_TYPE'):
        env.pop(key, None)
    for kind in ('config', 'state', 'cache', 'data'):
        env[f'XDG_{kind.upper()}_HOME'] = str(root / kind)
    env.update(XDG_RUNTIME_DIR=str(root / 'runtime'), MESA_SHADER_CACHE_DIR=str(root / 'shaders'))
    Path(env['XDG_RUNTIME_DIR']).mkdir(mode=0o700)
    server = proc = display = None
    result = dict(schema=1, source=args.source, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                  collector_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                  original_sha256=hashlib.sha256(original).hexdigest(), fixture_bytes=len(original), rounds=args.rounds,
                  latency='first XDamage after synthetic input; no physical-display or completed-analysis claim',
                  save_latency='Ctrl-S to exact observed file bytes, including 2ms polling and the byte oracle; no filesystem durability claim',
                  status='failed', samples={})
    try:
        readfd, writefd = os.pipe()
        try:
            server = subprocess.Popen([shutil.which('Xvfb'), '-displayfd', str(writefd), '-screen', '0', '1600x1000x24', '-nolisten', 'tcp'],
                                      pass_fds=(writefd,), env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        finally:
            os.close(writefd)
        try:
            env['DISPLAY'] = ':' + read_display_number(readfd)
        finally:
            os.close(readfd)
        display = x.XOpenDisplay(env['DISPLAY'].encode())
        assert display
        with (root / 'app.log').open('w') as log:
            proc = subprocess.Popen([str(binary), str(fixture)], env=env, cwd=root, stdout=log, stderr=log)
            def live():
                if proc.poll() is not None:
                    raise RuntimeError(f'Reader exited: {proc.returncode}')
            def until(done, label, seconds=15):
                deadline = time.monotonic() + seconds
                while not done():
                    live()
                    if time.monotonic() > deadline:
                        raise RuntimeError('timeout: ' + label)
                    time.sleep(.002)
            def window():
                rr=U(); parent=U(); children=C.POINTER(U)(); count=C.c_uint()
                x.XQueryTree(display,x.XDefaultRootWindow(display),C.byref(rr),C.byref(parent),C.byref(children),C.byref(count))
                found = None
                for i in range(count.value):
                    name=C.c_char_p()
                    if x.XFetchName(display,children[i],C.byref(name)) and name.value:
                        if name.value: found=children[i]
                        x.XFree(name)
                    if found: break
                if children: x.XFree(children)
                return found
            until(window, 'owned Reader window')
            win = window()
            # Settle initial loading before the exact-input oracle establishes a ready editable document.
            time.sleep(1)
            base=I(); error=I()
            assert damage.XDamageQueryExtension(display,C.byref(base),C.byref(error))
            handle=damage.XDamageCreate(display,win,3)
            x.XSetInputFocus(display,win,1,0); x.XSync(display,0)
            event=(C.c_long*24)()
            def key(name, modifiers=()):
                codes=[x.XKeysymToKeycode(display,x.XStringToKeysym(k.encode())) for k in (*modifiers,name)]
                assert all(codes), (name,modifiers)
                for code in codes: xtest.XTestFakeKeyEvent(display,code,1,0)
                for code in reversed(codes): xtest.XTestFakeKeyEvent(display,code,0,0)
                x.XFlush(display)
            def drain():
                while x.XPending(display): x.XNextEvent(display,C.byref(event))
                damage.XDamageSubtract(display,handle,0,0); x.XSync(display,0)
                while x.XPending(display): x.XNextEvent(display,C.byref(event))
                if errors: raise RuntimeError(f'private X server error: {errors}')
            def feedback(name, modifiers=()):
                time.sleep(.08)
                drain()
                started=time.monotonic(); key(name,modifiers)
                deadline=started+5
                while time.monotonic()<deadline:
                    while x.XPending(display):
                        x.XNextEvent(display,C.byref(event))
                        if C.cast(C.byref(event),C.POINTER(I))[0]==base.value:
                            return (time.monotonic()-started)*1000
                    live(); time.sleep(.001)
                raise RuntimeError('no paint feedback for '+name)
            def save(expected):
                started=time.monotonic(); key('s',('Control_L',))
                until(lambda: fixture.read_bytes()==expected, 'exact Save')
                return (time.monotonic()-started)*1000
            samples = dict(edit_ms=[], undo_ms=[], redo_ms=[], save_ms=[], rss_mib=[])
            def memory():
                raw=Path(f'/proc/{proc.pid}/status').read_text()
                value=lambda name: next(int(line.split()[1])/1024 for line in raw.splitlines() if line.startswith(name+':'))
                return value('VmRSS'),value('VmHWM')
            peak=0
            for trial in range(args.rounds+3):
                key('g',('Control_L',)); time.sleep(.04); key('a',('Control_L',)); key('1'); key('Return'); time.sleep(.08)
                edit=feedback('x')
                save_ms=save(b'x'+original)
                undo=feedback('z',('Control_L',)); save(original)
                redo=feedback('z',('Control_L','Shift_L')); save(b'x'+original)
                feedback('z',('Control_L',)); save(original)
                rss,hwm=memory(); peak=max(peak,hwm)
                if trial>=3:
                    for name,value in [('edit_ms',edit),('undo_ms',undo),('redo_ms',redo),('save_ms',save_ms),('rss_mib',rss)]: samples[name].append(value)
            time.sleep(1)
            settled,hwm=memory(); peak=max(peak,hwm)
            result.update(status='passed', samples=samples, peak_rss_mib=peak, settled_rss_mib=settled, final_sha256=hashlib.sha256(fixture.read_bytes()).hexdigest(),
                          checks=['each edit/Undo/Redo saved exact expected CRLF and Unicode bytes', 'final original bytes restored'])
    except BaseException as error:
        result['error']=repr(error)
        raise
    finally:
        for child in (proc,):
            if child:
                if child.poll() is None: child.terminate()
                try: child.wait(timeout=5)
                except subprocess.TimeoutExpired: child.kill(); child.wait()
        if display: x.XCloseDisplay(display)
        if server:
            if server.poll() is None: server.terminate()
            try: server.wait(timeout=5)
            except subprocess.TimeoutExpired: server.kill(); server.wait()
        (root/'result.json').write_text(json.dumps(result,indent=2)+'\n')
        print(json.dumps({k:result[k] for k in ('status','error','peak_rss_mib') if k in result}))


if __name__ == '__main__':
    main()

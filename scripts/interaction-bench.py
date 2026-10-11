#!/usr/bin/env python3
"""Linux interaction/RSS probe using only a private Xvfb server.

Latency ends at the first XDamage notification after synthetic input: it includes
software presentation, but not physical display latency or asynchronous file readiness.
Only use disposable fixtures: the scenarios change selections and in-memory merge choices.
"""
import ctypes as C
import ctypes.util
import argparse, json, os, re, selectors, shutil, statistics, subprocess, tempfile, time
from pathlib import Path
from perf_support import prepare_gui_launch, gui_about_visible, GUI_ABOUT_ORACLE_VERSION, isolate_user_state

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

isolate_user_state()  # never measure with the user's own settings or saved positions
cli=argparse.ArgumentParser(description=__doc__)
cli.add_argument('--binary',default='target/release/lenz')
cli.add_argument('--xvfb',default=shutil.which('Xvfb'))
cli.add_argument('--scenario',choices=['diff','merge','files','history','images','about'],required=True)
cli.add_argument('--files',type=int,default=12,help='Number of files in the files scenario')
cli.add_argument('--rounds',type=int,default=3,help='Forward/backward passes for the files scenario')
cli.add_argument('--file-pause',type=float,default=.2,help='Seconds after initial file selections; use 1 for large images')
cli.add_argument('--max-growth-mib',type=float,help='Fail if settled RSS exceeds initial idle RSS by this amount')
cli.add_argument('--cache-dir',help='Optional reusable application cache')
cli.add_argument('--screenshot',help='Save the settled window; requires ImageMagick import')
cli.add_argument('--check',action='store_true',help='Fail if any measured key gets no repaint within 3 seconds')
cli.add_argument('args',nargs=argparse.REMAINDER,help='Arguments after --; four-file merges use disposable copies outside Git')
opt=cli.parse_args()
if not opt.xvfb or opt.files<2 or opt.rounds<1 or not 0<=opt.file_pause<=30:
    cli.error('requires Xvfb, --files >= 2 and --rounds >= 1')
if opt.screenshot and not shutil.which('import'):
    cli.error('--screenshot requires ImageMagick import')
if opt.scenario == 'about' and (not shutil.which('import') or not shutil.which('tesseract')):
    cli.error('About content verification requires ImageMagick import and tesseract')
binary=Path(opt.binary).resolve(); case=opt.scenario
args=opt.args[1:] if opt.args[:1]==['--'] else opt.args
scratch=tempfile.TemporaryDirectory(prefix='lenz-interaction-')
root=Path(scratch.name)
args,launch_cwd=prepare_gui_launch(args,root/'merge-inputs')
if case=='merge' and launch_cwd is None:
    cli.error('merge scenario requires four disposable input files: merge base ours theirs merged')
env=dict(os.environ,GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1',LENZ_TIMINGS='1',XDG_RUNTIME_DIR=str(root),XDG_CACHE_HOME=opt.cache_dir or str(root/'cache'))
env.pop('WAYLAND_DISPLAY',None)
r,w=os.pipe()
server=subprocess.Popen([opt.xvfb,'-displayfd',str(w),'-screen','0','1600x1000x24','-nolisten','tcp','-noreset'],env=env,pass_fds=(w,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
os.close(w)
proc=None; display=None
try:
    with selectors.DefaultSelector() as s:
        s.register(r,selectors.EVENT_READ)
        if not s.select(10): raise RuntimeError('private display failed')
    number=os.read(r,100).decode().strip(); os.close(r)
    if not number.isdigit(): raise RuntimeError('private display returned no display number')
    env['DISPLAY']=':'+number
    display=x.XOpenDisplay(env['DISPLAY'].encode()); assert display
    log=(root/'lenz.log').open('w')
    proc=subprocess.Popen([str(binary),*args],env=env,cwd=launch_cwd,stdout=log,stderr=log)
    start=time.monotonic(); win=None
    while time.monotonic()-start<15:
        rr=U(); parent=U(); children=C.POINTER(U)(); count=C.c_uint()
        x.XQueryTree(display,x.XDefaultRootWindow(display),C.byref(rr),C.byref(parent),C.byref(children),C.byref(count))
        for idx in range(count.value):
            name=C.c_char_p()
            if x.XFetchName(display,children[idx],C.byref(name)) and name.value:
                title=name.value.decode(errors='replace'); x.XFree(name)
                if title:
                    win=children[idx]; break
        if children: x.XFree(children)
        if win: break
        if proc.poll() is not None: raise RuntimeError('Lenz exited')
        time.sleep(.01)
    assert win,'window missing'
    while time.monotonic()-start<15:
        if 'lenz: first paint' in Path(log.name).read_text(): break
        if proc.poll() is not None: raise RuntimeError('Lenz exited before paint')
        time.sleep(.01)
    else: raise RuntimeError('first paint timeout')
    startup=re.search(r'lenz: first paint (\d+) ms',Path(log.name).read_text())
    print(json.dumps({'phase':'startup','first_paint_ms':int(startup[1])}),flush=True)
    base=I(); error=I(); assert damage.XDamageQueryExtension(display,C.byref(base),C.byref(error))
    handle=damage.XDamageCreate(display,win,3)
    x.XSetInputFocus(display,win,1,0); x.XSync(display,0)
    ev=(C.c_long*24)()
    def drain():
        while x.XPending(display): x.XNextEvent(display,C.byref(ev))
        damage.XDamageSubtract(display,handle,0,0); x.XSync(display,0)
        while x.XPending(display): x.XNextEvent(display,C.byref(ev))
        if errors: raise RuntimeError(f'private X server error: {errors}')
    high_water=[0.0]
    def memory():
        status=Path(f'/proc/{proc.pid}/status').read_text()
        stat=Path(f'/proc/{proc.pid}/stat').read_text().rsplit(')',1)[1].split()
        rss=int(re.search(r'VmRSS:\s+(\d+)',status)[1])/1024
        high_water[0]=max(high_water[0],rss,int(re.search(r'VmHWM:\s+(\d+)',status)[1])/1024)
        return {'rss_mib':round(rss,1),
                'peak_mib':round(high_water[0],1),
                'cpu_s':(int(stat[11])+int(stat[12]))/os.sysconf('SC_CLK_TCK')}
    def key(name,mods=()):
        codes=[x.XKeysymToKeycode(display,x.XStringToKeysym(k.encode())) for k in [*mods,name]]
        assert all(codes),(name,mods)
        for code in codes: xtest.XTestFakeKeyEvent(display,code,1,0)
        for code in reversed(codes): xtest.XTestFakeKeyEvent(display,code,0,0)
        x.XFlush(display)
    def action(name,mods=(),pause=.1):
        drain(); t=time.monotonic(); key(name,mods); latency=None
        while time.monotonic()-t<3:
            while x.XPending(display):
                x.XNextEvent(display,C.byref(ev))
                if C.cast(C.byref(ev),C.POINTER(I))[0]==base.value:
                    latency=(time.monotonic()-t)*1000; break
            if latency is not None: break
            if proc.poll() is not None: raise RuntimeError('Lenz exited during action')
            time.sleep(.001)
        if pause: time.sleep(pause)
        return latency
    rows=[]
    def series(label,name,n=20,mods=(),pause=.08):
        samples=[action(name,mods,pause) for _ in range(n)]
        good=sorted(v for v in samples if v is not None)
        row={'phase':label,'first_damage_samples_ms':[round(v,2) if v is not None else None for v in samples],
             'median_ms':round(statistics.median(good),2) if good else None,
             'p95_ms':round(good[min(len(good)-1,int(len(good)*.95))],2) if good else None,**memory()}
        rows.append(row); print(json.dumps(row),flush=True)
    time.sleep(4)
    a=memory(); time.sleep(2); b=memory()
    print(json.dumps({'phase':'idle','cpu_seconds_per_second':round((b['cpu_s']-a['cpu_s'])/2,3),**b}),flush=True)
    if case=='about':
        def about_menu():
            # F10 opens the command palette; typing filters it to About Lenz.
            for name in ('F10','a','b','o','u','t'):
                action(name)
        def verify_about():
            deadline=time.monotonic()+3
            screenshot=root/'about.png'; visible=''
            while time.monotonic()<deadline:
                subprocess.run(['import','-display',env['DISPLAY'],'-window',str(win),str(screenshot)],env=env,check=True,timeout=5)
                visible=subprocess.check_output(['tesseract',str(screenshot),'stdout','--psm','11'],stderr=subprocess.DEVNULL,text=True,timeout=5)
                if gui_about_visible(visible): return
            raise RuntimeError(f'About dialog content was not visible (oracle {GUI_ABOUT_ORACLE_VERSION}): {visible}')
        for _ in range(20):
            about_menu()
            series(f'about-open-v{GUI_ABOUT_ORACLE_VERSION}','Return',1,pause=.08)
            verify_about()  # Separate from first-XDamage feedback timing.
            series(f'about-close-v{GUI_ABOUT_ORACLE_VERSION}','Escape',1,pause=.08)
        print(json.dumps({'phase':'about-validation','oracle_version':GUI_ABOUT_ORACLE_VERSION,'dialog_content_confirmed':True,'cycles':20}),flush=True)
        if opt.screenshot:
            about_menu(); action('Return'); verify_about()
    elif case=='diff':
        series('page-scroll','Next')
        series('collapse','z',6)
        # a full cycle of the four Scroll Modes, so find below runs in Shared as before
        series('scroll-mode','s',4,pause=.2)
        series('whitespace','w',4,pause=.3)
        series('highlight','h',3,pause=.3)
        action('Home')
        action('f',('Control_L',))
        series('find-e','e',1,pause=.3)
        series('find-scroll','Next',20)
        series('find-next','F3',10)
        action('Escape')
    elif case=='merge':
        # Start on the initial unresolved conflict. Undo each choice before the next.
        merged=Path(args[-1])
        def saved_result(expected=None, different=None):
            key('s',('Control_L',))
            deadline=time.monotonic()+3
            while time.monotonic()<deadline:
                value=merged.read_bytes() if merged.exists() else b''
                if value and (expected is None or value==expected) and (different is None or value!=different):
                    return value
                if proc.poll() is not None: raise RuntimeError('Lenz exited during semantic merge check')
                time.sleep(.01)
            raise RuntimeError('merge save/undo did not produce the expected file bytes')
        before=saved_result(different=merged.read_bytes() if merged.exists() else b'')
        if b'<<<<<<<' not in before or b'>>>>>>>' not in before:
            raise RuntimeError('merge benchmark did not start on unresolved conflict markers')
        for label,keyname in [('accept-ours','Right'),('accept-theirs','Left'),('accept-both','Down'),('keep-base','BackSpace')]:
            series(label,keyname,1,('Alt_L','Shift_L'),pause=.7)
            saved_result(different=before)
            series('undo-'+label,'z',1,('Control_L',),pause=.7)
            saved_result(expected=before)
        print(json.dumps({'phase':'merge-validation','unresolved_start':True,'accepted_choices':4,'exact_undo':True,'original_inputs_untouched':True}),flush=True)
        series('page-scroll','Next')
        series('next-change','n')
        series('base-toggle','b',6,pause=.7)
    elif case=='files':
        series('load-file','bracketright',opt.files-1,pause=opt.file_pause)
        series('revisit-file','bracketleft',opt.files-1,pause=opt.file_pause)
        for i in range(opt.rounds):
            series(f'forward-{i}','bracketright',opt.files-1,pause=.03)
            series(f'backward-{i}','bracketleft',opt.files-1,pause=.03)
    elif case=='history':
        series('log-selection','Down')
        series('log-up','Up')
    elif case=='images':
        for mode in ('swipe','onion'):
            series(mode,'m',1,pause=.3)
        series('opacity-up','period',4,pause=.3)
        series('opacity-down','comma',4,pause=.3)
        for mode in ('difference','two-up'):
            series(mode,'m',1,pause=.3)
        series('zoom-in','equal',6,pause=.2)
        series('fit','0',1,pause=.2)
    time.sleep(4)
    settled=memory()
    growth=round(settled['rss_mib']-b['rss_mib'],1)
    print(json.dumps({'phase':'settled','rss_growth_mib':growth,**settled}),flush=True)
    if opt.screenshot:
        subprocess.run(['import','-display',env['DISPLAY'],'-window',str(win),opt.screenshot],env=env,check=True)
    if opt.check and any(None in row['first_damage_samples_ms'] for row in rows):
        raise RuntimeError('an interaction failed to repaint within 3 seconds')
    if opt.max_growth_mib is not None and growth>opt.max_growth_mib:
        raise RuntimeError(f'RSS grew by {growth} MiB, exceeding {opt.max_growth_mib} MiB')
finally:
    if proc is not None:
        proc.terminate()
        try: proc.wait(timeout=3)
        except subprocess.TimeoutExpired: proc.kill(); proc.wait()
    if display: x.XCloseDisplay(display)
    server.terminate()
    try: server.wait(timeout=3)
    except subprocess.TimeoutExpired: server.kill(); server.wait()
    scratch.cleanup()

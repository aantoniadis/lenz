#!/usr/bin/env python3
"""Version 1 Markdown probes; owned PTYs/private Xvfb, no desktop input.

Terminal completion requires a reconstructed current screen, completed Rendered
status and a tail reached through real End navigation. Native GUI feedback is
first XDamage only; release GUI adapter tests supply the completion boundary.
"""
import argparse
from contextlib import suppress
import math
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import time

from perf_support import process_stats, isolate_user_state, read_display_number


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


workflow = load('markdown_workflow', 'tui-workflow-bench.py')
Screen = workflow.Screen
TerminalProbe = workflow.TerminalProbe


def completed_render(text: str, ready: bool) -> bool:
    return ready and 'MARKDOWN_TAIL_END' in text


def terminal_completed(text):
    status = (part.strip().removeprefix('F2 details · ')
              for line in text.splitlines() for part in line.split('│'))
    return completed_render(text, any(part == 'Rendered Markdown' or part.startswith('Rendered Markdown · ')
                                    for part in status)
                            and 'Preparing rendered Markdown' not in text)


def gui_rendered_tail(text):
    """Formatted fixture prose is the oracle; Source also contains the tail label.

    This is an OCR observation after settling, not a production completion timer.
    """
    return ('MARKDOWN' in text and 'TAIL' in text
            and re.search(r'Paragraph\s+\d+\s+with\s+strong,\s+emphasis,', text) is not None)


def file_identity(path):
    path = Path(path)
    return {'sha256': hashlib.sha256(path.read_bytes()).hexdigest(), 'bytes': path.stat().st_size}


def generate_fixtures(root):
    """Reproduce version-one inputs in an empty, caller-owned directory."""
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    if any(root.iterdir()):
        raise ValueError('fixture directory must be empty')
    for side, marker in [('left', 'LEFT'), ('right', 'RIGHT')]:
        (root / f'source-{side}.txt').write_text(f'{marker}_SOURCE_MARKER\n' + 'common line\n' * 100000)
    for name, count in [('small', 1000), ('medium', 2000), ('large', 30000)]:
        left, right = [], []
        for i in range(count):
            heading = f'## Section {i}\n\n'
            text = f'Paragraph {i} with **strong**, *emphasis*, `code`, [guide][g] and 漢字 é.\n\n'
            more = '- [x] task\n- list item\n\n> Quote\n\n| Name | Value |\n| --- | --- |\n| entry | value |\n\n```rust\nlet value = 42;\n```\n\n' if i % 100 == 0 else ''
            left.append(heading + text + more)
            right.append(heading + text.replace(f'Paragraph {i}', f'Changed {i}' if i % 31 == 0 else f'Paragraph {i}') + more)
        for side, parts, target in [('left', left, 'old'), ('right', right, 'new')]:
            parts.append(f'[g]: https://example.invalid/{target}\n\n# MARKDOWN_TAIL_END\n')
            (root / f'{name}-{side}.md').write_text(''.join(parts))
    return {'root': str(root.resolve()), 'workload_version': 1,
            'files': {p.name: file_identity(p) for p in sorted(root.iterdir())}}


def verify_fixtures(manifest):
    if not manifest.get('files'):
        raise ValueError('missing fixture identities')
    for name, expected in manifest['files'].items():
        if file_identity(Path(manifest['root']) / name) != expected:
            raise ValueError(f'changed fixture: {name}')


def validate_identity(identity):
    binary, source = identity.get('binary', {}), identity.get('source', {})
    if (len(binary.get('sha256', '')) != 64 or not binary.get('bytes') or
            not source.get('commit') or not source.get('source')):
        raise ValueError('missing binary/source identity')


def validate_stress(data, required):
    timeline = data.get('timeline', [])
    if (data.get('duration_seconds', 0) < required or data.get('cycles', 0) < 1 or
            len(timeline) < 2 or timeline[0] != 0 or timeline[-1] < required or
            not all(math.isfinite(t) for t in timeline) or
            any(a >= b for a,b in zip(timeline,timeline[1:])) or
            timeline[-1] > data['duration_seconds']):
        raise ValueError('incomplete stress duration/cycles/timeline')


def source_identity():
    files = subprocess.check_output(['git', 'ls-files', 'crates', 'Cargo.toml', 'Cargo.lock', 'scripts'], text=True).splitlines()
    return {'commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
            'dirty_diff': subprocess.check_output(['git', 'diff', 'HEAD'], text=True),
            'source': {p: file_identity(p)['sha256'] for p in files if Path(p).is_file()},
            'rust': subprocess.check_output(['rustc', '-vV'], text=True),
            'platform': platform.platform(), 'cpu': Path('/proc/cpuinfo').read_text(),
            'environment': {k: os.environ.get(k) for k in ('RUSTFLAGS', 'CARGO_PROFILE_RELEASE_DEBUG', 'LENZ_THEME')}}


class MarkdownTerminal(workflow.Workflow):
    def __init__(self, binary, root, args, output):
        super().__init__(str(Path(binary).resolve()), root, args, 'markdown-v1')
        self.output_dir = output
        self.timeline = []
        self.started = time.monotonic()

    def snapshot(self, phase):
        sample = {'phase': phase, 'elapsed': time.monotonic()-self.started, **self.probe.sample()}
        self.timeline.append(sample)
        return sample

    def completed(self, timeout=30):
        start = time.monotonic()
        while True:
            self.screen.update(self.probe)
            if terminal_completed(self.screen.text()):
                return (time.monotonic()-start)*1000
            if time.monotonic()-start > timeout or self.probe.proc.poll() is not None:
                raise RuntimeError('no completed rendered tail:\n'+self.screen.text())
            self.probe.read(.002)
            self.probe.sample()

    def rendered(self):
        self.key(b':', 'Commands:')
        # Source | Inline | Side by side: the Preview is entered in a layout
        label = getattr(self, 'preview_label', 'Inline').encode()
        self.key(label, label.decode())
        self.probe.quiet(.03)
        started = time.monotonic()
        os.write(self.probe.master, b'\r')
        if not self.probe.read(5):
            raise RuntimeError('no Rendered feedback')
        feedback = (time.monotonic()-started)*1000
        self.wait('Rendered Markdown', absent=('Preparing rendered Markdown',), timeout=30)
        os.write(self.probe.master, b'\x1b[F')
        self.completed()
        return {'request_to_completed_tail_ms': (time.monotonic()-started)*1000,
                'first_byte_ms': feedback, 'ready_screen': self.screen.text()}

    def source(self):
        self.menu('View: Source')  # the palette's Markdown view row
        self.wait(absent=('Rendered Markdown', 'Preparing rendered Markdown'))

    def resize_checked(self, width, height):
        self.probe.quiet(.03)
        # Retain old cells until the actual full repaint arrives; reset dimensions.
        self.screen.width, self.screen.height = width, height
        self.screen.cells = [[' ']*width for _ in range(height)]
        self.screen.x = self.screen.y = 0
        start = time.monotonic()
        self.probe.resize(width, height)
        os.write(self.probe.master, b'\x1b[F')  # Wake PTY event loop after SIGWINCH.
        self.wait('Rendered Markdown', absent=('Preparing rendered Markdown',))
        os.write(self.probe.master, b'\x1b[F')
        self.completed()
        return (time.monotonic()-start)*1000

    def close(self):
        self.output_dir.mkdir(parents=True, exist_ok=True)
        (self.output_dir/'terminal.ansi').write_bytes(self.probe.output)
        (self.output_dir/'screen.txt').write_text(self.screen.text())
        (self.output_dir/'timeline.json').write_text(json.dumps(self.timeline, indent=2))
        self.probe.close()


def stress_repository(manifest, output):
    root = Path(tempfile.mkdtemp(prefix='lenz-markdown-stress-'))
    env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1',
               GIT_AUTHOR_NAME='Markdown Bench', GIT_AUTHOR_EMAIL='markdown@example.invalid',
               GIT_COMMITTER_NAME='Markdown Bench', GIT_COMMITTER_EMAIL='markdown@example.invalid',
               GIT_AUTHOR_DATE='2026-10-05T00:00:00Z', GIT_COMMITTER_DATE='2026-10-05T00:00:00Z')
    def git(*args):
        return subprocess.check_output(['git', '-C', str(root), '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', *args], env=env, text=True).strip()
    git('init', '-q')
    for side in ['left', 'right']:
        for name, size in [('a.md', 'small'), ('b.md', 'medium')]:
            shutil.copyfile(Path(manifest['root'])/f'{size}-{side}.md', root/name)
        git('add', '.'); git('commit', '-qm', 'Markdown '+side)
    identity = {'root': str(root), 'head': git('rev-parse', 'HEAD'),
                'files': {name: file_identity(root/name) for name in ['a.md','b.md']}}
    (output/'stress-fixtures.json').write_text(json.dumps(identity, indent=2))
    return root, identity


def tui_case(opt, manifest):
    root = Path(manifest['root'])
    args = ['--no-index', f'{opt.fixture}-left.md', f'{opt.fixture}-right.md']
    stress_identity = None
    if opt.stress_seconds:
        root, stress_identity = stress_repository(manifest, opt.output); args = ['HEAD^', 'HEAD']
    run = MarkdownTerminal(opt.binary, root, args, opt.output)
    run.preview_label = 'Side by side' if opt.markdown_layout == 'side-by-side' else 'Inline'
    result = {'workload_version': 1, 'fixture': opt.fixture, 'theme': os.environ.get('LENZ_THEME', 'ambient default (not pinned)')}
    try:
        run.wait('Section 0'); run.probe.quiet(.2)
        result['source'] = run.snapshot('source')
        if opt.fixture == 'large':
            run.menu('Rendered')
            run.wait('showing Source', absent=('Preparing rendered Markdown',), timeout=60)
            run.probe.quiet(.5); run.screen.update(run.probe)
            visible=run.screen.text()
            if any(line.strip() == 'Rendered Markdown' for line in visible.splitlines()) or 'exceeds parser events' not in visible:
                raise RuntimeError('large fixture did not visibly fall back to Source: '+visible)
            result['fallback_screen'] = visible
        else:
            result['rendered'] = [run.rendered()]
            result['rendered_rss'] = run.snapshot('rendered')
            result['resize_and_end_completion_ms'] = [run.resize_checked(w,h) for w,h in [(60,20),(180,50),(120,36)]]
            result['scroll_first_byte_ms'] = [run.probe.input_latency(k) for k in [b'\x1b[H',b'\x1b[6~',b'\x1b[F',b'\x1b[A']]
            run.key(b'\x01'); run.key(b'y', 'Copied (terminal OSC 52 clipboard)')
            if b'\x1b]52;' not in run.probe.output:
                raise RuntimeError('copy did not emit OSC52 clipboard transport')
            result['copy_osc52'] = True
            run.key(b'\x1b')  # Clear copy selection so End means viewport navigation.
            if opt.find:
                run.key(b'/', 'Find:'); start=time.monotonic();os.write(run.probe.master,b'Paragraph\r')
                run.wait('Find: Paragraph', absent=('Preparing rendered Markdown',))
                result['find_query_feedback_ms']=(time.monotonic()-start)*1000
                result['find_active_scroll_first_byte_ms']=[run.probe.input_latency(k) for k in [b'\x1b[6~',b'\x1b[F',b'\x1b[A',b'\x1b[13~']*6]
                run.key(b'\x1b[F');run.completed();result['find_screen']=run.screen.text();run.key(b'\x1b')
            result['navigation_first_byte_ms'] = [run.probe.input_latency(k) for k in [b'n', b'p']]
            run.key(b'\x1b[F'); run.completed()
            if opt.stress_seconds:
                start = time.monotonic(); cycles = 0; timeline = [0]
                while time.monotonic()-start < opt.stress_seconds:
                    run.source()
                    run.key(b']' if cycles%2 == 0 else b'[', 'Lenz  2/2' if cycles%2 == 0 else 'Lenz  1/2')
                    result['rendered'].append(run.rendered()); cycles += 1
                    run.resize_checked(100 if cycles%2 else 120, 36)
                    run.key(b'\x1b[H'); run.key(b'\x1b[F'); run.completed()
                    run.source(); run.key(b'\x1b[H'); run.key(b'b', '2026-10-05')
                    run.key(b'B', 'Markdown Bench <markdown@example.invalid>')
                    run.key(b'd', 'view 2/2'); result['rendered'].append(run.rendered())
                    run.key(b'q', absent=('view 2/2', 'view 1/2')); run.close_overlay(); run.key(b'b')
                    run.snapshot('stress-view-closed'); timeline.append(time.monotonic()-start)
                result['stress'] = {'duration_seconds': time.monotonic()-start, 'cycles': cycles, 'timeline': timeline}
                validate_stress(result['stress'], opt.stress_seconds)
            run.source(); run.probe.quiet(.5)
            result['after_source'] = run.snapshot('after-source')
        if stress_identity: verify_fixtures(stress_identity)
        result['peak_rss_mib'] = run.probe.peak
        run.probe.finish()
        return result
    finally:
        run.close()


def tui_startup(opt, manifest):
    root = Path(manifest['root']); results=[]
    builds=[('baseline',opt.baseline),('candidate',opt.binary)]
    if opt.reverse_order:builds.reverse()
    for iteration in range(opt.runs+1):
        for label,binary in (builds if iteration%2 else builds[::-1]):
            p=TerminalProbe([str(Path(binary).resolve()),'--no-index','source-left.txt','source-right.txt'],root)
            try:
                paint=p.expect(b'details'); first_stat=Path(f'/proc/{p.proc.pid}/stat').read_text().rsplit(')',1)[1].split()
                content=p.expect(b'RIGHT_SOURCE_MARKER'); p.quiet(.5)
                sample={'build':label,'run':iteration,'warmup':iteration==0,'first_paint_ms':paint,'first_content_ms':content,'first_paint_minor_faults':int(first_stat[7]),'first_paint_major_faults':int(first_stat[9]),**p.sample()}
                results.append(sample); p.finish()
            finally:p.close()
    return results


class NativeGui:
    """Only opens the freshly allocated private display; owns all child processes."""
    def __init__(self, binary, args, cwd, output, xvfb):
        self.output=output; self.timeline=[]
        try:
            self.start(binary,args,cwd,output,xvfb)
        except BaseException:
            self.close()
            raise

    def start(self, binary, args, cwd, output, xvfb):
        import ctypes as C
        import ctypes.util
        self.C=C; self.output=output; output.mkdir(parents=True,exist_ok=True)
        self.scratch=tempfile.TemporaryDirectory(prefix='lenz-markdown-display-')
        self.env=dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1', LENZ_TIMINGS='1',
                      XDG_RUNTIME_DIR=self.scratch.name, XDG_CONFIG_HOME=self.scratch.name+'/config',
                      XDG_CACHE_HOME=self.scratch.name+'/cache', XDG_STATE_HOME=self.scratch.name+'/state')
        self.env.pop('WAYLAND_DISPLAY',None)
        self.x=C.CDLL(ctypes.util.find_library('X11')); self.d=C.CDLL(ctypes.util.find_library('Xdamage'));self.t=C.CDLL(ctypes.util.find_library('Xtst'))
        P=C.c_void_p; U=C.c_ulong; I=C.c_int
        def sig(lib,name,result,*args):
            fn=getattr(lib,name);fn.restype=result;fn.argtypes=list(args)
        for name,result,signature_args in [('XOpenDisplay',P,[C.c_char_p]),('XDefaultRootWindow',U,[P]),('XQueryTree',I,[P,U,C.POINTER(U),C.POINTER(U),C.POINTER(C.POINTER(U)),C.POINTER(C.c_uint)]),('XFetchName',I,[P,U,C.POINTER(C.c_char_p)]),('XFree',I,[P]),('XSetInputFocus',I,[P,U,I,U]),('XStringToKeysym',U,[C.c_char_p]),('XKeysymToKeycode',C.c_ubyte,[P,U]),('XPending',I,[P]),('XNextEvent',I,[P,P]),('XSync',I,[P,I]),('XFlush',I,[P]),('XCloseDisplay',I,[P]),('XResizeWindow',I,[P,U,C.c_uint,C.c_uint])]:sig(self.x,name,result,*signature_args)
        sig(self.d,'XDamageQueryExtension',I,P,C.POINTER(I),C.POINTER(I));sig(self.d,'XDamageCreate',U,P,U,I);sig(self.d,'XDamageSubtract',None,P,U,U,U)
        sig(self.t,'XTestFakeKeyEvent',I,P,C.c_uint,I,U)
        sig(self.t,'XTestFakeMotionEvent',I,P,I,I,I,U);sig(self.t,'XTestFakeButtonEvent',I,P,C.c_uint,I,U)
        r,w=os.pipe(); self.server_log=(output/'xvfb.log').open('w')
        self.server=subprocess.Popen([str(xvfb),'-displayfd',str(w),'-screen','0','1600x1000x24','-nolisten','tcp','-noreset'],pass_fds=(w,),stdout=self.server_log,stderr=self.server_log,env=self.env)
        os.close(w)
        try:number=read_display_number(r)
        finally:os.close(r)
        self.env['DISPLAY']=':'+number;self.display=self.x.XOpenDisplay(self.env['DISPLAY'].encode())
        if not self.display:raise RuntimeError('cannot open private display')
        self.log=(output/'application.log').open('w')
        self.proc=subprocess.Popen([str(Path(binary).resolve()),*args],cwd=cwd,env=self.env,stdout=self.log,stderr=self.log,start_new_session=True)
        started=time.monotonic();self.win=None
        while time.monotonic()-started<30:
            windows=self.windows()
            if windows:self.win=windows[0]
            if self.win and 'lenz: first paint' in (output/'application.log').read_text():break
            if self.proc.poll() is not None:raise RuntimeError('GUI exited before first paint')
            time.sleep(.01)
        if not self.win:raise RuntimeError('no GUI window')
        self.base=I();err=I();assert self.d.XDamageQueryExtension(self.display,C.byref(self.base),C.byref(err))
        self.damage=self.d.XDamageCreate(self.display,self.win,3);self.event=(C.c_long*24)()
        self.focus(self.win);self.started=time.monotonic();self.timeline=[]
        self.pause(1)

    def windows(self):
        C=self.C;U=C.c_ulong;root=U();parent=U();children=C.POINTER(U)();count=C.c_uint();result=[]
        self.x.XQueryTree(self.display,self.x.XDefaultRootWindow(self.display),C.byref(root),C.byref(parent),C.byref(children),C.byref(count))
        for i in range(count.value):
            name=C.c_char_p()
            if self.x.XFetchName(self.display,children[i],C.byref(name)) and name.value:
                title=name.value.decode(errors='replace');self.x.XFree(name)
                if title:result.append(children[i])
        if children:self.x.XFree(children)
        return result

    def focus(self,window):
        self.x.XSetInputFocus(self.display,window,1,0);self.x.XSync(self.display,0)

    def sample(self,phase):
        sample={'phase':phase,'elapsed':time.monotonic()-self.started,**process_stats(self.proc.pid)}
        self.timeline.append(sample);return sample

    def pause(self,seconds):
        start=time.monotonic()
        while time.monotonic()-start<seconds:
            self.sample('wait');time.sleep(.02)

    def drain(self):
        while self.x.XPending(self.display):self.x.XNextEvent(self.display,self.C.byref(self.event))
        self.d.XDamageSubtract(self.display,self.damage,0,0);self.x.XSync(self.display,0)
        while self.x.XPending(self.display):self.x.XNextEvent(self.display,self.C.byref(self.event))

    def key(self,name,mods=()):
        codes=[self.x.XKeysymToKeycode(self.display,self.x.XStringToKeysym(k.encode())) for k in [*mods,name]]
        if not all(codes):raise ValueError('unknown X key')
        for code in codes:self.t.XTestFakeKeyEvent(self.display,code,1,0)
        for code in codes[::-1]:self.t.XTestFakeKeyEvent(self.display,code,0,0)
        self.x.XFlush(self.display)

    def feedback(self,trigger,pause=.08):
        self.drain();start=time.monotonic();trigger();elapsed=None
        while time.monotonic()-start<5:
            while self.x.XPending(self.display):
                self.x.XNextEvent(self.display,self.C.byref(self.event))
                if self.C.cast(self.C.byref(self.event),self.C.POINTER(self.C.c_int))[0]==self.base.value:
                    elapsed=(time.monotonic()-start)*1000;break
            if elapsed is not None:break
            if self.proc.poll() is not None:raise RuntimeError('GUI exited during input')
            time.sleep(.001)
        if elapsed is None:raise RuntimeError('input produced no XDamage')
        self.pause(pause);return elapsed

    def action(self,name,mods=(),pause=.08):
        return self.feedback(lambda:self.key(name,mods),pause)

    # `:` opens the command groups; View is the sixth (crates/app/src/ui/menus.rs: Navigate, Search, Edit, Copy,
    # File, View) and starts with Source, Rendered. The rendered-tail OCR check catches a reordered menu.
    VIEW_GROUP=5
    def mode(self,rendered):
        self.action('semicolon',('Shift_L',));self.action('Home')
        for _ in range(self.VIEW_GROUP):self.action('Down')
        self.action('Return');self.action('Home')
        if rendered:self.action('Down')
        return self.action('Return',pause=.35)

    def independent_view(self):
        base_window, base_damage = self.win, self.damage
        before = set(self.windows())
        self.drain()
        self.t.XTestFakeMotionEvent(self.display,0,50,84,0)
        self.t.XTestFakeButtonEvent(self.display,3,1,0);self.t.XTestFakeButtonEvent(self.display,3,0,0);self.x.XFlush(self.display)
        self.pause(.1);self.action('End');self.key('Return')
        started=time.monotonic()
        while not (opened := set(self.windows())-before):
            if time.monotonic()-started>10:raise RuntimeError('independent native window did not open')
            time.sleep(.02)
        if len(opened)!=1:raise RuntimeError('unexpected native window count')
        self.win=opened.pop();self.pause(.6);self.damage=self.d.XDamageCreate(self.display,self.win,3);self.focus(self.win)
        self.mode(True);self.action('End')
        if not (self.output/'independent-rendered.png').exists():
            visible=self.screenshot('independent-rendered')
            if not gui_rendered_tail(visible):raise RuntimeError('independent view did not expose formatted rendered tail')
        children=Path(f'/proc/{self.proc.pid}/task/{self.proc.pid}/children').read_text().split()
        child_stats=[{'pid':int(pid),**process_stats(int(pid))}for pid in children if Path(f'/proc/{pid}/status').exists()]
        self.key('q');started=time.monotonic()
        while self.win in self.windows():
            if time.monotonic()-started>10:raise RuntimeError('independent native window failed to close')
            time.sleep(.01)
        self.win,self.damage=base_window,base_damage;self.focus(self.win);self.pause(.1)
        return child_stats

    def screenshot(self,name):
        path=self.output/(name+'.png')
        subprocess.run(['import','-display',self.env['DISPLAY'],'-window',str(self.win),str(path)],check=True,env=self.env)
        ocr=subprocess.check_output(['tesseract',str(path),'stdout'],stderr=subprocess.DEVNULL,text=True)
        (self.output/(name+'.txt')).write_text(ocr)
        return ocr

    def close(self):
        import signal
        if self.output.is_dir():
            (self.output/'timeline.json').write_text(json.dumps(self.timeline,indent=2))
        if hasattr(self,'proc'):
            with suppress(ProcessLookupError):os.killpg(self.proc.pid,signal.SIGTERM)
            try:self.proc.wait(timeout=3)
            except subprocess.TimeoutExpired:
                with suppress(ProcessLookupError):os.killpg(self.proc.pid,signal.SIGKILL)
                self.proc.wait()
        if getattr(self,'display',None):self.x.XCloseDisplay(self.display);self.display=None
        if hasattr(self,'server'):
            self.server.terminate()
            try:self.server.wait(timeout=3)
            except subprocess.TimeoutExpired:self.server.kill();self.server.wait()
        for attr in ['log','server_log']:
            if hasattr(self,attr):getattr(self,attr).close()
        if hasattr(self,'scratch'):self.scratch.cleanup()


def gui_case(opt,manifest):
    root=Path(manifest['root']);args=['--no-index',f'{opt.fixture}-left.md',f'{opt.fixture}-right.md'];stress_identity=None
    if opt.stress_seconds:
        root,stress_identity=stress_repository(manifest,opt.output);args=['HEAD^','HEAD']
    run=NativeGui(opt.binary,args,root,opt.output,opt.xvfb)
    result={'workload_version':1,'gui_tail_oracle_version':2,'fixture':opt.fixture,'theme':os.environ.get('LENZ_THEME','ambient default (not pinned)'),'completion_boundary':'not observed in production; native first XDamage only'}
    try:
        result['source']=run.sample('source');result['switch_first_damage_ms']=[run.mode(True)]
        run.action('End',pause=.5);ocr=run.screenshot('rendered-tail')
        if not gui_rendered_tail(ocr):raise RuntimeError('native screenshot is missing formatted rendered tail')
        result['tail_screenshot']='rendered-tail.png';result['rendered']=run.sample('rendered')
        result['scroll_first_damage_ms']=[run.action(k) for k in ['Home','Next','End','Up']]
        result['resize_first_damage_ms']=[]
        for w,h in [(640,400),(1400,900),(1200,800)]:
            result['resize_first_damage_ms'].append(run.feedback(lambda:run.x.XResizeWindow(run.display,run.win,w,h),.15))
        run.action('a',('Control_L',));run.key('c',('Control_L',));run.pause(.1)
        if opt.find:run.action('Escape')  # Standalone Find probe clears its copy selection.
        result['navigation_first_damage_ms']=[run.action('n'),run.action('p')];run.action('End')
        if opt.find:
            run.action('f',('Control_L',))
            for letter in 'Paragrap':run.key(letter);run.pause(.03)
            result['find_query_first_damage_ms']=run.action('h')
            result['find_active_scroll_first_damage_ms']=[run.action(k) for k in ['Next','End','Up','F3']*6]
            run.action('End');visible=run.screenshot('active-find')
            if 'Find:' not in visible or 'paragraph' not in visible.lower() or 'TAIL' not in visible:
                raise RuntimeError('active Find navigation did not expose query and rendered tail')
            run.action('Escape')
        if opt.stress_seconds:
            start=time.monotonic();timeline=[0];cycles=0
            while time.monotonic()-start<opt.stress_seconds:
                run.mode(False);run.action('bracketright' if cycles%2==0 else 'bracketleft',pause=.25)
                result['switch_first_damage_ms'].append(run.mode(True));run.action('End')
                run.feedback(lambda:run.x.XResizeWindow(run.display,run.win,1200 if cycles%2 else 1100,800))
                result.setdefault('independent_child_rss',[]).append(run.independent_view())
                run.sample('stress');cycles+=1;timeline.append(time.monotonic()-start)
            result['stress']={'duration_seconds':time.monotonic()-start,'cycles':cycles,'timeline':timeline};validate_stress(result['stress'],opt.stress_seconds)
        run.mode(False);run.pause(1);result['after_source']=run.sample('after-source')
        if stress_identity:verify_fixtures(stress_identity)
        return result
    except BaseException:
        with suppress(Exception):run.screenshot('failure')
        raise
    finally:run.close()


def main():
    scratch=isolate_user_state()  # never measure with the user's own settings or saved positions
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', choices=['tui','tui-startup','gui','identity','fixtures'], required=True)
    parser.add_argument('--xvfb',type=Path,default=shutil.which('Xvfb') or 'Xvfb')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--baseline', type=Path)
    parser.add_argument('--fixtures',type=Path,default=Path('target/markdown/fixtures.json'))
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--fixture',choices=['small','medium','large'],default='small')
    parser.add_argument('--stress-seconds',type=float,default=0)
    parser.add_argument('--find',action='store_true')
    parser.add_argument('--reverse-order',action='store_true')
    parser.add_argument('--runs',type=int,default=6)
    parser.add_argument('--markdown-layout',choices=['inline','side-by-side'],help='[view] markdown_layout for the measured run')
    opt=parser.parse_args()
    if opt.output.exists() and any(opt.output.iterdir()):parser.error('output directory must be new or empty')
    if not math.isfinite(opt.stress_seconds) or opt.stress_seconds<0 or opt.runs<1:parser.error('invalid stress duration or run count')
    opt.output.mkdir(parents=True,exist_ok=True)
    if opt.markdown_layout:
        with open(os.path.join(scratch,'config','lenz','settings.toml'),'a') as f:f.write(f'[view]\nmarkdown_layout = "{opt.markdown_layout}"\n')
    if opt.mode == 'fixtures':
        manifest = generate_fixtures(opt.output / 'inputs')
        (opt.output / 'fixtures.json').write_text(json.dumps(manifest, indent=2) + '\n')
        return
    if opt.binary is None:parser.error('--binary is required for executable probes')
    manifest=json.loads(opt.fixtures.read_text());verify_fixtures(manifest)
    identity={'binary':file_identity(opt.binary),'source':source_identity(),'collector':file_identity(__file__),'arguments':vars(opt)|{'binary':str(opt.binary),'baseline':str(opt.baseline),'fixtures':str(opt.fixtures),'output':str(opt.output),'xvfb':str(opt.xvfb)}};validate_identity(identity)
    (opt.output/'identity.json').write_text(json.dumps(identity,indent=2))
    (opt.output/'fixtures.json').write_text(json.dumps(manifest,indent=2))
    if opt.baseline:
        baseline={'binary':file_identity(opt.baseline),'source':json.loads((opt.baseline.parent/'source.json').read_text())}
        validate_identity(baseline)
        expected=json.loads((opt.baseline.parent/'binaries.json').read_text())[opt.baseline.name]
        if baseline['binary']!=expected:raise ValueError('baseline binary identity mismatch')
        (opt.output/'baseline-identity.json').write_text(json.dumps(baseline,indent=2))
    if opt.mode=='identity':return
    try:
        data={'tui':tui_case,'tui-startup':tui_startup,'gui':gui_case}[opt.mode](opt,manifest)
        verify_fixtures(manifest)
        (opt.output/'results.json').write_text(json.dumps(data,indent=2))
    except BaseException as error:
        (opt.output/'failure.txt').write_text(repr(error))
        raise


if __name__=='__main__':main()

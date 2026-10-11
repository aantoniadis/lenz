#!/usr/bin/env python3
"""Capture GUI palettes on an owned private display (Pillow and python-xlib)."""
import argparse, json, os, selectors, shutil, subprocess, tempfile, time, hashlib
from pathlib import Path
from perf_support import process_stats
from PIL import Image
from Xlib import display, X, XK
from Xlib.ext import xtest

cli=argparse.ArgumentParser(description=__doc__)
cli.add_argument('--app',type=Path,default=Path('target/release/lenz'))
cli.add_argument('--output',type=Path,default=Path('target/themes-validation'))
cli.add_argument('--xvfb',default=shutil.which('Xvfb'))
cli.add_argument('--stress-seconds',type=float,default=0)
opt=cli.parse_args()
if not opt.xvfb:cli.error('--xvfb is required when Xvfb is not on PATH')
binary=opt.app.resolve();out=opt.output.resolve();out.mkdir(parents=True,exist_ok=True)
records=[]
with tempfile.TemporaryDirectory(prefix='lenz-theme-gui-') as scratch:
    root=Path(scratch); env=dict(os.environ,XDG_RUNTIME_DIR=scratch,XDG_CACHE_HOME=str(root/'cache'),XDG_CONFIG_HOME=str(root/'config'),XDG_STATE_HOME=str(root/'state'),LENZ_TIMINGS='1',LENZ_APPEARANCE='');env.pop('WAYLAND_DISPLAY',None)
    r,w=os.pipe(); server=subprocess.Popen([opt.xvfb,'-displayfd',str(w),'-screen','0','1600x1000x24','-nolisten','tcp','-noreset'],pass_fds=(w,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(w)
    with selectors.DefaultSelector()as sel:sel.register(r,selectors.EVENT_READ);assert sel.select(10)
    env['DISPLAY']=':'+os.read(r,100).decode().strip();os.close(r); d=display.Display(env['DISPLAY'])
    for n,text in [('left.rs','fn value() {\n    let old = "old";\n}\n'),('right.rs','fn value() {\n    let new = "new";\n}\n'),('base','base\n'),('ours','ours\n'),('theirs','theirs\n'),('result','ours\n')]: (root/n).write_text(text)
    def start(theme,args):
        log=(root/'app.log').open('w');proc=subprocess.Popen([str(binary),*args],cwd=root,env=dict(env,LENZ_THEME=theme),stdout=log,stderr=log)
        until=time.monotonic()+15;win=None
        while time.monotonic()<until:
            assert proc.poll() is None,(root/'app.log').read_text()
            for child in d.screen().root.query_tree().children:
                if child.get_wm_name():win=child;break
            if win and 'first paint' in (root/'app.log').read_text():break
            time.sleep(.02)
        assert win,'no window';win.set_input_focus(X.RevertToParent,X.CurrentTime);d.sync();time.sleep(.3);return proc,win,log
    def key(win,name,shift=False):
        codes=([d.keysym_to_keycode(XK.string_to_keysym('Shift_L'))] if shift else[])+[d.keysym_to_keycode(XK.string_to_keysym(name))]
        for code in codes:xtest.fake_input(d,X.KeyPress,code)
        for code in reversed(codes):xtest.fake_input(d,X.KeyRelease,code)
        d.sync();time.sleep(.04)
    def image(win):
        geom=win.get_geometry();raw=win.get_image(0,0,geom.width,geom.height,X.ZPixmap,0xffffffff);return Image.frombytes('RGB',(geom.width,geom.height),raw.data,'raw','BGRX')
    def capture(win,name,expected=None):
        im=image(win);path=out/(name+'.png');im.save(path);counts=dict((color,n)for n,color in im.getcolors(im.width*im.height));rgb=tuple((expected>>s)&255 for s in (16,8,0))if expected is not None else None
        if rgb:assert counts.get(rgb,0)>100,(name,rgb,sorted(counts.items(),key=lambda x:x[1],reverse=True)[:8])
        records.append({'file':path.name,'size':im.size,'expected_background':hex(expected)if expected is not None else None,'matching_pixels':counts.get(rgb,0),'sha256':hashlib.sha256(path.read_bytes()).hexdigest()})
    def stop(proc,log):proc.terminate();proc.wait(timeout=5);log.close();d.sync();time.sleep(.1)
    try:
        for theme,bg in [('darcula',0x2b2b2b),('light',0xffffff),('catppuccin',0x1e1e2e),('catppuccin-latte',0xeff1f5),('tokyo-night',0x1a1b26),('gruvbox',0x282828)]:
            proc,win,log=start(theme,['--no-index',str(root/'left.rs'),str(root/'right.rs')]);capture(win,theme+'-comparison',bg);win.configure(width=400,height=240);d.sync();time.sleep(.3);capture(win,theme+'-narrow',bg);stop(proc,log)
        proc,win,log=start('catppuccin',['merge',str(root/'base'),str(root/'ours'),str(root/'theirs'),str(root/'result')]);capture(win,'catppuccin-merge',0x1e1e2e);stop(proc,log)
        subprocess.run(['git','init','-q'],cwd=root,check=True);subprocess.run(['git','-c','user.name=Theme Author','-c','user.email=theme@example.invalid','add','left.rs','right.rs','base','ours','theirs','result'],cwd=root,check=True);subprocess.run(['git','-c','user.name=Theme Author','-c','user.email=theme@example.invalid','-c','commit.gpgsign=false','commit','-qm','Theme fixture'],cwd=root,check=True)
        proc,win,log=start('catppuccin-latte',['log']);capture(win,'latte-repository',0xeff1f5);stop(proc,log)
        proc,win,log=start('catppuccin',['--about']);capture(win,'catppuccin-about',0x1e1e2e);stop(proc,log)
        current=root/'state/omarchy/current';(current/'theme').mkdir(parents=True)
        colors='mode="dark"\nbackground="#1e1e2e"\nforeground="#cdd6f4"\naccent="#89b4fa"\nred="#f38ba8"\ngreen="#a6e3a1"\nblue="#89b4fa"\nyellow="#f9e2af"\nmagenta="#cba6f7"\ncyan="#94e2d5"\n'
        (current/'theme/colors.toml').write_text(colors);(current/'theme.name').write_text('GUI live initial')
        proc,win,log=start('system',['--no-index',str(root/'left.rs'),str(root/'right.rs')]);capture(win,'system-before',0x1e1e2e)
        (current/'next').mkdir();(current/'next/colors.toml').write_text(colors.replace('#1e1e2e','#282828'));start_time=time.monotonic();(current/'theme').rename(current/'previous');(current/'next').rename(current/'theme');(current/'theme.name').write_text('GUI live custom')
        until=time.monotonic()+5
        while time.monotonic()<until:
            im=image(win);counts=dict((c,n)for n,c in im.getcolors(im.width*im.height))
            if counts.get((40,40,40),0)>1000:break
            time.sleep(.01)
        else:raise RuntimeError('GUI did not recolor without a keypress')
        records.append({'live_replacement_capture_ms':(time.monotonic()-start_time)*1000,'input_sent':False})
        capture(win,'system-after',0x282828)
        xtest.fake_input(d,X.MotionNotify,x=win.get_geometry().x+24,y=win.get_geometry().y+15);xtest.fake_input(d,X.ButtonPress,1);xtest.fake_input(d,X.ButtonRelease,1);d.sync();time.sleep(.3);capture(win,'system-chooser')
        key(win,'Escape');capture(win,'system-dismissed',0x282828)
        stress_start=time.monotonic();timeline=[];latencies=[];cycles=0
        while time.monotonic()-stress_start<opt.stress_seconds:
            bg=0x1e1e2e if cycles%2==0 else 0x282828
            (current/'next').mkdir();(current/'next/colors.toml').write_text(colors.replace('#1e1e2e',f'#{bg:06x}'))
            at=time.monotonic();(current/'theme').rename(current/'old');(current/'next').rename(current/'theme');(current/'theme.name').write_text(f'GUI stress {cycles}');(current/'old/colors.toml').unlink();(current/'old').rmdir()
            until=time.monotonic()+5
            while time.monotonic()<until:
                im=image(win);counts=dict((c,n)for n,c in im.getcolors(im.width*im.height))
                if counts.get(tuple((bg>>shift)&255 for shift in (16,8,0)),0)>100:break
                time.sleep(.01)
            else:raise RuntimeError('stress theme failed to recolor')
            latencies.append((time.monotonic()-at)*1000)
            xtest.fake_input(d,X.MotionNotify,x=win.get_geometry().x+24,y=win.get_geometry().y+15);xtest.fake_input(d,X.ButtonPress,1);xtest.fake_input(d,X.ButtonRelease,1);d.sync();time.sleep(.04);key(win,'End');key(win,'Up');key(win,'Escape')
            if cycles%8==0:win.configure(width=400 if cycles%16==0 else 1536,height=240 if cycles%16==0 else 1000);d.sync()
            timeline.append({'elapsed_s':time.monotonic()-stress_start,**process_stats(proc.pid)})
            cycles+=1;time.sleep(.05)
        if opt.stress_seconds:records.append({'gui_theme_stress':{'requested_s':opt.stress_seconds,'elapsed_s':time.monotonic()-stress_start,'cycles':cycles,'recolored_capture_ms':latencies,'memory_timeline':timeline,'limits':'Private software display capture and process RSS; no hardware scanout/GPU leak proof.'}})
        stop(proc,log)
        assert (root/'right.rs').read_text()=='fn value() {\n    let new = "new";\n}\n'
        (out/'gui-manifest.json').write_text(json.dumps({'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'display':'owned private Xvfb, software rendering','collector_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'source_dirty':bool(subprocess.check_output(['git','status','--porcelain'],text=True).strip()),'records':records},indent=2)+'\n')
    finally:d.close();server.terminate();server.wait(timeout=5)

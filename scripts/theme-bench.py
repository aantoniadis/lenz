#!/usr/bin/env python3
"""Owned PTY theme transitions, correctness and memory stress. Metrics are additive."""
import argparse
import codecs
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import pty
import subprocess
import tempfile
import termios
import time
import unicodedata

_spec = importlib.util.spec_from_file_location('tui_bench', Path(__file__).with_name('tui-bench.py'))
_bench = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_bench)


class ColorScreen:
    """VT subset emitted by Ratatui; current cells retain their SGR colors."""
    def __init__(self, width=120, height=36):
        self.width, self.height = width, height
        self.cells = [[' ']*width for _ in range(height)]
        self.background = [[None]*width for _ in range(height)]
        self.foreground = [[None]*width for _ in range(height)]
        self.x = self.y = 0
        self.fg = self.bg = None
        self.pending = ''
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')

    def put(self, ch):
        if self.x >= self.width:
            self.x = 0
            self.y = min(self.height-1, self.y+1)
        width = 2 if unicodedata.east_asian_width(ch) in ('W', 'F') else 1
        for i in range(min(width, self.width-self.x)):
            self.cells[self.y][self.x+i] = ch if i == 0 else ''
            self.foreground[self.y][self.x+i] = self.fg
            self.background[self.y][self.x+i] = self.bg
        self.x += width

    def erase(self, y, start, end):
        self.cells[y][start:end] = [' ']*(end-start)
        self.foreground[y][start:end] = [self.fg]*(end-start)
        self.background[y][start:end] = [self.bg]*(end-start)

    def csi(self, params, final):
        if params.startswith('?'):
            return
        v = [int(p) if p.isdigit() else 0 for p in params.split(';')]
        n = v[0] or 1
        if final == 'm':
            i = 0
            while i < len(v):
                if v[i] == 0: self.fg = self.bg = None
                elif v[i] == 39: self.fg = None
                elif v[i] == 49: self.bg = None
                elif v[i] in (38, 48) and i+4 < len(v) and v[i+1] == 2:
                    rgb = (v[i+2]<<16) | (v[i+3]<<8) | v[i+4]
                    if v[i] == 38: self.fg = rgb
                    else: self.bg = rgb
                    i += 4
                elif v[i] in (38, 48) and i+2 < len(v) and v[i+1] == 5:
                    # Retain indexed colors distinctly; they cannot satisfy an RGB assertion.
                    if v[i] == 38: self.fg = ('indexed', v[i+2])
                    else: self.bg = ('indexed', v[i+2])
                    i += 2
                elif 30 <= v[i] <= 37: self.fg = ('indexed', v[i]-30)
                elif 40 <= v[i] <= 47: self.bg = ('indexed', v[i]-40)
                i += 1
        elif final in 'Hf':
            self.y = min(self.height-1, max(0, n-1))
            self.x = min(self.width-1, max(0, (v[1] if len(v)>1 else 1)-1))
        elif final == 'G': self.x = min(self.width-1, n-1)
        elif final == 'd': self.y = min(self.height-1, n-1)
        elif final == 'A': self.y = max(0, self.y-n)
        elif final == 'B': self.y = min(self.height-1, self.y+n)
        elif final == 'C': self.x = min(self.width-1, self.x+n)
        elif final == 'D': self.x = max(0, self.x-n)
        elif final in 'EF':
            self.y = min(self.height-1, self.y+n) if final == 'E' else max(0, self.y-n)
            self.x = 0
        elif final == 'b':
            ch = self.cells[self.y][max(0, self.x-1)]
            for _ in range(n): self.put(ch)
        elif final == 'K':
            start, end = (0, self.width) if v[0] == 2 else ((0, min(self.x+1, self.width)) if v[0] == 1 else (min(self.x,self.width), self.width))
            self.erase(self.y, start, end)
        elif final == 'J':
            if v[0] in (2,3):
                for y in range(self.height): self.erase(y, 0, self.width)
            elif v[0] == 0:
                self.erase(self.y, min(self.x,self.width), self.width)
                for y in range(self.y+1,self.height): self.erase(y,0,self.width)
            elif v[0] == 1:
                for y in range(self.y): self.erase(y,0,self.width)
                self.erase(self.y,0,min(self.x+1,self.width))

    def feed(self, data):
        self.pending += self.decoder.decode(data)
        i = 0
        while i < len(self.pending):
            ch = self.pending[i]
            if ch == '\x1b':
                if i+1 == len(self.pending): break
                if self.pending[i+1] == '[':
                    end = i+2
                    while end < len(self.pending) and not ('@' <= self.pending[end] <= '~'): end += 1
                    if end == len(self.pending): break
                    self.csi(self.pending[i+2:end],self.pending[end]); i=end+1; continue
                if self.pending[i+1] == ']':
                    ends = [(self.pending.find('\x07',i+2),1),(self.pending.find('\x1b\\',i+2),2)]
                    ends = [(at,n) for at,n in ends if at>=0]
                    if not ends: break
                    at,n=min(ends); i=at+n; continue
                i += 2; continue
            if ch == '\r': self.x = 0
            elif ch == '\n': self.y = min(self.height-1,self.y+1)
            elif ch == '\b': self.x = max(0,self.x-1)
            elif ord(ch)>=32 and not unicodedata.combining(ch): self.put(ch)
            i += 1
        self.pending = self.pending[i:]

    def text(self): return '\n'.join(''.join(row) for row in self.cells)


def matches_palette(screen, expected_rgb):
    return screen.background[0][0] == expected_rgb


def wait_transition(probe, screen, expected_rgb, marker, timeout=5):
    started = time.monotonic()
    deadline = started+timeout
    while not (matches_palette(screen, expected_rgb) and marker in screen.text()):
        if time.monotonic() >= deadline: raise TimeoutError(f'No palette transition to {expected_rgb:#x}; cell00={screen.background[0][0]!r}; prefix={bytes(getattr(probe, "output", b"")[:200])!r}\n{screen.text()}')
        screen.feed(probe.read(.01))
        probe.sample()
    return (time.monotonic()-started)*1000


class Probe(_bench.TerminalProbe):
    def __init__(self, command, root, env):
        self.master, self.slave = pty.openpty()
        self.original = termios.tcgetattr(self.slave)
        self.resize(120,36)
        self.output = bytearray()
        self.peak = None
        self.started = time.monotonic()
        self.proc = subprocess.Popen(command,cwd=root,stdin=self.slave,stdout=self.slave,stderr=self.slave,start_new_session=True,env=dict(os.environ,TERM='xterm-256color',NO_COLOR='',COLORTERM='truecolor',GIT_CONFIG_GLOBAL='/dev/null',GIT_CONFIG_NOSYSTEM='1',**env))


def palette_text(bg):
    return f'''mode = "dark"
background = "#{bg:06x}"
foreground = "#cdd6f4"
accent = "#89b4fa"
red = "#f38ba8"
green = "#a6e3a1"
blue = "#89b4fa"
yellow = "#f9e2af"
magenta = "#cba6f7"
cyan = "#94e2d5"
[palette]
header_bg = "#{bg:06x}"
'''


def digest(path): return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--app',type=Path,required=True)
    p.add_argument('--seconds',type=float,default=60)
    p.add_argument('--lines',type=int,default=100000)
    p.add_argument('--output',type=Path,required=True)
    args=p.parse_args()
    if args.seconds<=0 or args.lines<1: p.error('seconds and lines must be positive')
    app=args.app.resolve(); args.output.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='lenz-theme-probe-') as tmp:
        root=Path(tmp)
        for name,word in [('left','OLD'),('right','NEW')]:
            (root/name).write_text(''.join(f'THEME_{word}_{i}\n' if i%80==0 else f'unchanged {i}\n' for i in range(args.lines)))
        before={name:digest(root/name) for name in ('left','right')}
        current=root/'state/omarchy/current'; (current/'theme').mkdir(parents=True)
        (current/'theme/colors.toml').write_text(palette_text(0x1e1e2e));(current/'theme.name').write_text('probe-start')
        probe=Probe([str(app),'--no-index','left','right'],root,{'XDG_CONFIG_HOME':str(root/'config'),'XDG_STATE_HOME':str(root/'state'),'LENZ_THEME':'system','LENZ_APPEARANCE':''})
        screen=ColorScreen();samples=[];transitions=[];cycles=0
        started=time.monotonic()
        try:
            first=wait_transition(probe,screen,0x1e1e2e,'THEME_NEW_0',timeout=15)
            stress_start=time.monotonic()
            while time.monotonic()-stress_start<args.seconds:
                bg=0xeff1f5 if cycles%2==0 else 0x1e1e2e
                next_dir=current/'next'; next_dir.mkdir();(next_dir/'colors.toml').write_text(palette_text(bg))
                at=time.monotonic();(current/'theme').rename(current/'previous');next_dir.rename(current/'theme');(current/'previous/colors.toml').unlink();(current/'previous').rmdir();(current/'theme.name').write_text(f'probe-{cycles}')
                latency=wait_transition(probe,screen,bg,'THEME_NEW_0')
                transitions.append({'cycle':cycles,'replacement_s':at-started,'complete_s':time.monotonic()-started,'pty_color_ms':latency})
                if cycles%4==0:
                    os.write(probe.master,b':Change theme\r')
                    deadline=time.monotonic()+5
                    while f'Current: probe-{cycles}' not in screen.text():
                        if time.monotonic()>deadline: raise TimeoutError('chooser did not show current source')
                        screen.feed(probe.read(.01))
                    os.write(probe.master,b'\x1b')
                    time.sleep(.06)
                    while data:=probe.read(.01): screen.feed(data)
                samples.append({'elapsed_s':time.monotonic()-started,**probe.sample()})
                cycles+=1
                time.sleep(.1)
            # Settled output must stay silent; theme watcher is event driven.
            while data:=probe.read(.05): screen.feed(data)
            idle_start=len(probe.output); idle_at=time.monotonic();idle_cpu=probe.sample()
            while time.monotonic()-idle_at<.5: screen.feed(probe.read(.05))
            idle_after=probe.sample();idle_bytes=len(probe.output)-idle_start;assert idle_bytes==0,'idle emitted output'
            if idle_cpu['cpu_seconds'] is not None and idle_after['cpu_seconds'] is not None:
                assert idle_after['cpu_seconds']-idle_cpu['cpu_seconds']<=.02,'idle theme watcher used excessive CPU'
            assert before=={name:digest(root/name) for name in before},'theme operations changed fixture bytes'
            (args.output/'screen.txt').write_text(screen.text())
            (args.output/'terminal.ansi').write_bytes(probe.output)
            elapsed=time.monotonic()-stress_start
            probe.finish()
            result={'workload_version':1,'app':str(app),'executable_sha256':digest(app),'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'source_dirty':bool(subprocess.check_output(['git','status','--porcelain'],text=True).strip()),'collector_sha256':digest(Path(__file__)),'config':{'lines':args.lines,'requested_seconds':args.seconds,'size':[120,36],'NO_COLOR':'','COLORTERM':'truecolor'},'fixture_hashes':before,'first_content_ms':first,'stress_elapsed_s':elapsed,'cycles':cycles,'transitions':transitions,'memory_timeline':samples,'peak_rss_mib':probe.peak,'idle_bytes':idle_bytes,'idle_cpu_before':idle_cpu,'idle_cpu_after':idle_after,'restored':True,'limits':'PTY receipt, not terminal display latency; RSS, not GPU memory.'}
            (args.output/'results.json').write_text(json.dumps(result,indent=2)+'\n')
            print(json.dumps({'cycles':cycles,'stress_elapsed_s':elapsed,'peak_rss_mib':probe.peak,'output':str(args.output)}))
        finally:
            (args.output/'terminal.ansi').write_bytes(probe.output)
            probe.close()

if __name__=='__main__':main()

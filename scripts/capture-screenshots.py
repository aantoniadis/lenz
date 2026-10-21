#!/usr/bin/env python3
"""Capture the README/docs screenshots on an owned private display (Pillow, python-xlib, Alacritty).

Builds a disposable demo repository, opens GUI windows and the TUI inside Alacritty on a
private Xvfb, and writes PNGs to docs/images/. Never touches the user's desktop or repositories.
"""
import argparse, os, shutil, subprocess, tempfile, time
from pathlib import Path
from perf_support import read_display_number
from PIL import Image
from Xlib import display, X, XK
from Xlib.ext import xtest

ROOT = Path(__file__).resolve().parents[1]
cli = argparse.ArgumentParser(description=__doc__)
cli.add_argument('--gui', type=Path, default=ROOT/'target/release/lenz')
cli.add_argument('--tui', type=Path, default=ROOT/'target/release/lenz-tui')
cli.add_argument('--reader', type=Path, help='Also capture the Reader with this executable')
cli.add_argument('--output', type=Path, default=ROOT/'docs/images')
cli.add_argument('--xvfb', default=shutil.which('Xvfb'))
cli.add_argument('--theme', default='catppuccin')
cli.add_argument('--font', default='JetBrainsMono Nerd Font Mono', help='Alacritty font for TUI captures')
cli.add_argument('--only', nargs='*', help='Capture only these names')
opt = cli.parse_args()
if not opt.xvfb:
    cli.error('--xvfb is required when Xvfb is not on PATH')
if not shutil.which('alacritty'):
    cli.error('alacritty is required for TUI captures')
GUI_SIZE, TUI_GRID = (1440, 860), (150, 40)

CONFIG_OLD = '''use std::time::Duration;

/// Runtime settings, loaded from `ledger.toml`.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub workers: usize,
    pub timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".into(),
            workers: 4,
            timeout: Duration::from_secs(30),
        }
    }
}

pub fn load(text: &str) -> Result<Config, String> {
    let mut config = Config::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key.trim() {
            "listen" => config.listen = value.trim().into(),
            "workers" => config.workers = value.trim().parse().map_err(|e| format!("workers: {e}"))?,
            _ => {}
        }
    }
    Ok(config)
}
'''
CONFIG_NEW = '''use std::time::Duration;

/// Runtime settings, loaded from `ledger.toml` and the environment.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: String,
    pub workers: usize,
    pub timeout: Duration,
    pub max_body_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:8080".into(),
            workers: std::thread::available_parallelism().map_or(4, |n| n.get()),
            timeout: Duration::from_secs(30),
            max_body_bytes: 1 << 20,
        }
    }
}

pub fn load(text: &str) -> Result<Config, String> {
    let mut config = Config::default();
    for (number, line) in text.lines().enumerate() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "listen" => config.listen = value.into(),
            "workers" => config.workers = value.parse().map_err(|e| format!("line {}: workers: {e}", number + 1))?,
            "timeout_secs" => config.timeout = Duration::from_secs(value.parse().map_err(|e| format!("timeout: {e}"))?),
            other => return Err(format!("line {}: unknown key `{other}`", number + 1)),
        }
    }
    Ok(config)
}
'''
SERVER = '''use crate::config::Config;
use crate::store::Store;

pub struct Server {
    config: Config,
    store: Store,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self { config, store: Store::default() }
    }

    pub fn run(&self) -> std::io::Result<()> {
        println!("listening on {}", self.config.listen);
        for _ in 0..self.config.workers {
            self.spawn_worker();
        }
        Ok(())
    }

    fn spawn_worker(&self) {
        let _ = &self.store;
    }
}
'''
README_OLD = '''# Ledger

A small double-entry bookkeeping service.

## Running

Start the server with the default settings:

```sh
cargo run --release
```

## Configuration

Settings live in `ledger.toml`:

- `listen`: address to bind
- `workers`: number of worker threads
'''


def git(repo, *args, date=None):
    env = dict(os.environ, GIT_CONFIG_GLOBAL='/dev/null', GIT_CONFIG_NOSYSTEM='1')
    if date:
        env.update(GIT_AUTHOR_DATE=date, GIT_COMMITTER_DATE=date)
    return subprocess.run(['git', '-c', 'commit.gpgsign=false', *args], cwd=repo, env=env, check=True,
                          capture_output=True, text=True).stdout


def commit(repo, message, who, day, files):
    for name, text in files.items():
        (repo/name).parent.mkdir(parents=True, exist_ok=True)
        (repo/name).write_text(text)
    git(repo, 'add', '-A')
    name, email = who
    git(repo, '-c', f'user.name={name}', '-c', f'user.email={email}', 'commit', '-qm', message,
        date=f'2026-09-{day:02} 10:{day:02}:00 +0200')


def demo_repository(repo):
    """A Rust service with branches, a merge and uncommitted changes."""
    ada, lin, sam = ('Ada Park', 'ada@example.invalid'), ('Lin Ortega', 'lin@example.invalid'), ('Sam Rivera', 'sam@example.invalid')
    repo.mkdir()
    git(repo, 'init', '-q', '-b', 'main')
    commit(repo, 'Initial ledger service', ada, 1, {'Cargo.toml': '[package]\nname = "ledger"\nversion = "0.1.0"\nedition = "2024"\n',
           'src/main.rs': 'mod config;\nmod server;\nmod store;\n\nfn main() {\n    let config = config::Config::default();\n    server::Server::new(config).run().unwrap();\n}\n',
           'src/config.rs': CONFIG_OLD.replace('    pub timeout: Duration,\n', '').replace('            timeout: Duration::from_secs(30),\n', ''),
           'src/server.rs': SERVER, 'src/store.rs': '#[derive(Default)]\npub struct Store;\n', 'README.md': README_OLD})
    commit(repo, 'Add request timeout setting', lin, 3, {'src/config.rs': CONFIG_OLD})
    git(repo, 'checkout', '-qb', 'feature/accounts')
    commit(repo, 'Add account model', sam, 4, {'src/account.rs': 'pub struct Account {\n    pub id: u64,\n    pub name: String,\n}\n'})
    commit(repo, 'Store accounts in memory', sam, 6, {'src/store.rs': '#[derive(Default)]\npub struct Store {\n    accounts: Vec<crate::account::Account>,\n}\n'})
    git(repo, 'checkout', '-q', 'main')
    commit(repo, 'Document configuration keys', ada, 5, {'README.md': README_OLD + '\nSee `src/config.rs` for parsing rules.\n'})
    commit(repo, 'Log the listening address', lin, 7, {'src/server.rs': SERVER.replace('listening on', 'ledger listening on')})
    git(repo, '-c', 'user.name=Ada Park', '-c', 'user.email=ada@example.invalid', 'merge', '-q', '--no-ff',
        'feature/accounts', '-m', "Merge branch 'feature/accounts'", date='2026-09-08 09:00:00 +0200')
    for day, (message, who, extra) in enumerate([('Validate worker count', lin, '// validated\n'), ('Add health endpoint', sam, '// health\n'),
                                                  ('Bump edition', ada, '// edition\n'), ('Tidy store API', lin, '// tidy\n')], start=9):
        commit(repo, message, who, day, {'src/store.rs': (repo/'src/store.rs').read_text() + extra})
    git(repo, 'tag', 'v0.1.0')
    git(repo, 'checkout', '-qb', 'feature/limits')
    commit(repo, 'Limit request body size', sam, 14, {'src/server.rs': SERVER.replace('listening on', 'ledger listening on') + '\n// limits\n'})
    git(repo, 'checkout', '-q', 'main')
    (repo/'src/config.rs').write_text(CONFIG_NEW)
    (repo/'src/limits.rs').write_text('/// Largest request body the server accepts.\npub const MAX_BODY_BYTES: usize = 1 << 20;\n')
    (repo/'src/server.rs').write_text((repo/'src/server.rs').read_text().replace(
        '        println!("ledger listening on {}", self.config.listen);',
        '        eprintln!("ledger listening on {} with {} workers", self.config.listen, self.config.workers);'))


def merge_inputs(folder):
    """Base/ours/theirs/result for a three-way Merge View with one conflict and clean changes."""
    folder.mkdir()
    base = CONFIG_OLD
    ours = base.replace('"127.0.0.1:8080"', '"0.0.0.0:8080"').replace('workers: 4,', 'workers: 8,')
    theirs = base.replace('workers: 4,', 'workers: std::thread::available_parallelism().map_or(4, |n| n.get()),').replace(
        '            _ => {}', '            other => return Err(format!("unknown key `{other}`")),')
    for name, text in [('base.rs', base), ('ours.rs', ours), ('theirs.rs', theirs), ('config.rs', base)]:
        (folder/name).write_text(text)
    return [str(folder/n) for n in ('base.rs', 'ours.rs', 'theirs.rs', 'config.rs')]


class Display:
    def __init__(self, scratch):
        r, w = os.pipe()
        self.server = subprocess.Popen([opt.xvfb, '-displayfd', str(w), '-screen', '0', '1920x1200x24', '-nolisten', 'tcp', '-noreset'],
                                       pass_fds=(w,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(w)
        try:
            number = read_display_number(r)
        finally:
            os.close(r)
        self.name = ':' + number
        self.d = display.Display(self.name)
        self.env = dict(os.environ, DISPLAY=self.name, XDG_RUNTIME_DIR=str(scratch), XDG_CONFIG_HOME=str(scratch/'config'),
                        XDG_CACHE_HOME=str(scratch/'cache'), XDG_STATE_HOME=str(scratch/'state'), GIT_CONFIG_GLOBAL='/dev/null',
                        GIT_CONFIG_NOSYSTEM='1', LENZ_THEME=opt.theme, LENZ_ICONS='nerd', COLORTERM='truecolor', LENZ_TIMINGS='1')
        self.env.pop('WAYLAND_DISPLAY', None)

    def windows(self):
        return [w for w in self.d.screen().root.query_tree().children
                if w.get_wm_name() and w.get_attributes().map_state == X.IsViewable]

    def launch(self, command, cwd, ready):
        log = tempfile.NamedTemporaryFile('w+', prefix='lenz-capture-', suffix='.log')
        proc = subprocess.Popen(command, cwd=cwd, env=self.env, stdout=log, stderr=log, start_new_session=True)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if proc.poll() is not None:
                log.seek(0)
                raise RuntimeError(f'{command[0]} exited: {log.read()[-2000:]}')
            wins = self.windows()
            log.seek(0)
            if wins and ready(log.read()):
                win = wins[0]
                win.set_input_focus(X.RevertToParent, X.CurrentTime)
                self.d.sync()
                return proc, win, log
            time.sleep(.05)
        raise RuntimeError(f'{command[0]} never showed a ready window')

    def key(self, win, name, *mods):
        codes = [self.d.keysym_to_keycode(XK.string_to_keysym(k)) for k in (*mods, name)]
        for code in codes:
            xtest.fake_input(self.d, X.KeyPress, code)
        for code in reversed(codes):
            xtest.fake_input(self.d, X.KeyRelease, code)
        self.d.sync()
        time.sleep(.15)

    def click(self, win, x, y):
        geometry = win.get_geometry()
        origin = win.translate_coords(self.d.screen().root, 0, 0)
        xtest.fake_input(self.d, X.MotionNotify, x=x - origin.x, y=y - origin.y)
        xtest.fake_input(self.d, X.ButtonPress, 1)
        xtest.fake_input(self.d, X.ButtonRelease, 1)
        self.d.sync()
        time.sleep(.2)
        return geometry

    def click_away(self, win):
        """Park the pointer over empty content so no toolbar tooltip stays open."""
        origin = win.translate_coords(self.d.screen().root, 0, 0)
        xtest.fake_input(self.d, X.MotionNotify, x=GUI_SIZE[0] - 200 - origin.x, y=GUI_SIZE[1] - 100 - origin.y)
        self.d.sync()

    def image(self, win):
        g = win.get_geometry()
        raw = win.get_image(0, 0, g.width, g.height, X.ZPixmap, 0xffffffff)
        return Image.frombytes('RGB', (g.width, g.height), raw.data, 'raw', 'BGRX')

    def find_word(self, win, word):
        """Locate a visible label with tesseract; returns the centre in window coordinates."""
        with tempfile.NamedTemporaryFile(suffix='.png') as shot:
            self.image(win).resize((GUI_SIZE[0] * 2, GUI_SIZE[1] * 2)).save(shot.name)
            tsv = subprocess.check_output(['tesseract', shot.name, 'stdout', 'tsv'], stderr=subprocess.DEVNULL, text=True)
        for row in tsv.splitlines()[1:]:
            cells = row.split('\t')
            if len(cells) == 12 and cells[11].strip() == word:
                left, top, width, height = map(int, cells[6:10])
                return (left + width // 2) // 2, (top + height // 2) // 2
        raise RuntimeError(f'label {word!r} not found')

    def close(self):
        self.d.close()
        self.server.terminate()
        self.server.wait(timeout=5)


def stop(proc, log):
    if proc.poll() is None:
        os.killpg(proc.pid, 15)
        proc.wait(timeout=5)
    log.close()


def main():
    out = opt.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    Image.open(ROOT/'assets/lenz-icon.png').resize((256, 256), Image.LANCZOS).save(out/'lenz-logo.png', optimize=True)
    with tempfile.TemporaryDirectory(prefix='lenz-screenshots-') as scratch:
        scratch = Path(scratch)
        repo = scratch/'ledger'
        demo_repository(repo)
        merge = merge_inputs(scratch/'merge')
        (scratch/'docs').mkdir()
        (scratch/'docs/old/').mkdir()
        (scratch/'docs/new/').mkdir()
        fixtures = ROOT/'crates/app/fixtures/markdown'
        shutil.copy(fixtures/'reading-view.md', scratch/'docs/old/README.md')
        shutil.copy(fixtures/'reading-view-new.md', scratch/'docs/new/README.md')
        for side in ('old', 'new'):
            shutil.copy(fixtures/'diagram.png', scratch/'docs'/side/'diagram.png')
        markdown = ['--no-index', 'old/README.md', 'new/README.md']
        screen = Display(scratch)

        def gui(name, args, cwd, actions=()):
            proc, win, log = screen.launch([str(opt.gui.resolve()), *args], cwd, lambda text: 'first paint' in text)
            try:
                win.configure(width=GUI_SIZE[0], height=GUI_SIZE[1])
                screen.d.sync()
                time.sleep(1.5)
                for action in actions:
                    action(win)
                    time.sleep(1)
                screen.image(win).save(out/f'{name}.png', optimize=True)
                print(out/f'{name}.png')
            finally:
                stop(proc, log)

        def tui(name, args, cwd, keys=()):
            cols, lines = TUI_GRID
            command = ['alacritty', '--config-file', '/dev/null', '-o', f'font.normal.family="{opt.font}"', '-o', 'font.size=11',
                       '-o', f'window.dimensions.columns={cols}', '-o', f'window.dimensions.lines={lines}',
                       '-e', str(opt.tui.resolve()), *args]
            proc, win, log = screen.launch(command, cwd, lambda text: True)
            try:
                time.sleep(2.5)
                for key in keys:
                    screen.key(win, *key)
                time.sleep(.8)
                screen.image(win).save(out/f'{name}.png', optimize=True)
                print(out/f'{name}.png')
            finally:
                stop(proc, log)

        def rendered(win):
            x, y = screen.find_word(win, 'Inline')
            screen.click(win, x, y)
            screen.click_away(win)

        def reader(name, root, file, keys):
            proc, win, log = screen.launch([str(opt.reader.resolve()), str(root)], root, lambda text: True)
            try:
                win.configure(width=GUI_SIZE[0], height=GUI_SIZE[1])
                screen.d.sync()
                time.sleep(2)
                screen.key(win, 'p', 'Control_L')
                for c in file:
                    screen.key(win, 'period' if c == '.' else c)
                screen.key(win, 'Return')
                time.sleep(2)
                screen.click(win, 266, 10)  # Reveal the opened file in the explorer.
                for key in keys:
                    screen.key(win, *key)
                screen.click_away(win)
                time.sleep(1)
                screen.image(win).save(out/f'{name}.png', optimize=True)
                print(out/f'{name}.png')
            finally:
                stop(proc, log)

        shots = {
            'gui-comparison': lambda: gui('gui-comparison', [], repo),
            'gui-repository': lambda: gui('gui-repository', ['log'], repo),
            'gui-merge': lambda: gui('gui-merge', ['merge', *merge], scratch),
            'gui-markdown': lambda: gui('gui-markdown', markdown, scratch/'docs', [rendered]),
            'tui-comparison': lambda: tui('tui-comparison', [], repo),
            'tui-repository': lambda: tui('tui-repository', ['log'], repo),
            'tui-merge': lambda: tui('tui-merge', ['merge', *merge], scratch),
        }
        if opt.reader:
            shots.update({
                'reader-code': lambda: reader('reader-code', repo, 'config.rs',
                                              [('f', 'Control_L'), *[(c,) for c in 'workers']]),
                'reader-markdown': lambda: reader('reader-markdown', scratch/'docs/new', 'README.md',
                                                  [('v', 'Control_L', 'Shift_L')]),
            })
        try:
            for name, take in shots.items():
                if not opt.only or name in opt.only:
                    take()
        finally:
            screen.close()
        assert (scratch/'merge/config.rs').read_text() == CONFIG_OLD, 'merge result must stay unsaved'
        assert (repo/'src/config.rs').read_text() == CONFIG_NEW, 'Reader must preserve source bytes'
        assert (scratch/'docs/new/README.md').read_bytes() == (fixtures/'reading-view-new.md').read_bytes()


if __name__ == '__main__':
    main()

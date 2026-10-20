"""Exercise the real packaging script without compiling Rust or touching user files."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest


# Each installed executable and the Cargo executable it is built as.
SHIPPED = [('lenz', 'lenz-launcher'), ('lenz-code', 'lenz-reader'), ('lenz-code-tui', 'lenz-reader-tui'),
           ('lenz-diff', 'lenz'), ('lenz-diff-tui', 'lenz-tui')]


class PackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='lenz-package-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'packaging').mkdir()
        shutil.copyfile(Path(__file__).resolve().parents[1] / 'packaging/package.sh', self.root / 'packaging/package.sh')
        for name in ['README.md', 'LICENSE-MIT', 'LICENSE-THEMES', 'crates/engine/LICENSE', 'crates/engine/NOTICE',
                     'packaging/lenz.desktop', 'packaging/lenz-diff.desktop', 'assets/lenz-icon.png', 'assets/lenz.icns']:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(name)
        stale = self.root / 'target/dist/lenz'
        stale.parent.mkdir(parents=True)
        stale.write_bytes(b'stale executable')
        self.tools = self.root / 'tools'
        self.tools.mkdir()
        self.artifact = self.root / 'custom target/x86_64-test-linux-gnu/dist/lenz'
        self.env = os.environ.copy()
        self.env.update(PATH=str(self.tools) + os.pathsep + self.env['PATH'],
                        CARGO_TARGET_DIR=str(self.root / 'custom target'),
                        CARGO_BUILD_TARGET='aarch64-other-target',
                        LENZ_TEST_ARTIFACT=str(self.artifact), LENZ_TEST_CALLS=str(self.root / 'calls.json'))
        self.fake('rustc', "print('host: x86_64-test-linux-gnu')")
        self.fake('git', "print('974d7df')")
        self.fake('cargo', '''import json, os, pathlib, sys
calls = pathlib.Path(os.environ['LENZ_TEST_CALLS'])
calls.write_text(json.dumps((json.loads(calls.read_text()) if calls.exists() else []) + [sys.argv[1:]]))
pathlib.Path(os.environ['LENZ_TEST_CALLS']).with_name('flags.json').write_text(json.dumps({k: os.environ.get(k) for k in ('RUSTFLAGS', 'CFLAGS', 'CXXFLAGS')}))
package = sys.argv[sys.argv.index('-p') + 1]
# Cargo names an executable by its bin target, not its package.
name = {'reader-gui': 'lenz-reader', 'reader-tui': 'lenz-reader-tui'}.get(package, package)
artifact = pathlib.Path(os.environ['LENZ_TEST_ARTIFACT']).with_name(name)
artifact.parent.mkdir(parents=True, exist_ok=True)
artifact.write_bytes(b'fresh ' + name.encode())
artifact.chmod(0o755)  # as Cargo leaves it
if not os.environ.get('LENZ_TEST_SKIP_ARTIFACT'):
    print(json.dumps({'reason': 'compiler-artifact', 'target': {'name': name, 'kind': ['bin']}, 'executable': str(artifact)}))
print(json.dumps({'reason': 'build-finished', 'success': True}))''')

    def fake(self, name, body):
        path = self.tools / name
        path.write_text('#!/usr/bin/env python3\n' + body + '\n')
        path.chmod(0o755)

    def package(self, version='0.2.0'):
        return subprocess.run(['bash', 'packaging/package.sh', version], cwd=self.root,
                              env=self.env, capture_output=True, text=True)

    def test_packages_reported_artifact_and_explicit_host_target(self):
        result = self.package('0.2.0-rc.1+test')
        self.assertEqual(result.returncode, 0, result.stderr)
        name = 'lenz-0.2.0-rc.1+test-x86_64-test-linux-gnu'
        with tarfile.open(self.root / 'target/package' / (name + '.tar.gz')) as archive:
            for installed, executable in SHIPPED:
                self.assertEqual(archive.extractfile(f'{name}/{installed}').read(), b'fresh ' + executable.encode())
                self.assertTrue(archive.getmember(f'{name}/{installed}').mode & 0o111, installed)
            self.assertFalse([n for n in archive.getnames() if n.endswith('/lenz-tui')], 'no old terminal name')
            self.assertEqual(archive.extractfile(name + '/LICENSE-THEMES').read(), b'LICENSE-THEMES')
            for desktop in ['lenz.desktop', 'lenz-diff.desktop']:
                self.assertEqual(archive.extractfile(f'{name}/{desktop}').read(), f'packaging/{desktop}'.encode())
        calls = json.loads((self.root / 'calls.json').read_text())
        self.assertEqual([args[args.index('-p') + 1] for args in calls],
                         ['lenz-launcher', 'reader-gui', 'reader-tui', 'lenz', 'lenz-tui'], 'built separately')
        for args in calls:
            self.assertEqual(args[args.index('--target') + 1], 'x86_64-test-linux-gnu')

    def test_archives_hide_the_builder_and_binaries_with_their_home_are_refused(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.root / 'target/package/lenz-0.2.0-x86_64-test-linux-gnu.tar.gz') as archive:
            self.assertTrue(all(m.uid == 0 and m.gid == 0 for m in archive.getmembers()), 'files owned by root')
            self.assertFalse([n for n in archive.getnames() if '/._' in n])
        calls = json.loads((self.root / 'calls.json').read_text())
        self.assertTrue(calls, 'cargo ran')
        # a binary that still names the builder's home (an unmapped source path) is not packaged
        self.env['HOME'] = str(self.root / 'home')
        self.fake('cargo', (self.tools / 'cargo').read_text().split('\n', 1)[1].replace(
            "b'fresh ' + name.encode()", "('fresh ' + os.environ['HOME'] + '/src/lib.rs').encode()"))
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('nothing packaged', result.stderr)
        self.assertIn(str(self.root / 'home/src/lib.rs'), result.stderr, 'the offending path is named')
        # both compilers were told to map the checkout and Cargo's home
        flags = json.loads((self.root / 'flags.json').read_text())
        self.assertIn(f'--remap-path-prefix={self.root.resolve()}=/lenz', flags['RUSTFLAGS'])
        self.assertIn('=/cargo', flags['RUSTFLAGS'])
        for var in ('CFLAGS', 'CXXFLAGS'):
            self.assertIn(f'-ffile-prefix-map={self.root.resolve()}=/lenz', flags[var])
        self.assertFalse((self.root / 'target/package/lenz-0.2.0-x86_64-test-linux-gnu.tar.gz').exists())

    def test_linux_package_has_no_app_bundle(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        with tarfile.open(self.root / 'target/package/lenz-0.2.0-x86_64-test-linux-gnu.tar.gz') as archive:
            self.assertFalse([n for n in archive.getnames() if 'Lenz.app' in n])
            self.assertTrue(archive.getmember('lenz-0.2.0-x86_64-test-linux-gnu/lenz').isfile())

    def test_macos_package_bundles_the_gui_with_its_icon(self):
        # The Dock shows a bare executable's generic icon at launch and quit; a bundle carries the logo.
        self.fake('rustc', "print('host: aarch64-apple-darwin')")
        self.fake('codesign', "import os, pathlib, sys; pathlib.Path(os.environ['LENZ_TEST_CALLS']).with_name('codesign.txt').write_text(' '.join(sys.argv[1:]))")
        self.artifact = self.root / 'custom target/aarch64-apple-darwin/dist/lenz'
        self.env['LENZ_TEST_ARTIFACT'] = str(self.artifact)
        result = self.package('0.2.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        name = 'lenz-0.2.0-aarch64-apple-darwin'
        with tarfile.open(self.root / 'target/package' / (name + '.tar.gz')) as archive:
            app = name + '/Lenz.app/Contents/'
            for installed, executable in SHIPPED:
                self.assertEqual(archive.extractfile(app + 'MacOS/' + installed).read(), b'fresh ' + executable.encode())
                self.assertTrue(archive.getmember(app + 'MacOS/' + installed).mode & 0o111, 'executable')
                link = archive.getmember(f'{name}/{installed}')
                self.assertTrue(link.issym(), f'the top-level {installed} runs through the bundle')
                self.assertEqual(link.linkname, 'Lenz.app/Contents/MacOS/' + installed)
            self.assertEqual(archive.extractfile(app + 'Resources/lenz.icns').read(), b'assets/lenz.icns')
            self.assertEqual(archive.extractfile(name + '/LICENSE-THEMES').read(), b'LICENSE-THEMES')
            import plistlib
            info = plistlib.loads(archive.extractfile(app + 'Info.plist').read())
            self.assertEqual((info['CFBundleExecutable'], info['CFBundleIconFile'], info['CFBundlePackageType']), ('lenz', 'lenz', 'APPL'))
            self.assertEqual((info['CFBundleName'], info['CFBundleShortVersionString']), ('Lenz', '0.2.0'))
        signed = (self.root / 'codesign.txt').read_text()
        self.assertTrue(signed.startswith('--force --sign -') and signed.endswith('/Lenz.app'), signed)

    def test_missing_artifact_does_not_package_stale_binary(self):
        self.env['LENZ_TEST_SKIP_ARTIFACT'] = '1'
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / 'target/package/lenz-0.2.0-x86_64-test-linux-gnu.tar.gz').exists())

    def test_aur_installs_the_launcher_its_apps_desktop_entries_and_notices(self):
        archive = self.root / 'lenz-0.2.0-x86_64-unknown-linux-gnu'
        archive.mkdir()
        for name in [installed for installed, _ in SHIPPED] + ['lenz.desktop', 'lenz-diff.desktop', 'lenz-icon.png',
                                                             'README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE',
                                                             'LICENSE-THEMES']:
            (archive / name).write_text(name)
        recipe = Path(__file__).resolve().parents[1] / 'packaging/aur/PKGBUILD'
        installed = self.root / 'installed'
        result = subprocess.run(['bash', '-c', 'source "$1"; pkgver=0.2.0; CARCH=x86_64; pkgdir="$2"; package',
                                 'package-test', str(recipe), str(installed)],
                                cwd=self.root, env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        for name, _ in SHIPPED:
            path = installed / 'usr/bin' / name
            self.assertEqual(path.read_text(), name)
            self.assertTrue(path.stat().st_mode & 0o111, name)
        for desktop in ['lenz.desktop', 'lenz-diff.desktop']:
            self.assertEqual((installed / 'usr/share/applications' / desktop).read_text(), desktop)
        self.assertEqual((installed / 'usr/share/licenses/lenz/LICENSE-THEMES').read_text(), 'LICENSE-THEMES')

    def test_rejects_version_path_before_build_or_removal(self):
        marker = self.root / 'target/keep'
        marker.mkdir()
        for version in ['../../keep', '/tmp/escape', '.', '..', '0.2.0/../../keep', '0.2.0 with space']:
            with self.subTest(version=version):
                self.assertNotEqual(self.package(version).returncode, 0)
                self.assertFalse((self.root / 'calls.json').exists())
                self.assertTrue(marker.is_dir())


if __name__ == '__main__':
    unittest.main()

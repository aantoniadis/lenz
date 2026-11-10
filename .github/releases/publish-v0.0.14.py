"""Publish only the exact, successful, checksum-verified beta build."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import tarfile

ROOT = Path(__file__).resolve().parent
EXPECTED = json.loads((ROOT / '0.0.14-manifest.json').read_text())
REPO = 'aantoniadis/lenz'
TAG = 'v0.0.14'


def api(path, payload=None):
    command = ['gh', 'api', f'repos/{REPO}/{path}']
    if payload is not None:
        command += ['--method', 'PATCH', '--input', '-']
    result = subprocess.run(command, input=json.dumps(payload) if payload is not None else None,
                            capture_output=True, text=True, check=True)
    return json.loads(result.stdout)


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b''):
            result.update(chunk)
    return 'sha256:' + result.hexdigest()


def assets(release):
    assert all(asset['state'] == 'uploaded' for asset in release['assets'])
    result = {asset['name']: asset['digest'] for asset in release['assets']}
    assert len(result) == len(release['assets']) == 6
    return result


assert os.environ['GITHUB_REF'] == 'refs/heads/automation/publish-v0.0.14'
run = api(f"actions/runs/{EXPECTED['build_run']}")
assert run['conclusion'] == 'success' and run['head_sha'] == EXPECTED['commit']
assert run['path'] == '.github/workflows/release.yml'
jobs = api(f"actions/runs/{EXPECTED['build_run']}/jobs?filter=all&per_page=100")['jobs']
for runner in ['ubuntu-22.04', 'ubuntu-22.04-arm', 'macos-26']:
    job = max((job for job in jobs if job['name'] == f'package ({runner})'), key=lambda job: job['id'])
    assert job['conclusion'] == 'success'
    smoke = next(step for step in job['steps'] if step['name'] == 'Verify the unpacked package and launcher routing')
    assert smoke['conclusion'] == 'success'
for previous in EXPECTED['superseded_runs']:
    assert api(f'actions/runs/{previous}')['status'] == 'completed'
tag = api(f'git/ref/tags/{TAG}')['object']
assert tag['type'] == 'tag'
annotated = api(f"git/tags/{tag['sha']}")
assert annotated['tag'] == TAG
assert annotated['object']['type'] == 'commit' and annotated['object']['sha'] == EXPECTED['commit']
release = api(f"releases/{EXPECTED['release_id']}")
assert release['tag_name'] == TAG and assets(release) == EXPECTED['assets']
notes_path = ROOT / '0.0.14.md'
assert digest(notes_path) == EXPECTED['notes_digest']
notes = notes_path.read_text()
with tempfile.TemporaryDirectory(prefix='lenz-release-verify-') as temporary:
    destination = Path(temporary)
    subprocess.run(['gh', 'release', 'download', TAG, '--repo', REPO, '--dir', temporary], check=True)
    assert {path.name for path in destination.iterdir()} == set(EXPECTED['assets'])
    for name, expected in EXPECTED['assets'].items():
        assert digest(destination / name) == expected, name
        if name.endswith('.sha256'):
            checksum, filename = (destination / name).read_text().strip().split()
            assert filename == name.removesuffix('.sha256')
            assert 'sha256:' + checksum == EXPECTED['assets'][filename]
            print('CHECKSUM_FILE ' + json.dumps({'name': name, 'content': (destination / name).read_text()}), flush=True)
    build_artifacts = destination / 'ci-artifacts'
    subprocess.run(['gh', 'run', 'download', str(EXPECTED['build_run']), '--repo', REPO,
                    '--dir', str(build_artifacts)], check=True)
    build_files = [path for path in build_artifacts.rglob('*') if path.is_file()]
    assert len(build_files) == 6
    assert {path.name: digest(path) for path in build_files} == EXPECTED['assets']
    print('Verified all six release downloads against the final workflow artifacts and checksum sidecars', flush=True)
    workspace = ROOT.parent.parent
    logs = Path(os.environ['RUNNER_TEMP']) / 'lenz-package-validation'
    logs.mkdir()
    native = destination / 'native'
    native.mkdir()
    with tarfile.open(destination / 'lenz-0.0.14-x86_64-unknown-linux-gnu.tar.gz') as archive:
        archive.extractall(native, filter='data')
    package = native / 'lenz-0.0.14-x86_64-unknown-linux-gnu'

    def check(name, command):
        with (logs / (name + '.log')).open('w') as output:
            result = subprocess.run(command, stdout=output, stderr=subprocess.STDOUT, timeout=180)
        print((logs / (name + '.log')).read_text(), flush=True)
        assert result.returncode == 0, name

    check('package-cli', [sys.executable, str(workspace / 'packaging/check-package.py'), str(package), '0.0.14'])
    for name in ['lenz', 'lenz-code', 'lenz-code-tui', 'lenz-diff', 'lenz-diff-tui']:
        version = subprocess.check_output([str(package / name), '--version'], text=True)
        assert EXPECTED['commit'][:7] in version, version
    fixture = destination / 'fixtures'
    fixture.mkdir()
    left = fixture / 'left.rs'
    right = fixture / 'right.rs'
    left.write_text(''.join(f'let value_{i} = {i};\n' for i in range(2000)))
    right.write_text(left.read_text().replace('let value_100 = 100;', 'let value_100 = 101;'))
    check('diff-gui', [sys.executable, str(workspace / 'scripts/startup-bench.py'),
                      '--binary', str(package / 'lenz-diff'), '--runs', '1', '--seconds', '1', '--', str(left), str(right)])
    check('code-gui', [sys.executable, str(workspace / 'scripts/code-editor-gui-bench.py'),
                      '--binary', str(package / 'lenz-code'), '--source', EXPECTED['commit'],
                      '--rounds', '1', '--out', str(logs / 'code-gui')])
    check('diff-tui', [sys.executable, str(workspace / 'scripts/tui-bench.py'),
                      '--app', str(package / 'lenz-diff-tui'), '--lines', '1000', '--rounds', '3'])
    check('code-tui', [sys.executable, str(workspace / 'scripts/code-tui-bench.py'),
                      '--binary', str(package / 'lenz-code-tui'), '--source', EXPECTED['commit'],
                      '--lines', '1000', '--rounds', '3', '--stress-seconds', '1', '--out', str(logs / 'code-tui')])
    print('Verified exact embedded source IDs and packaged Linux GUI/TUI launches and editing', flush=True)
if release['draft']:
    release = api(f"releases/{EXPECTED['release_id']}", {
        'name': 'Lenz 0.0.14', 'body': notes, 'draft': False,
        'prerelease': True, 'make_latest': 'false',
    })
assert not release['draft'] and release['prerelease'] and release['published_at']
assert release['body'] == notes and assets(release) == EXPECTED['assets']
print(json.dumps({key: release[key] for key in ['id', 'tag_name', 'html_url', 'draft', 'prerelease', 'published_at']}, indent=2))

import hashlib,json,subprocess,time
from pathlib import Path
ROOT=Path(__file__).resolve().parent
COMMIT='1ddeb8ed94cb415e0c27b00f696578bdf995e6f7'
REPO='aantoniadis/lenz'
def api(path):
 return json.loads(subprocess.check_output(['gh','api',f'repos/{REPO}/{path}'],text=True))
for attempt in range(120):
 runs=api(f'actions/workflows/release.yml/runs?head_sha={COMMIT}&event=push&per_page=20')['workflow_runs']
 if runs:
  run=max(runs,key=lambda x:x['id'])
  assert run['head_sha']==COMMIT
  print('BUILD_STATUS',run['id'],run['status'],run['conclusion'],flush=True)
  if run['status']=='completed':
   assert run['conclusion']=='success',run['conclusion']
   break
 time.sleep(30)
else: raise RuntimeError('Native release builds did not finish in one hour')
r=api('releases/tags/v0.0.15')
names={f'lenz-0.0.15-{t}.tar.gz{s}' for t in ['x86_64-unknown-linux-gnu','aarch64-unknown-linux-gnu','aarch64-apple-darwin'] for s in ['', '.sha256']}
assert len(r['assets'])==6 and {a['name'] for a in r['assets']}==names
assets={a['name']:a['digest'] for a in r['assets']}
assert all(isinstance(v,str) and v.startswith('sha256:') and len(v)==71 for v in assets.values())
expected=dict(commit=COMMIT,build_run=run['id'],release_id=r['id'],assets=assets,superseded_runs=[],notes_digest='sha256:'+hashlib.sha256((ROOT/'0.0.15.md').read_bytes()).hexdigest())
(ROOT/'0.0.15-manifest.json').write_text(json.dumps(expected,indent=2))
print('VERIFIED_MANIFEST',json.dumps(expected),flush=True)

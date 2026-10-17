#!/usr/bin/env bash
# Local experiment: build the real apps, then stage the proposed command family together.
set -euo pipefail
cd "$(dirname "$0")/.."
artifacts=$(mktemp)
trap 'rm -f "$artifacts"' EXIT
cargo build --release --locked -p lenz-launcher -p lenz -p reader-gui --message-format=json-render-diagnostics > "$artifacts"
# Build independently so the terminal app's dependency graph excludes GUI features.
cargo build --release --locked -p lenz-tui --message-format=json-render-diagnostics >> "$artifacts"
python3 - "$artifacts" <<'PY'
import json
import os
from pathlib import Path
import shutil
import sys

binaries = {}
for line in Path(sys.argv[1]).read_text().splitlines():
    row = json.loads(line)
    if row.get('reason') == 'compiler-artifact' and 'bin' in row.get('target', {}).get('kind', []) and row.get('executable'):
        binaries[row['target']['name']] = Path(row['executable']).resolve()
names = {'lenz': 'lenz-launcher', 'lenz-code': 'lenz-reader', 'lenz-diff': 'lenz', 'lenz-diff-tui': 'lenz-tui'}
for binary in names.values():
    if binary not in binaries or not binaries[binary].is_file():
        sys.exit(f'Missing {binary}; nothing staged')
out = binaries['lenz-launcher'].parent.parent / 'launcher-trial'
out.mkdir(exist_ok=True)
for name, binary in names.items():
    target = out / name
    target.unlink(missing_ok=True)
    if name == 'lenz':
        shutil.copy2(binaries[binary], target)
    else:
        target.symlink_to(os.path.relpath(binaries[binary], out))
print(f'\nTrial ready: {out}')
print(f'  {out}/lenz path/to/source.rs')
print(f'  {out}/lenz diff left right')
print(f'  {out}/lenz diff --tui left right')
PY

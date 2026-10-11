#!/usr/bin/env bash
# Build a release tarball for this machine's target into target/package/ (nothing is uploaded):
#   packaging/package.sh 0.2.0
# Produces lenz-<version>-<target>.tar.gz (lenz, lenz-tui, README, licences) and its .sha256. On macOS the GUI
# is inside Lenz.app, so the Dock shows the logo from launch to quit, and `lenz` is a symlink into it.
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: packaging/package.sh <version>}
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z][0-9A-Za-z.+-]*)?$ ]]; then
    echo 'version must be a numeric major.minor.patch with an optional prerelease/build suffix' >&2
    exit 1
fi
target=$(rustc -vV | sed -n 's/^host: //p')
test -n "$target"
artifact_json=$(mktemp)
trap 'rm -f "$artifact_json"' EXIT
# Packaging is host-only; Cargo's reported path also respects CARGO_TARGET_DIR. Each package builds on its own so
# the TUI does not pick up the GUI's features through Cargo feature unification.
build() {
    LENZ_BUILD_ID="$version ($(git rev-parse --short HEAD))" cargo build --profile dist --locked -p "$1" \
        --target "$target" --message-format=json-render-diagnostics > "$artifact_json"
    python3 - "$artifact_json" "$1" <<'PY'
import json
from pathlib import Path
import sys

name = sys.argv[2]
records = [json.loads(line) for line in Path(sys.argv[1]).read_text().splitlines() if line.strip()]
executables = {record['executable'] for record in records
               if record.get('reason') == 'compiler-artifact'
               and record.get('target', {}).get('name') == name
               and 'bin' in record.get('target', {}).get('kind', [])
               and record.get('executable')}
if len(executables) != 1:
    sys.exit(f'Cargo did not report exactly one {name} executable; nothing packaged')
binary = Path(executables.pop()).resolve()
if not binary.is_file():
    sys.exit(f'Cargo reported a missing {name} executable; nothing packaged')
print(binary)
PY
}
gui=$(build lenz)
tui=$(build lenz-tui)
name="lenz-$version-$target"
out=target/package
rm -rf "$out/$name"
mkdir -p "$out/$name"
if [[ $target == *-apple-darwin ]]; then
    app="$out/$name/Lenz.app/Contents"
    mkdir -p "$app/MacOS" "$app/Resources"
    cp "$gui" "$app/MacOS/lenz"
    cp assets/lenz.icns "$app/Resources/lenz.icns"
    cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>lenz</string>
    <key>CFBundleIconFile</key><string>lenz</string>
    <key>CFBundleIdentifier</key><string>io.github.aantoniadis.lenz</string>
    <key>CFBundleName</key><string>Lenz</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
    # The linker's ad-hoc signature covers the bare executable, not the bundle around it; sign the bundle.
    codesign --force --sign - "$out/$name/Lenz.app"
    ln -s Lenz.app/Contents/MacOS/lenz "$out/$name/lenz"
else
    cp "$gui" "$out/$name/lenz"
fi
cp "$tui" "$out/$name/lenz-tui"
cp README.md LICENSE-MIT "$out/$name/"
cp crates/engine/LICENSE "$out/$name/LICENSE-APACHE"
cp crates/engine/NOTICE "$out/$name/NOTICE"
cp packaging/lenz.desktop assets/lenz-icon.png "$out/$name/"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
# macOS ships shasum; sha256sum only on recent releases.
(cd "$out" && if command -v sha256sum >/dev/null; then sha256sum "$name.tar.gz"; else shasum -a 256 "$name.tar.gz"; fi > "$name.tar.gz.sha256")
echo "$out/$name.tar.gz"
cat "$out/$name.tar.gz.sha256"

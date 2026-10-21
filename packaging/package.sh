#!/usr/bin/env bash
# Build a release tarball for this machine's target into target/package/ (nothing is uploaded):
#   packaging/package.sh 0.2.0
# Produces lenz-<version>-<target>.tar.gz (the lenz launcher and its four apps, README, licences) and its .sha256.
# On macOS the executables are inside Lenz.app, so the Dock shows the logo from launch to quit, with a top-level
# symlink into it for each.
set -euo pipefail
cd "$(dirname "$0")/.."
version=${1:?usage: packaging/package.sh <version>}
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z][0-9A-Za-z.+-]*)?$ ]]; then
    echo 'version must be a numeric major.minor.patch with an optional prerelease/build suffix' >&2
    exit 1
fi
target=$(rustc -vV | sed -n 's/^host: //p')
test -n "$target"
# A failed build must not leave an older tarball looking like this one.
rm -f "target/package/lenz-$version-$target.tar.gz" "target/package/lenz-$version-$target.tar.gz.sha256"
# gpui compiles Metal shaders; the Command Line Tools have no `metal`, so fall back to an installed Xcode (as ci-local.sh).
if [[ $(uname) == Darwin && -z ${DEVELOPER_DIR:-} ]] && ! xcrun -find metal > /dev/null 2>&1 \
        && [[ -d /Applications/Xcode.app/Contents/Developer ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi
artifact_json=$(mktemp)
trap 'rm -f "$artifact_json"' EXIT
# Packaging is host-only; Cargo's reported path also respects CARGO_TARGET_DIR. Each package builds on its own so
# the TUI does not pick up the GUI's features through Cargo feature unification.
# build <package> <executable>
build() {
    LENZ_BUILD_ID="$version ($(git rev-parse --short HEAD))" cargo build --profile dist --locked -p "$1" \
        --target "$target" --message-format=json-render-diagnostics > "$artifact_json"
    python3 - "$artifact_json" "$2" <<'PY'
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
# Source paths end up in the binaries: Rust's panic locations, and C's __FILE__ in the assert messages of
# tree-sitter and its grammars. Map the checkout and Cargo's home to fixed prefixes in both compilers, so no
# builder's home directory (and user name) ships. Both flag variables split on whitespace.
# With rustup's rust-src installed, rustc maps std's /rustc/<hash> paths back to the local toolchain; map that too.
checkout=$(pwd -P)
cargo_home=${CARGO_HOME:-$HOME/.cargo}
sysroot=$(rustc --print sysroot)
case "$checkout$cargo_home$sysroot" in *[[:space:]]*) echo "build paths must not contain whitespace: $checkout, $cargo_home, $sysroot" >&2; exit 1;; esac
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$checkout=/lenz --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rustc"
cmap="-ffile-prefix-map=$checkout=/lenz -ffile-prefix-map=$cargo_home=/cargo"
export CFLAGS="${CFLAGS:+$CFLAGS }$cmap" CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }$cmap"
# The installed name of each executable, and the package and Cargo executable it comes from.
names=(lenz lenz-code lenz-code-tui lenz-diff lenz-diff-tui)
packages=(lenz-launcher reader-gui reader-tui lenz lenz-tui)
executables=(lenz-launcher lenz-reader lenz-reader-tui lenz lenz-tui)
built=()
for i in "${!names[@]}"; do
    built+=("$(build "${packages[$i]}" "${executables[$i]}")")
done
for binary in "${built[@]}"; do
    if grep -qaF "$HOME/" "$binary"; then
        echo "$binary still contains $HOME/; nothing packaged:" >&2
        strings -n 8 "$binary" 2>/dev/null | sed -n "s|.*\($HOME/[^\"' ]*\).*|\1|p" | sort | uniq -c | sort -rn | head -10 >&2 || true
        exit 1
    fi
done
name="lenz-$version-$target"
out=target/package
rm -rf "$out/$name"
mkdir -p "$out/$name"
if [[ $target == *-apple-darwin ]]; then
    app="$out/$name/Lenz.app/Contents"
    mkdir -p "$app/MacOS" "$app/Resources"
    for i in "${!names[@]}"; do
        cp "${built[$i]}" "$app/MacOS/${names[$i]}"
    done
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
    for executable in "${names[@]}"; do
        ln -s "Lenz.app/Contents/MacOS/$executable" "$out/$name/$executable"
    done
else
    for i in "${!names[@]}"; do
        cp "${built[$i]}" "$out/$name/${names[$i]}"
    done
fi
cp README.md LICENSE-MIT LICENSE-THEMES "$out/$name/"
cp crates/engine/LICENSE "$out/$name/LICENSE-APACHE"
cp crates/engine/NOTICE "$out/$name/NOTICE"
cp packaging/lenz.desktop packaging/lenz-diff.desktop assets/lenz-icon.png "$out/$name/"
# Files owned by root, without the builder's user or (on macOS) extended attributes and ._ files.
if tar --version 2>/dev/null | grep -q bsdtar; then
    COPYFILE_DISABLE=1 tar --uid 0 --gid 0 --uname root --gname wheel --no-xattrs --no-mac-metadata \
        -C "$out" -czf "$out/$name.tar.gz" "$name"
else
    tar --owner=0 --group=0 --numeric-owner -C "$out" -czf "$out/$name.tar.gz" "$name"
fi
# macOS ships shasum; sha256sum only on recent releases.
(cd "$out" && if command -v sha256sum >/dev/null; then sha256sum "$name.tar.gz"; else shasum -a 256 "$name.tar.gz"; fi > "$name.tar.gz.sha256")
echo "$out/$name.tar.gz"
cat "$out/$name.tar.gz.sha256"

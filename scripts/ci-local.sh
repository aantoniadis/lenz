#!/usr/bin/env bash
# The CI checks, run locally (GitHub Actions is not used: no minutes). The pre-push hook in .githooks runs it.
#   scripts/ci-local.sh            # lint, panic-site ratchet, tests, GUI adapter tests, release build size
#   scripts/ci-local.sh --old-git  # also the core tests against git 2.30 (Debian bullseye) in podman or docker
# Skip once with `git push --no-verify`.
set -euo pipefail
cd "$(dirname "$0")/.."
step() { printf '\n== %s\n' "$*"; }
# gpui compiles Metal shaders; the Command Line Tools have no `metal`, so fall back to an installed Xcode.
if [[ $(uname) == Darwin && -z ${DEVELOPER_DIR:-} ]] && ! xcrun -find metal > /dev/null 2>&1 \
        && [[ -d /Applications/Xcode.app/Contents/Developer ]]; then
    export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
fi

step "clippy, warnings as errors"
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --locked --features lenz/gui-test,reader-tui/gui-test -- -D warnings
step "panic-prone sites"
python3 scripts/panic-sites.py
python3 scripts/panic-sites.py --reader
python3 scripts/panic-sites.py --reader-tui
step "tests"
cargo test --workspace --locked
# They read the whole process's memory, which concurrent tests' allocations skew.
cargo test -p lenz-content --lib --locked -- --ignored --exact --test-threads=1 \
    memory_probe::tests::footprint_counts_live_heap_not_freed_pages \
    highlight::tests::concurrent_parses_return_the_trees_memory_to_the_system \
    highlight::tests::declaration_extraction_returns_the_trees_memory_to_the_system \
    highlight::tests::fold_parses_return_the_trees_memory_to_the_system \
    highlight::tests::selection_steps_return_the_trees_memory_to_the_system \
    syntax_arena::tests::ending_a_scope_returns_touched_pages
step "GUI adapter tests"
cargo test -p lenz --features gui-test --locked gui_
cargo test -p reader-gui --features test-support --locked
cargo test -p reader-tui --features gui-test --locked
step "collector tests"
python3 -m unittest discover -s scripts -p 'test_*.py'
step "standalone terminal release builds (without GUI features)"
cargo build --release --locked -p lenz-tui -p reader-tui
step "release build and stripped size (≤ 80,000,000 bytes)"
cargo build --release --locked -p lenz
stripped=$(mktemp)
trap 'rm -f "$stripped"' EXIT
strip -o "$stripped" target/release/lenz
size=$(wc -c < "$stripped")
echo "stripped lenz: $size bytes"
test "$size" -le 80000000

if [[ ${1:-} == --old-git ]]; then
    step "core tests on git 2.30 (Debian bullseye)"
    engine=$(command -v podman || command -v docker)
    # bullseye is past end of life: use the image's pinned snapshot of the archive. Build output stays in a volume.
    "$engine" run --rm -v "$PWD:/src:ro" -v lenz-old-git-target:/target -e CARGO_TARGET_DIR=/target debian:bullseye bash -euc '
        sed -i -e "s|^deb |# deb |" -e "s|^# deb http://snapshot|deb http://snapshot|" /etc/apt/sources.list
        apt-get -qq -o Acquire::Check-Valid-Until=false update
        apt-get -qq install -y git curl build-essential pkg-config libxkbcommon-x11-dev libwayland-dev \
            libfontconfig-dev libx11-xcb-dev libvulkan1 libglib2.0-dev libzstd-dev > /dev/null
        git --version | grep -F "git version 2.30."
        curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal > /dev/null
        cp -r /src /work && cd /work && "$HOME/.cargo/bin/cargo" test -p lenz --locked'
fi
printf '\nAll local CI checks passed.\n'

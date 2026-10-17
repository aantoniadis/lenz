#!/usr/bin/env bash
# Native macOS acceptance for the reader (ticket 20). Run on a Mac from the repository root, logged in to a desktop
# session (launch and frame-pacing workloads open real windows). Untested until first run on a Mac: anything that
# fails here is native evidence to record, not to work around.
#
#   scripts/code-macos-acceptance.sh <evidence-dir> [work-dir]
#
# Real language servers run only when they are installed and named (see docs/code/macos-acceptance.md):
# READER_RUST_ANALYZER, READER_PYRIGHT(_PATH), READER_TYPESCRIPT(_PATH), READER_TSSERVER, READER_JDTLS(_PATH),
# READER_RUBY_LSP(_PATH), READER_KOTLIN, READER_NODE_PATH. Unset ones are skipped and must be reported as unverified.
set -uo pipefail

OUT=${1:?evidence directory}
WORK=${2:-$(mktemp -d -t reader-macos)}
mkdir -p "$OUT" "$WORK"
OUT=$(cd "$OUT" && pwd)
log() { printf '\n== %s\n' "$*" | tee -a "$OUT/run.log"; }
run() { local name=$1; shift; log "$name: $*"; "$@" >"$OUT/$name.txt" 2>&1; local code=$?; echo "exit $code" >>"$OUT/$name.txt"; echo "$name exit $code" | tee -a "$OUT/run.log"; }

[ "$(uname)" = Darwin ] || { echo "run this on macOS" >&2; exit 2; }

log "identity"
{
    sw_vers
    sysctl -n machdep.cpu.brand_string hw.model hw.ncpu hw.memsize
    system_profiler SPDisplaysDataType 2>/dev/null | grep -E 'Chipset|Metal|Resolution|Refresh|Display Type'
    rustc -Vv
    cargo -V
    echo "reader $(git rev-parse HEAD) $(git status --porcelain | wc -l | tr -d ' ') dirty paths"
    grep -n 'lenz-' Cargo.toml
} >"$OUT/identity.txt" 2>&1

run build-locked cargo build --locked --release --workspace
run core-graph sh -c "cargo tree -p reader-core -e normal | grep -ci gpui; echo '(expected 0 gpui crates in reader-core)'"
run build-probe cargo build --release -p reader-gui --features perf-probe
cp target/release/lenz-reader "$WORK/lenz-reader-probe"
for i in 1 2 3; do run "tests-$i" cargo test --workspace --features reader-gui/test-support; done
run real-providers cargo test --release -p reader-core --test providers -- --ignored --nocapture --test-threads 1 --skip corpus_readiness
run real-rust-analyzer cargo test --release -p reader-core --test semantics real_rust_analyzer -- --ignored --nocapture
run real-gui cargo test -p reader-gui --features test-support real_ -- --ignored --nocapture --test-threads 1

log "corpora (pinned, disposable)"
if [ ! -d "$WORK/lenz" ]; then
    git clone -q https://github.com/aantoniadis/lenz "$WORK/lenz" && git -C "$WORK/lenz" checkout -q 12b682bacf17adf6e57c1f569378aba12b870b50
fi
if [ ! -d "$WORK/metabase" ]; then
    mkdir -p "$WORK/metabase" && git -C "$WORK/metabase" init -q && git -C "$WORK/metabase" remote add origin https://github.com/metabase/metabase \
        && git -C "$WORK/metabase" fetch -q --depth 1 origin c0238e60240ccb855abacbb225a9c3ac425acb59 && git -C "$WORK/metabase" checkout -q FETCH_HEAD
fi
git -C "$WORK/lenz" log -1 --format='lenz %H' >>"$OUT/identity.txt"
git -C "$WORK/metabase" log -1 --format='metabase %H' >>"$OUT/identity.txt"

READER_CORPUS="$WORK/lenz" READER_CORPUS_NAME=lenz READER_SEARCH_TARGET_MS=2000 run acceptance-lenz cargo test --release -p reader-core --lib acceptance_perf -- --ignored --nocapture
READER_CORPUS="$WORK/metabase" READER_CORPUS_NAME=metabase READER_SEARCH_TARGET_MS=5000 run acceptance-metabase cargo test --release -p reader-core --lib acceptance_perf -- --ignored --nocapture
READER_CORPUS="$WORK/lenz" run rust-corpus-readiness cargo test --release -p reader-core --test providers real_rust_corpus_readiness -- --ignored --nocapture
READER_CORPUS="$WORK/metabase" run typescript-corpus-readiness cargo test --release -p reader-core --test providers real_typescript_corpus_readiness -- --ignored --nocapture

log "launch and memory: real windows open on this display; do not use the Mac meanwhile"
run launch-bench python3 scripts/code-launch-bench.py --native --binary "$WORK/lenz-reader-probe" --file "$WORK/lenz/crates/engine/src/lib.rs" --corpus "$WORK/lenz" --out "$OUT"

log "frame pacing: a window scrolls a large file for 20 s while Metabase is indexed"
READER_PROBE_ROOT="$WORK/metabase" READER_PROBE_SCROLL=20 "$WORK/lenz-reader-probe" "$WORK/metabase/test/metabase/warehouses_rest/api_test.clj" >"$OUT/native-scroll.txt" 2>&1
echo "native-scroll exit $?" | tee -a "$OUT/run.log"

log "15-minute stress (core process, physical footprint)"
READER_CORPUS="$WORK/metabase" READER_STRESS_MINUTES=15 run stress-metabase-15min cargo test --release -p reader-core --lib stress_perf -- --ignored --nocapture

log "summary"
grep -h 'ACCEPTANCE' "$OUT"/acceptance-*.txt | tee -a "$OUT/run.log"
grep -h -E 'launch to readable|ten tabs' "$OUT/launch-bench.txt" | tee -a "$OUT/run.log"
grep -h -E 'scrolled|slow' "$OUT/native-scroll.txt" | tee -a "$OUT/run.log"
grep -h -E 'memory:|stress over|first three|test result' "$OUT/stress-metabase-15min.txt" | tee -a "$OUT/run.log"
grep -h -E '^test |test result' "$OUT/real-providers.txt" "$OUT/real-gui.txt" | tee -a "$OUT/run.log"
echo "Evidence in $OUT; complete the manual checklist in docs/code/macos-acceptance.md."

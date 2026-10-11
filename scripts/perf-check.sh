#!/usr/bin/env bash
# Performance gate: the engine's release-mode benches on generated fixed inputs (budgets: BUDGETS in
# crates/engine/benches/engine.rs), the view model's, the Merge View's and the commit log's timing tests (budgets in
# view.rs, ui/merge_view.rs and history.rs); exits non-zero when a
# median exceeds 2x its budget.
# Takes ~20 s after the build.
set -euo pipefail
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset LENZ_PERF_REPORT_ONLY
cd "$(dirname "$0")/.."

cpu=$(grep -m1 'model name' /proc/cpuinfo 2>/dev/null | cut -d: -f2- || sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)
echo "machine:${cpu:- unknown}, $(getconf _NPROCESSORS_ONLN) threads"
cargo bench --locked -p lenz-engine --bench engine -- --check
cargo test --locked --release -p lenz view_model_perf -- --ignored --nocapture
cargo test --locked --release -p lenz merge_view_perf -- --ignored --nocapture
cargo test --locked --release -p lenz highlight::tests::perf -- --ignored --nocapture
cargo test --locked --release -p lenz search_perf -- --ignored --nocapture
cargo test --locked --release -p lenz image_perf -- --ignored --nocapture
exec cargo test --locked --release -p lenz layout_a_million -- --ignored --nocapture

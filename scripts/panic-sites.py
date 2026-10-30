#!/usr/bin/env python3
"""Hold panic-prone sites in production code to a baseline: the counts may fall, never rise.

    python3 scripts/panic-sites.py            # check against scripts/panic-sites.txt (CI)
    python3 scripts/panic-sites.py --update   # write the current counts as the new baseline

Counts clippy's `unwrap_used`, `expect_used`, `panic`, `unreachable` and `string_slice` over production
libraries and binaries (tests excluded), each site once. Linux only: counts include platform code.
The default Diff budget covers its shared Content, GPUI and Engine dependencies. --code owns only
code-core and code-gui sites; --code-tui owns only terminal-adapter sites and takes precedence.
Shared diagnostics remain in the default budget rather than being counted again as Lenz Code sites.
"""
import json, pathlib, subprocess, sys

LINTS = ["unwrap_used", "expect_used", "panic", "unreachable", "string_slice"]
BASELINE = pathlib.Path(__file__).with_name("panic-sites.txt")

def current(code=False, code_tui=False) -> dict[str, int]:
    packages = ["--workspace", "--exclude", "code-core", "--exclude", "code-gui", "--exclude", "code-tui"]
    if code_tui:
        packages = ["-p", "code-tui"]
    elif code:
        packages = ["-p", "code-core", "-p", "code-gui"]
    targets = ["--bins"] if code_tui else ["--lib", "--bins"]
    cmd = ["cargo", "clippy", "--locked", *packages, *targets, "--message-format=json", "--"]
    cmd += [f"-Wclippy::{lint}" for lint in LINTS]
    out = subprocess.run(cmd, capture_output=True, text=True, check=True).stdout
    sites = {lint: set() for lint in LINTS}
    for line in out.splitlines():
        msg = json.loads(line).get("message") or {}
        lint = ((msg.get("code") or {}).get("code") or "").removeprefix("clippy::")
        if lint in sites:
            span = next(s for s in msg["spans"] if s["is_primary"])
            if code_tui:
                if not span["file_name"].startswith("crates/code-tui/"):
                    continue
            elif code and not span["file_name"].startswith(("crates/code-core/", "crates/code-gui/")):
                continue  # Content, GPUI and Engine remain enforced by the Diff budget.
            sites[lint].add((span["file_name"], span["line_start"], span["column_start"]))
    return {lint: len(found) for lint, found in sites.items()}

def main() -> int:
    code = "--code" in sys.argv
    code_tui = "--code-tui" in sys.argv
    suffix = "-code-tui" if code_tui else "-code" if code else ""
    baseline_path = BASELINE.with_name(f"panic-sites{suffix}.txt")
    counts = current(code, code_tui)
    if "--update" in sys.argv:
        baseline_path.write_text("".join(f"{lint} {n}\n" for lint, n in counts.items()))
        print(f"wrote {baseline_path}")
        return 0
    baseline = {k: int(v) for k, v in (line.split() for line in baseline_path.read_text().splitlines() if line.strip())}
    failed = False
    for lint, n in counts.items():
        allowed = baseline.get(lint, 0)
        status = "over" if n > allowed else "ok"
        failed |= n > allowed
        print(f"{lint:14} {n:4} (baseline {allowed}) {status}")
    if failed:
        print("New panic-prone sites: handle the error, or prove the invariant in a comment and raise the baseline.")
    elif counts != baseline:
        print("Counts fell: lower the baseline with --update.")
    return 1 if failed else 0

if __name__ == "__main__":
    sys.exit(main())

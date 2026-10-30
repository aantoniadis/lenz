# 60: Choose and persist a visible SQL dialect

**What to build:** Set a project default or per-file/standalone PostgreSQL or DuckDB dialect and see when none is selected.

**Blocked by:** 25: Respect project editing and formatting preferences.

**Status:** resolved

- [x] Persist default/override choices, keep unset visible and expose the selection through both adapters.
- [x] Changing selection invalidates earlier dialect requests/results and updates action availability immediately.
- [x] Opening a file or selecting a dialect initiates no metadata connection; missing dialect adapters show an honest status.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 90, 91, 92, 97, 101.

## Answer

Merged in e145ca8 (branch work/editor-60-sql-dialect, feature 76f01df, integration-merge 770f6a5; clean). `crates/code-core/src/sql_dialect.rs` holds `Dialect`, `Selection {dialect, origin}` and the per-dialect assistance table. Settings persist `projects.<root>.sql_dialect` and `sql_dialects.<path>`; unset is absence and renders as `SQL: not set` in both footers. `Session::sql_identity()` = dialect + generation + `SnapshotKey`; a switch, project-default change, file switch or edit rejects earlier identities while a same-value choice keeps them. Seven centrally defined SQL commands (menu/palette, no preset chords) with disabled reasons; no metadata connection path exists, and details say so. Core 622, GUI 187, TUI 135/163, clippy clean, panic-site ratchets at baseline. Limits: assistance unwired until 61/62, untitled/comparison/retrieved sources cannot hold a file dialect — see docs/editor-follow-ups.md. Performance deferred to the end-of-epic campaign.

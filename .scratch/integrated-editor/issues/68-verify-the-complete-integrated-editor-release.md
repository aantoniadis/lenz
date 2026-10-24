# 68: Verify the complete integrated editor release

**What to build:** Demonstrate the whole agreed editor experience and publish its verified capabilities, limits and performance evidence.

**Blocked by:** 24: Enable and verify native macOS editor closure; 26: Offer IntelliJ and VS Code keymaps with overrides; 28: Find and replace within the current draft; 33: Restore the project session around recovered work; 34: Reconcile an external file change against a dirty draft; 37: Delete recoverably and confirm permanent removal; 45: Deliver Kotlin and mixed JVM project assistance; 46: Deliver Rust editing assistance; 47: Deliver Go editing assistance; 48: Deliver TypeScript, JavaScript and TSX assistance; 52: Deliver Python environment-aware assistance; 53: Deliver schema-aware YAML editing; 54: Deliver Bash and sh editing assistance; 55: Treat environment files as editable data; 56: Edit Markdown with live preview and authoring help; 58: Deliver first-class ERB editing; 59: Deliver first-class Haml editing; 63: Use an explicit PostgreSQL metadata connection; 64: Use an explicit DuckDB metadata file; 67: Inspect and revert marked changes in the shared draft.

**Status:** ready-for-agent

- [ ] All required feature and native-platform tickets are resolved with real evidence; no missing template/SQL capability is silently downgraded to highlighting or generic suggestions.
- [ ] Pass full shared-core/real-GUI/TUI parity, existing Diff/repository/PR/merge regressions, safe-save/recovery/native-quit and actual provider/SQL-boundary scenarios.
- [ ] Repeat equivalent release startup/interaction/child-resource/memory and sustained editing/completion workloads, fix regressions or obtain explicit agreement for deliberate tradeoffs, and report unverified platforms/physical-latency endpoints.

**Spec coverage:** user stories 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 114, 115, 116, 117, 118.

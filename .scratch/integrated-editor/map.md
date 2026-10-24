# Integrated editor implementation

Integration branch: `integration/integrated-editor`.

Parent: [Integrated Lenz Reader and Editor](spec.md). Work is governed by the approved dependency graph in the individual ticket files.

## Notes

The main integration worktree owns tracker state. Implementers work on separate branches/worktrees, merge the latest integration tip before reporting, and provide red/green and regression evidence. Tickets resolve only after their work is merged and verified.

## Decisions so far

- Shared Session/real GUI/TUI seams and focused write/provider/performance checks were confirmed by the user.
- The user confirmed that no native macOS runner is available. Native termination and editor acceptance remain unresolved until required native evidence exists; other work proceeds.

## Fog

Native termination, automatic template/source mapping and complete offline PostgreSQL/DuckDB routes remain required capabilities. A missing tool or environment is not a passing test.

- [Ticket 02](issues/02-share-exact-ending-writes-with-explicit-save-outcomes.md) resolved after writer merge `ebc9354` and full integration gate at `177c167`; exact Reader write outcomes and target identity are available, with existing Diff behavior preserved.

- [Ticket01](issues/01-expand-the-reader-text-snapshot-boundary.md) resolved after snapshot merge/follow-up `177c167` and full integration gate; public indexed TextSnapshot and shared SourceMeta are available. Worker-only materialization and decoded-text construction preserve original metadata.

- [Ticket04](issues/04-resolve-template-projection-and-encoding-blockers.md) resolved after merge `45d3ce2`: bounded automatic template and corrected signature route demonstrated with real providers. Full context/adapter gaps are retained explicitly in57–59.

- [Ticket05](issues/05-select-postgresql-assistance-that-never-runs-edited-sql.md) resolved after merge8a58ce9 and isolated16-test verification: native PostgreSQL syntax plus offline query-local assistance demonstrated; product integration and finite-fixture gaps remain in61.

- [Ticket03](issues/03-provide-a-vetoable-native-macos-termination-route.md) remains blocked on native macOS execution. Source-pinned research and native acceptance procedure merged at `e63d1a0`; no production hook or native pass is claimed.

- [Ticket06](issues/06-resolve-duckdb-query-local-completion-gaps.md) resolved after merge `5032a37`: exact missing-name fixtures and native syntax/help/format boundary verified with private artifacts. Complete inference, adapter integration and product performance remain in62.

- [Ticket07](issues/07-migrate-reader-core-reads-to-current-snapshots.md) resolved after runtime merge `39a830c`, full local gate and explicit user acceptance of the documented small opening/cached-operation costs. Shared current snapshots and bounded View reads are available;08–11 complete adapter/analysis ownership migration.

- Tickets08–10 resolved at runtime29fc0a/finalfixturebb162738, phased local checks complete, costs explicitly accepted2026-10-24. Reports807b27b and target/perf/editor-implementation/merge-08-10 retain evidence. Ticket11 claimed/released toworker08; nextmulti-ticketbatch11,12,13,16,18 follows dependencies. Nineof68resolved,59remain;03/nativeMacstillneeds-info.

- Ticket11 implementation-ready1cb06bb merged13ef78b; finalacceptance pending multi-ticket gate/performance. Ticket12 claimed/released to10 in da19 against merged API. Nine fullyclosed, one additional implemented pendingbatch; no productiontypingyet.

- Ticket12 implementation-ready e3d7fa5 merged16c1dc1; finalacceptance pending batch gate/performance.13 Save(08/b67f),16 draftsearch(10/da19),18 blame(09/2d02) claimed/released in parallel.9fullyclosed+2implementedpendingbatch.


- Tickets11/12/13/16/18 resolved2026-10-24 after complete phased correctness/release/stress evidence atbb2f55f4 and explicit user acceptance of the documented costs. **14of68resolved,54remaining.** Native03/24 stillunresolved. Mainc577e1d includes14/15/17/19/20/21/22 and23tooling/wholechain/nativefixes; newcohortfullgate/performancepending.
- User approvedfreshDiscardDraftandCloseafterUncertainSavewithoutrecheck2026-10-24;08implements actualclosureonly, diskuntouched, oldclosecancelled. Otherrecheckrulesstrict.
- User requestedPAUSEaftercurrentWIPfinishesandmerges. RemainingWIP:08freshcloseexception+09private23activation/verification. No newtickets afterthese; preserveallbranches/artifacts.
- [Ticket 33](issues/33-restore-the-project-session-around-recovered-work.md) resolved at merge `d0d8cf2`: recovered drafts restore into the reopened project's tabs, splits and places; an undecided draft's file is not edited (user decision 2026-10-24); a live session's drafts are not offered to another. 37 of 68 resolved.

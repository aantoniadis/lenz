# Investigate ordinary editor feedback after adding the branch cache
Status: needs-info
Type: task

Six normal-release pairs on the same frozen binaries show a small upward TUI
key-feedback tendency; GUI ordinary first-XDamage feedback also trends higher,
while sustained GUI editing has mixed signs. Preserve the initial and reversed
campaigns and their source/executable identities. Do not infer preservation from
a ceiling or discard the adverse samples.

Test one bounded layout hypothesis: the optional cold branch cache reserves
168 bytes inline in every Session, including outside Git. A Box reduces the
measured Session from 16,312 to 16,152 bytes while leaving State/Input/BranchesView
sizes unchanged. Ownership, cache bounds and cancellation must stay unchanged.
Use balanced contemporaneous baseline/old-candidate/boxed-candidate TUI triples,
the existing schema-4 workload and normal Cargo profiles. Keep or revert based
on actual results; record an inconclusive experiment honestly.

If retained, complete the combined gate and relevant final release/native checks.
A separate versioned GUI diagnostic without interleaved Saves is prepared but
unrun; it may isolate first-XDamage's background-refresh confound if needed.
Native macOS and physical presentation remain unverified.

## Answer

The layout hypothesis did not rescue latency and was reverted. Session size
falls by 160 bytes in the experiment, but ordinary feedback is mixed/equal and
sustained feedback is higher in all three direct comparisons. Runtime source
files and final normal-release Code binaries are identical to validated d7.
The separate GUI no-Save diagnostic passes and does not reproduce the upward
GUI edit tendency; this supports a confound hypothesis without proving it.

The final picker improves usable choices 20.635 → 1.407 ms while first bytes
increase 0.727 → 1.354 ms to include cached content. TUI key feedback retains a
small roughly 0.04 ms upward tendency. AGENTS.md requires explicit agreement for
a deliberate performance tradeoff. Await user acceptance before marking PR #53
ready. Full data, failed/inconclusive attempts and limits are in the
[audit report](../../../docs/research/code-branch-picker-audit.md).

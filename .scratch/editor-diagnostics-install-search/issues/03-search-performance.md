# Speed up Find and shared search paths
Status: resolved
Type: task

Profile and compare normal-release Find workloads, then improve the shared matching
or scheduling bottlenecks. Protect Unicode case folding, whole words, regex anchors,
empty/invalid patterns, CRLF, dense/absent matches, caps and cancellation. Confirm
actual GUI/TUI behavior and measure startup, input, memory and sustained work.


## Answer

Prepare a borrowed ASCII literal scanner once per bounded collection. Keep dense
lines, Unicode folding and regex on their original paths. Exact generated
regex-oracle, cap, ordering, whole-word and cancellation checks pass. Fix accepted
zero-hit completions failing to request repaint; the regression fails before the
fix and production TUI now paints No matches without another key.

Three independent normal-release process pairs measure hot absent ASCII scans
7.174→1.974ms and sparse sensitive scans 16.189→1.475ms, with comparable dense,
Unicode and regex cases. Native completed Find/reveal improves 9.875→3.450ms in
three standalone TUI pairs (165/168 exact checked queries). Six scans plus six
native processes each run >=20 seconds; an additional candidate absent/present run
lasts 20.399 seconds. Bytes, selection, resizing and terminal restoration pass.

GUI first image, editing, Save/Undo/Redo and RSS are measured on normal production
binaries. A real XDamage collector race was reproduced and fixed; final pairs
use one controlled outside-Git fixture. Early private-module timing bias, rejected
scanner variants, failed collector attempts and RSS modes remain visible in the
[validation report](../../../docs/research/editor-feedback-setup-search-validation.md). Project/fuzzy search speed and native macOS remain unverified.

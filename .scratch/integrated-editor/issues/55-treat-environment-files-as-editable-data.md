# 55: Treat environment files as editable data

**What to build:** Edit environment key/value files with predictable highlighting and quoting without executing their values.

**Blocked by:** 27: Indent, pair, comment, move and duplicate lines.

**Status:** ready-for-agent

- [ ] Recognize environment-file syntax and quote/comment/value boundaries in both adapters.
- [ ] Typing, paste, selection and undo preserve literal values, Unicode and line endings; no shell execution or interpolation side effect occurs.
- [ ] Do not expose environment contents in diagnostics, shared settings or capability logs.
- [ ] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 42, 89.

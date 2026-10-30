# 21: Enter and replace text through real GUI and terminal input

**What to build:** Type, select, cut/copy/paste and delete Unicode text in both adapters using the shared transaction commands.

**Blocked by:** 12: Edit one shared draft and history across views.

**Status:** resolved

- [x] Exercise real constructor/event mappings in existing test-support builds while keeping production writable input gated until the safety checkpoint.
- [x] Terminal q/j/k/slash/Space insert text and bracketed paste is one transaction rather than commands; explicit read-only and unsupported-source reasons remain visible.
- [x] Menus and clipboard/selection routes share availability and modal precedence; exact text and undo outcomes match between adapters.
- [x] Use the agreed Session-command seam and real adapter input/render replays for affected behavior; assert exact text/state/file output. Run relevant regressions and performance checks, retain evidence and keep unsupported claims explicit.

**Spec coverage:** user stories 1, 3, 4, 7, 9, 10, 58, 59, 60.

# Restore Code native Quit and visible close decisions
Status: resolved
Type: task

User reports macOS Code Command-Q does nothing; red window close sometimes
leaves the window open without a prompt. Code has no native Quit action/menu,
vetoes native close while any popup exists, and close preparation synchronizes
the file question but omits its terminal question until a later event.

Route Command-Q/menu Quit through the existing all-session close protocol.
Dismiss transient pickers on native close, preserve actual decisions, and
synchronize terminal questions immediately. Cancel repository-status reads on
Session Drop before joining workers. Validate registered production constructors,
all-window Cancel/Discard and idle-terminal close with exact file-byte assertions.
Native macOS execution remains unavailable; report that limit explicitly.

## Answer

Restored native Quit/Command-Q routes, dismissed transient pickers before shared
consent, synchronized terminal questions immediately and prevented unchanged
close prompts from causing a resize/repaint loop. Drop cancels Git-status reads.
Three actual-constructor GUI tests and the lifecycle regression pass in CI11;
four native X11 window-close scenarios cover Cancel, Save/Discard and idle/dirty
terminal cleanup with exact bytes and shell liveness. AppKit/macOS delivery
remains explicitly unverified. See the [audit report](../../../docs/research/code-branch-picker-audit.md).

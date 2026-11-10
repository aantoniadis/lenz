# Restore Code native Quit and visible close decisions
Status: claimed
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

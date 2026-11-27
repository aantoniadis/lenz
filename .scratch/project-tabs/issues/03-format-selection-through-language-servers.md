# Format Selection through language servers

Status: needs-triage

Format Selection formats JSON and XML offline. For files a language server formats, Ctrl+Alt+L with a selection
still formats the whole document. `textDocument/rangeFormatting` (where the server offers it) would let a selection
format alone, as IntelliJ does; it needs the same version fencing as Format Document.

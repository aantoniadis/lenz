# Template projection and provider resolution experiment

Ticket 04, 2026-10-23. This is a reproducible test-only route, not a production template feature. It follows the failures in [editor readiness](editor-readiness.md#erb-and-haml).

## Route and demonstrated behavior

[The experiment](../../crates/reader-core/tests/editor_template_resolution.rs) runs an actual Ruby LSP 0.26.11 process and the installed VS Code HTML server. [Projection code](../../crates/reader-core/tests/template_probe/probe.py) and [parser entry point](../../crates/reader-core/tests/template_probe/regions.rb) receive whole templates; callers never supply Ruby region spans.

* ERB regions come from the installed ERB scanner. Output and statement tags are extracted automatically; comments and escaped openers are excluded.
* Haml 7.5.1 is provisioned only into an owned disposable gem directory. Its actual parser validates syntax, indentation and AST contexts before the bounded source mapper extracts output expressions, simple interpolation and pipe continuations. It caught illegal inline-content-plus-child nesting in an early fixture; the retained failure demonstrates why the prior grammar-free recognizer was insufficient.
* Six real RPC cases cover ERB/Haml inline Ruby, interpolation and multiline Ruby. Every template uses CRLF and non-BMP text in host/Ruby contexts. Each case asserts the actual `greet` completion's source replacement and unchanged surrounding text; exact signature label `greet(name, excited: <default>)` and active parameter 0; and a real syntax diagnostic mapped to exactly the original `]` byte range. Draft changes use incremental synchronization; original source files stay unchanged.
* Four actual HTML completion edits are applied, not merely inspected as labels. Haml tag replacement keeps `%`; attributes keep their parentheses. Accepted Haml includes `%div` and `%div(title="😀" class="")`, under a parent tag with CRLF. The real Haml compiler accepts both results and its generated Ruby parses successfully. The HTML provider's `class="$1"` snippet is materialized as one empty attribute value; other snippet forms are refused.
* Fourteen additional real signature RPC cases assert exact labels and active parameters: opening `(`, existing arguments after the caret, comma, comma plus space, nested calls, Unicode arguments, nested commas inside an argument, a caret inside an array argument, a completed nested argument, existing closing `)`, cursor after a closed call, trailing space after a closed call and document start.

## Provider fix and failed alternatives

The [private launch preload](../../crates/reader-core/tests/template_probe/signature_cursor.rb) checks Ruby LSP's exact version and accepts only UTF-8 Ruby projection documents. Installed files, the real document scanner/cache and original request positions remain unchanged. It selects the innermost applicable parenthesized Prism call at the actual cursor, permits trailing whitespace in an incomplete call, excludes completed calls and computes the active parameter from lexed top-level commas before the cursor. Signature content, project indexing and receiver resolution still come from the actual Ruby provider.

The evidence retains these unsuccessful approaches:

1. Unmodified UTF-8 projections reproduce exact completions/diagnostics but empty signature help after a freshly typed `(`.
2. UTF-32 negotiation is accepted, but the provider selects Ruby's dummy `Encoding::UTF_32`; Prism's cached offsets are incorrectly scaled. A private UTF-32LE experiment restores lookup but outgoing completion ranges are still byte columns, so this route is unsafe.
3. Converting UTF-8 scanner character indices to bytes breaks lookup. Direct Prism inspection established that its UTF-8 cache offsets already count characters; the apparent encoding dependency was misleading.
4. A previous-character-only signature workaround fixes six simple template cases but produces wrong nested targets, missing whitespace signatures, wrong active parameters and stale signatures after `)`. The assertion-based signature edge run went red before the current target/parameter implementation went green.
5. A grammar-free Haml fixture had invalid nesting; the real Haml parser rejects it. The corrected multiline fixture puts Unicode in an attribute on its otherwise empty parent tag.

The native ERB range defect remains in the installed provider. This experiment bypasses it by sending automatically extracted Ruby as a Ruby document and applying edits only through the captured projection map. It does not declare the native ERB path safe.

## Reproduction and retained evidence

Prerequisites: Python 3, Ruby 3.4.10, installed Ruby LSP 0.26.11, Node 26.10.0, the VS Code bundled HTML server and an explicitly provisioned owned Haml 7.5.1 gem directory. The runner never installs dependencies. For this experiment the private directory contains Haml 7.5.1, Temple 0.10.7, Tilt 2.9.0, Thor 1.5.0 and Prism 2.0.0; Ruby LSP continues using its original installed Prism. Exact source hashes, executables, versions, configuration and source revision are captured by every run.

```sh
export EDITOR_HAML_GEM_HOME=/path/to/owned/template-gems
# Provision this disposable test environment explicitly; do not change user-installed tools.
gem install haml -v 7.5.1 --install-dir "$EDITOR_HAML_GEM_HOME" --no-document
export EDITOR_TEMPLATE_EVIDENCE_DIR=/path/to/fresh/owned/evidence
cargo test -p reader-core --test editor_template_resolution
cargo test -p reader-core --test editor_template_resolution template_real_provider_resolution -- --ignored --exact
```

Without the private Haml environment, the ordinary contract test explicitly skips its two Haml parser checks. Its two ERB recognition checks likewise skip when Ruby or the ERB scanner gem is absent; the three pure mapping/edit contracts still run using fixed projection inputs. The ignored real-provider test requires all prerequisites and fails when one is missing. Automatic recognition acceptance comes from the real-provider runs, never the fixed mapping fixtures.

Local evidence root: `/home/alex/.codex/worktrees/cf8f/lenz/target/perf/editor-implementation/04/`. `verified-1`, `verified-2` and `verified-3` retain equivalent final runs. Each contains `identity.json`, `summary.json`, private provider roots, framed RPC transcripts with timing, initialization capabilities, raw completion/signature/diagnostic payloads, source fixtures, exact accepted edits, generated projections, compiler output and process RSS/HWM. Earlier named runs and `failed-preloads/` retain failures. `python-contract.log`, `cargo-focused.log`, `cargo-provider.log` and `clippy.log` record validation. These ignored artifacts stay local and contain only owned synthetic fixtures.

## Limits and integration boundary

The reusable experiment boundary is `project(source, language) -> Projection`, strict `provider_range`/`apply_provider_edit` against that snapshot, and `markup_project`/`apply_markup_edit` for supported tag/attribute contexts. URI/version refusal here uses a fixed immutable test identity; it does not prove a production asynchronous lifecycle.

Haml mapping currently refuses dynamic attributes, indentation-controlled Ruby blocks, filters, comments, escaped lines, nested/incomplete interpolation and other unmapped contexts. ERB escaped closing delimiters and trim-mode behavior are not established. Markup projection covers isolated tag/attribute contexts; general mixed markup/Ruby routing, full snippet navigation and Rails helper context remain unverified. The private signature prototype covers parenthesized calls and the enumerated argument shapes, not all Ruby call forms, overloads, splats or keyword-parameter semantics. It deliberately does not support native ERB documents.

No production API, Session, GUI or TUI code changes. Later template integration must port a supported route into the shared core and exercise actual adapter input, availability, cancellation, stale responses, save and undo. The experiment does not establish a complete template feature or adapter parity.

Across three equivalent final runs, ERB provider lifetimes were 0.97–1.07 seconds, Haml lifetimes 1.27–1.40 seconds and HTML lifetimes 0.35–0.37 seconds. End-of-run provider RSS ranged from 84–85 MiB (ERB), 103–107 MiB (Haml) and 124–134 MiB (HTML); `verified-resources.json` retains exact values. These short samples characterize the experiment, not production performance preservation.

Request timings measure completed RPCs and initialization replies; process elapsed time includes indexing/waits and synthetic interaction. RSS/HWM cover each provider process only. There is no display latency, GPU measurement, sustained memory stress or production startup comparison. Native macOS is unverified.

Primary syntax references: [ERB](https://docs.ruby-lang.org/en/master/ERB.html), [Haml interpolation and multiline syntax](https://haml.info/docs/yardoc/file.REFERENCE.html), [LSP position encodings](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#positionEncodingKind). Installed provider/parser implementations are pinned by the evidence hashes.

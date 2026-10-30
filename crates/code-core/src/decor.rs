//! Language-server decorations of a document version: inlay hints (labels drawn between characters) and semantic
//! tokens (colours laid over the syntax colours), kept compact and sorted for per-line lookup by binary search.

use std::{ops::Range, sync::Arc};

use lenz_content::highlight::Capture;
use serde_json::Value;

use crate::{document::Document, semantics::lsp::{self, Encoding}};

/// What a hint names; each follows its own setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Type,
    Parameter,
}

/// One inlay hint: drawn before byte `byte` of its line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub byte: u32,
    /// As drawn: padding included, cut to [`LABEL_CHARS`].
    pub label: Box<str>,
    pub kind: HintKind,
}

/// Labels longer than this are cut, ending in `…`.
pub const LABEL_CHARS: usize = 30;

/// A version's hints, sorted by line then byte (a server's order kept at one byte).
#[derive(Debug, Default)]
pub struct Hints {
    lines: Vec<u32>,
    hints: Vec<Hint>,
}

impl Hints {
    pub fn new(mut hints: Vec<(u32, Hint)>) -> Self {
        hints.sort_by_key(|(l, h)| (*l, h.byte));
        let (lines, hints) = hints.into_iter().unzip();
        Self { lines, hints }
    }
    /// Line `line`'s hints.
    pub fn line(&self, line: usize) -> &[Hint] {
        let l = u32::try_from(line).unwrap_or(u32::MAX);
        &self.hints[self.lines.partition_point(|&x| x < l)..self.lines.partition_point(|&x| x <= l)]
    }
    pub fn len(&self) -> usize {
        self.hints.len()
    }
    pub fn is_empty(&self) -> bool {
        self.hints.is_empty()
    }
    /// Heap bytes held (the memory report).
    pub fn bytes(&self) -> usize {
        self.lines.capacity() * 4 + self.hints.capacity() * std::mem::size_of::<Hint>() + self.hints.iter().map(|h| h.label.len()).sum::<usize>()
    }
}

/// The two kinds of decoration, asked for separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decor {
    Hints = 0,
    Tokens = 1,
}

impl Decor {
    pub fn method(self) -> &'static str {
        match self {
            Self::Hints => "textDocument/inlayHint",
            Self::Tokens => "textDocument/semanticTokens/full",
        }
    }
    /// The server capability that offers it.
    pub fn capability(self) -> &'static str {
        match self {
            Self::Hints => "inlayHintProvider",
            Self::Tokens => "semanticTokensProvider",
        }
    }
}

/// Which hint kinds are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HintFilter {
    pub types: bool,
    pub parameters: bool,
}

/// A decoration response as sent: its result (`None`: an error or no result).
#[derive(serde::Deserialize)]
struct Reply<T> {
    result: Option<T>,
}

#[derive(serde::Deserialize)]
struct WirePosition {
    line: u64,
    character: u64,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum WireLabel {
    Text(String),
    Parts(Vec<WirePart>),
}

/// A label part: its text (tooltips, locations and commands are skipped unread).
#[derive(serde::Deserialize)]
struct WirePart {
    value: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireHint {
    position: WirePosition,
    label: WireLabel,
    kind: Option<u64>,
    #[serde(default)]
    padding_left: bool,
    #[serde(default)]
    padding_right: bool,
}

/// One entry of a hint list: a hint, or anything else (skipped).
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum WireItem {
    Hint(WireHint),
    Other(serde::de::IgnoredAny),
}

/// `hint`'s label as drawn: its parts joined, hidden characters as spaces, cut to [`LABEL_CHARS`], padded.
fn label(hint: &WireHint) -> String {
    let text = match &hint.label {
        WireLabel::Text(s) => s.clone(),
        WireLabel::Parts(parts) => parts.iter().map(|p| p.value.as_str()).collect(),
    };
    let mut text: String = text.chars().map(|c| if lenz_content::display::hides_text(c) { ' ' } else { c }).collect();
    if text.chars().count() > LABEL_CHARS {
        text = text.chars().take(LABEL_CHARS - 1).collect::<String>() + "…";
    }
    let pad = |on: bool| if on { " " } else { "" };
    format!("{}{text}{}", pad(hint.padding_left), pad(hint.padding_right))
}

/// An `inlayHint` response's bytes about `doc`: the kinds `filter` keeps, on lines of `doc`, with a visible label.
/// Decoded straight into hints: a server's JSON (rust-analyzer: about 6 MB for 10,000 lines) is never built as a tree.
pub fn decode_hints(body: &[u8], doc: &Document, encoding: Encoding, filter: HintFilter) -> Hints {
    let items = serde_json::from_slice::<Reply<Vec<WireItem>>>(body).ok().and_then(|r| r.result).unwrap_or_default();
    let mut out = Vec::with_capacity(items.len());
    for h in items.iter().filter_map(|i| match i {
        WireItem::Hint(h) => Some(h),
        WireItem::Other(_) => None,
    }) {
        let kind = match h.kind {
            Some(1) if filter.types => HintKind::Type,
            Some(2) if filter.parameters => HintKind::Parameter,
            _ => continue,
        };
        let (Ok(line), Ok(character)) = (usize::try_from(h.position.line), usize::try_from(h.position.character)) else { continue };
        let Some(at) = lsp::offset_at(doc, line, character, encoding) else { continue };
        let label = label(h);
        if label.trim().is_empty() {
            continue;
        }
        // The server's line may be part of one of the reader's (after a lone CR).
        let line = doc.line_of(at);
        out.push((line as u32, Hint { byte: (at - doc.line_range(line).start) as u32, label: label.into(), kind }));
    }
    Hints::new(out)
}

/// A `semanticTokens/full` result.
#[derive(serde::Deserialize)]
struct WireTokens {
    #[serde(deserialize_with = "lenient_numbers")]
    data: Vec<u64>,
}

/// A token array's numbers, each non-integer read as `u64::MAX` (its token is dropped, the others kept) instead of
/// failing the whole array. Read in place: nothing is buffered per element.
fn lenient_numbers<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    struct Numbers;
    impl<'de> serde::de::Visitor<'de> for Numbers {
        type Value = Vec<u64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an array of numbers")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u64>, A::Error> {
            let mut out = Vec::new();
            while let Some(Lenient(n)) = seq.next_element()? {
                out.push(n);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Numbers)
}

/// One number of a token array: a non-negative integer, or `u64::MAX` for anything else.
struct Lenient(u64);

impl<'de> serde::Deserialize<'de> for Lenient {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Number;
        impl<'de> serde::de::Visitor<'de> for Number {
            type Value = Lenient;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a number")
            }
            fn visit_u64<E: serde::de::Error>(self, n: u64) -> Result<Lenient, E> {
                Ok(Lenient(n))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Lenient, E> {
                Ok(Lenient(u64::MAX))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Lenient, E> {
                Ok(Lenient(u64::MAX))
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Lenient, E> {
                Ok(Lenient(u64::MAX))
            }
            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Lenient, E> {
                Ok(Lenient(u64::MAX))
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Lenient, E> {
                Ok(Lenient(u64::MAX))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, seq: A) -> Result<Lenient, A::Error> {
                serde::de::Visitor::visit_seq(serde::de::IgnoredAny, seq).map(|_| Lenient(u64::MAX))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, map: A) -> Result<Lenient, A::Error> {
                serde::de::Visitor::visit_map(serde::de::IgnoredAny, map).map(|_| Lenient(u64::MAX))
            }
        }
        d.deserialize_any(Number)
    }
}

/// Underlined: a variable the server says is mutable.
pub const MUTABLE: u8 = 1;
/// Struck through.
pub const DEPRECATED: u8 = 2;
const READONLY: u8 = 4;
const STATIC: u8 = 8;
const DEFAULT_LIBRARY: u8 = 16;
/// A diagnostic's range, drawn by adapters as a heavier underline in its severity's colour (never a token modifier).
pub const PROBLEM: u8 = 32;

/// The token types and modifiers the reader announces (the protocol's standard ones, and rust-analyzer's `mutable`).
pub const TOKEN_TYPES: &[&str] = &["namespace", "type", "class", "enum", "interface", "struct", "typeParameter", "parameter", "variable", "property", "enumMember", "event", "function", "method", "macro", "keyword", "modifier", "comment", "string", "number", "regexp", "operator", "decorator", "label"];
pub const TOKEN_MODIFIERS: &[&str] = &["declaration", "definition", "readonly", "static", "deprecated", "abstract", "async", "modification", "documentation", "defaultLibrary", "mutable"];

/// One semantic token on one line, in bytes of that line: a colour (`None`: the syntax colour stays) and
/// [`MUTABLE`] / [`DEPRECATED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub line: u32,
    pub start: u32,
    pub end: u32,
    pub capture: Option<Capture>,
    pub mods: u8,
}

/// A version's tokens, sorted by line then start, without overlaps.
#[derive(Debug, Default)]
pub struct Tokens {
    tokens: Vec<Token>,
}

impl Tokens {
    pub fn line(&self, line: usize) -> &[Token] {
        let l = u32::try_from(line).unwrap_or(u32::MAX);
        &self.tokens[self.tokens.partition_point(|t| t.line < l)..self.tokens.partition_point(|t| t.line <= l)]
    }
    pub fn len(&self) -> usize {
        self.tokens.len()
    }
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }
    pub fn bytes(&self) -> usize {
        self.tokens.capacity() * std::mem::size_of::<Token>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Variable,
    Type,
    Other,
}

fn base(ty: &str) -> (Option<Capture>, Class) {
    match ty {
        "namespace" => (Some(Capture::Module), Class::Other),
        "type" | "class" | "enum" | "interface" | "struct" | "typeParameter" => (Some(Capture::Type), Class::Type),
        "parameter" => (Some(Capture::Parameter), Class::Other),
        "variable" => (Some(Capture::Variable), Class::Variable),
        "property" => (Some(Capture::Property), Class::Other),
        "enumMember" => (Some(Capture::Constant), Class::Other),
        "function" | "method" => (Some(Capture::Function), Class::Other),
        "macro" => (Some(Capture::Macro), Class::Other),
        "decorator" => (Some(Capture::Attribute), Class::Other),
        _ => (None, Class::Other),
    }
}

/// A server's token legend, read once per server: each type's colour and each modifier's meaning.
#[derive(Debug, Default)]
pub struct Legend {
    types: Vec<(Option<Capture>, Class)>,
    modifiers: Vec<u8>,
}

impl Legend {
    /// From a `semanticTokensProvider.legend` (anything else: an empty legend, no tokens).
    pub fn new(legend: &Value) -> Self {
        let names = |key: &str| legend.get(key).and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().map(|v| v.as_str().unwrap_or_default().to_owned()).collect());
        let modifier = |m: &str| match m {
            "mutable" => MUTABLE,
            "deprecated" => DEPRECATED,
            "readonly" => READONLY,
            "static" => STATIC,
            "defaultLibrary" => DEFAULT_LIBRARY,
            _ => 0,
        };
        Self { types: names("tokenTypes").iter().map(|t| base(t)).collect(), modifiers: names("tokenModifiers").iter().map(|m| modifier(m)).collect() }
    }
    /// Type `ty` with modifier bits `bits`: its colour and drawn modifiers; `None` to drop it (unknown index, or
    /// nothing to draw).
    fn class(&self, ty: usize, bits: usize) -> Option<(Option<Capture>, u8)> {
        let &(base, class) = self.types.get(ty)?;
        let flags = self.modifiers.iter().enumerate().filter(|&(i, _)| i < usize::BITS as usize && bits >> i & 1 == 1).fold(0, |f, (_, m)| f | m);
        let capture = match class {
            Class::Variable if flags & (READONLY | STATIC) != 0 => Some(Capture::Constant),
            Class::Type if flags & DEFAULT_LIBRARY != 0 => Some(Capture::TypeBuiltin),
            _ => base,
        };
        let mods = flags & (MUTABLE | DEPRECATED);
        (capture.is_some() || mods != 0).then_some((capture, mods))
    }
}

/// `target` units into `text` from cursor `(byte, unit)` (restarted when behind): the byte and unit reached.
fn advance(text: &str, (mut byte, mut unit): (usize, usize), target: usize, encoding: Encoding) -> (usize, usize) {
    if unit > target {
        (byte, unit) = (0, 0);
    }
    for c in text[byte..].chars() {
        if unit >= target {
            break;
        }
        unit += match encoding {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        };
        byte += c.len_utf8();
    }
    (byte, unit)
}

/// A `semanticTokens/full` response's bytes about `doc` (relative five-integer form): tokens cut at their line end;
/// a token holding a non-integer is dropped, the others kept.
pub fn decode_tokens(body: &[u8], doc: &Document, encoding: Encoding, legend: &Legend) -> Tokens {
    let data = serde_json::from_slice::<Reply<WireTokens>>(body).ok().and_then(|r| r.result).map_or_else(Vec::new, |t| t.data);
    let mut out = Vec::with_capacity(data.len() / 5);
    let (mut line, mut start) = (0usize, 0usize);
    let mut cursor = (usize::MAX, 0, 0);
    for t in data.as_chunks::<5>().0 {
        let [dl, ds, len, ty, bits] = t.map(|n| usize::try_from(n).unwrap_or(usize::MAX));
        // A non-integer (read as MAX) drops its token only; the valid deltas beside it still move the position. An
        // unknown line delta leaves every later token's line unknown: the rest is not used.
        if dl == usize::MAX {
            break;
        }
        line = line.saturating_add(dl);
        if dl > 0 {
            start = 0;
        }
        if ds != usize::MAX {
            start = start.saturating_add(ds);
        }
        // The server's line, which may be part of one of the reader's (after a lone CR).
        let Some(range) = lsp::line_range(doc, line) else { break };
        if [dl, ds, len, ty, bits].contains(&usize::MAX) {
            continue;
        }
        let Some((capture, mods)) = legend.class(ty, bits) else { continue };
        let text = &doc.text()[range.clone()];
        if cursor.0 != line {
            cursor = (line, 0, 0);
        }
        let (a, au) = advance(text, (cursor.1, cursor.2), start, encoding);
        let (b, _) = advance(text, (a, au), start.saturating_add(len), encoding);
        cursor = (line, a, au);
        if a < b {
            let (shown, base) = match doc.protocol_line_starts() {
                Some(_) => {
                    let shown = doc.line_of(range.start);
                    (shown, doc.line_range(shown).start)
                }
                None => (line, range.start),
            };
            out.push(Token { line: shown as u32, start: (range.start + a - base) as u32, end: (range.start + b - base) as u32, capture, mods });
        }
    }
    out.shrink_to_fit();
    Tokens { tokens: out }
}

/// A decoded decoration for one document version.
pub enum Decorated {
    Hints(Arc<Hints>),
    Tokens(Arc<Tokens>),
}

impl Decorated {
    pub fn kind(&self) -> Decor {
        match self {
            Self::Hints(_) => Decor::Hints,
            Self::Tokens(_) => Decor::Tokens,
        }
    }
}

/// A line's syntax spans (sorted, disjoint) with its `tokens` laid over: a token's colour replaces the syntax colour
/// on its bytes (splitting the span), its modifiers ride along; a token without a colour keeps the syntax colour, or
/// none over unstyled text (`None`: the default text colour). Bytes with neither are left out.
pub fn overlay(syntax: &[(Range<usize>, Capture)], tokens: &[Token]) -> Vec<(Range<usize>, Option<Capture>, u8)> {
    if tokens.is_empty() {
        return syntax.iter().map(|(r, c)| (r.clone(), Some(*c), 0)).collect();
    }
    let mut cuts: Vec<usize> = syntax.iter().flat_map(|(r, _)| [r.start, r.end]).chain(tokens.iter().flat_map(|t| [t.start as usize, t.end as usize])).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut out: Vec<(Range<usize>, Option<Capture>, u8)> = Vec::with_capacity(syntax.len() + tokens.len() * 2);
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let s = syntax.get(syntax.partition_point(|(r, _)| r.end <= a)).filter(|(r, _)| r.start <= a).map(|(_, c)| *c);
        let t = tokens.get(tokens.partition_point(|t| t.end as usize <= a)).filter(|t| t.start as usize <= a);
        let (capture, mods) = match t {
            Some(t) => (t.capture.or(s), t.mods),
            None => (s, 0),
        };
        if capture.is_none() && mods == 0 {
            continue;
        }
        match out.last_mut() {
            Some((r, c, m)) if r.end == a && *c == capture && *m == mods => r.end = b,
            _ => out.push((a..b, capture, mods)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{document::Document, semantics::lsp::Encoding};
    use lenz_content::highlight::Capture;
    use serde_json::json;

    fn doc(text: &str) -> Document {
        Document::decode("/t.rs".into(), 7, text.as_bytes())
    }
    /// A decoration response's bytes, as a server sends them.
    fn wire(result: &Value) -> Vec<u8> {
        json!({ "jsonrpc": "2.0", "id": "decor-1", "result": result }).to_string().into_bytes()
    }
    fn dh(result: &Value, doc: &Document, encoding: Encoding, filter: HintFilter) -> Hints {
        decode_hints(&wire(result), doc, encoding, filter)
    }
    fn dt(result: &Value, doc: &Document, encoding: Encoding, legend: &Legend) -> Tokens {
        decode_tokens(&wire(result), doc, encoding, legend)
    }
    const ALL: HintFilter = HintFilter { types: true, parameters: true };

    fn legend() -> Legend {
        Legend::new(&json!({ "tokenTypes": ["variable", "function", "keyword", "type", "frobnicator"], "tokenModifiers": ["mutable", "deprecated", "readonly", "defaultLibrary"] }))
    }

    #[test]
    fn hint_kinds_labels_padding_and_the_cut() {
        let d = doc("let x = f(1);\n");
        let reply = json!([
            { "position": { "line": 0, "character": 5 }, "label": ": i32", "kind": 1 },
            { "position": { "line": 0, "character": 10 }, "label": [{ "value": "a" }, { "value": ":" }], "kind": 2, "paddingRight": true },
            { "position": { "line": 0, "character": 0 }, "label": "no kind" },
            { "position": { "line": 0, "character": 12 }, "label": "x".repeat(40), "kind": 1, "paddingLeft": true },
            { "position": { "line": 0, "character": 3 }, "label": "a\nb", "kind": 1 },
        ]);
        let h = dh(&reply, &d, Encoding::Utf16, ALL);
        let long = format!(" {}…", "x".repeat(29));
        let got: Vec<(u32, &str)> = h.line(0).iter().map(|h| (h.byte, &*h.label)).collect();
        assert_eq!(got, vec![(3, "a b"), (5, ": i32"), (10, "a: "), (12, long.as_str())]);
        assert_eq!(dh(&reply, &d, Encoding::Utf16, HintFilter { types: true, parameters: false }).len(), 3);
        assert_eq!(dh(&reply, &d, Encoding::Utf16, HintFilter { types: false, parameters: true }).len(), 1);
        assert!(dh(&json!(null), &d, Encoding::Utf16, ALL).is_empty());
    }

    #[test]
    fn hint_positions_count_the_servers_units_and_clamp_to_the_line() {
        let d = doc("é😀x\r\nb\n");
        let at = |encoding, character: u64| dh(&json!([{ "position": { "line": 0, "character": character }, "label": "T", "kind": 1 }]), &d, encoding, ALL).line(0)[0].byte;
        assert_eq!(at(Encoding::Utf16, 3), 6);
        assert_eq!(at(Encoding::Utf8, 6), 6);
        assert_eq!(at(Encoding::Utf16, 99), 7, "past the end: the line end, before its CRLF");
        assert!(dh(&json!([{ "position": { "line": 9, "character": 0 }, "label": "T", "kind": 1 }]), &d, Encoding::Utf16, ALL).is_empty());
    }

    #[test]
    fn tokens_decode_relative_positions_through_the_legend() {
        let d = doc("let mut v = old();\nfn old() {}\n");
        let data = json!({ "data": [0, 8, 1, 0, 0b1, 0, 4, 3, 1, 0b10, 1, 3, 3, 1, 0] });
        let t = dt(&data, &d, Encoding::Utf16, &legend());
        assert_eq!(t.line(0), &[Token { line: 0, start: 8, end: 9, capture: Some(Capture::Variable), mods: MUTABLE }, Token { line: 0, start: 12, end: 15, capture: Some(Capture::Function), mods: DEPRECATED }]);
        assert_eq!(t.line(1), &[Token { line: 1, start: 3, end: 6, capture: Some(Capture::Function), mods: 0 }]);
    }

    #[test]
    fn modifiers_change_captures_and_plain_or_unknown_types_keep_the_syntax_colour() {
        let d = doc("let x: u8 = K;\n");
        // `let` keyword (dropped), x readonly variable, u8 defaultLibrary type, K unknown type (dropped), index 9 (skipped).
        let data = json!({ "data": [0, 0, 3, 2, 0, 0, 4, 1, 0, 0b100, 0, 3, 2, 3, 0b1000, 0, 5, 1, 4, 0, 0, 0, 1, 9, 0] });
        let t = dt(&data, &d, Encoding::Utf16, &legend());
        assert_eq!(t.line(0).iter().map(|t| (t.start, t.capture)).collect::<Vec<_>>(), vec![(4, Some(Capture::Constant)), (7, Some(Capture::TypeBuiltin))]);
    }

    #[test]
    fn token_columns_count_the_servers_units_and_stop_at_the_line_end() {
        let d = doc("é😀ab\nc\n");
        let t = dt(&json!({ "data": [0, 3, 1, 0, 0, 0, 1, 9, 0, 0, 5, 0, 1, 0, 0] }), &d, Encoding::Utf16, &legend());
        assert_eq!(t.line(0).iter().map(|t| (t.start, t.end)).collect::<Vec<_>>(), vec![(6, 7), (7, 8)]);
        let t = dt(&json!({ "data": [0, 6, 1, 0, 0] }), &d, Encoding::Utf8, &legend());
        assert_eq!((t.line(0)[0].start, t.line(0)[0].end), (6, 7));
    }

    #[test]
    fn hostile_replies_decode_without_panicking() {
        let d = doc("ab\ncd\n");
        let big = u64::MAX;
        for data in [json!({ "data": "no" }), json!({ "data": [big, big, big, big, big] }), json!({ "data": [0, 0, 1, 0, big, 0, big, 1, 0, 0] }), json!({ "data": [0, 0, 1, 0, 0, "x", 0, 1, 0, 0] })] {
            let t = dt(&data, &d, Encoding::Utf16, &legend());
            assert!(t.len() <= 1, "{data}");
        }
        assert_eq!(dt(&json!({ "data": [0, 0, 1, 0, 0, "x", 0, 1, 0, 0] }), &d, Encoding::Utf16, &legend()).len(), 1, "the token before a non-integer is kept");
        assert_eq!(dh(&json!([{ "position": "x", "label": "T", "kind": 1 }, { "position": { "line": 0, "character": 1 }, "label": "T", "kind": 1 }]), &d, Encoding::Utf16, ALL).len(), 1, "a malformed hint is skipped, the next kept");
        assert!(decode_hints(b"{\"result\":", &d, Encoding::Utf16, ALL).is_empty() && decode_tokens(b"[", &d, Encoding::Utf16, &legend()).is_empty(), "cut JSON");
        for reply in [json!({ "a": 1 }), json!([{ "position": "x", "label": "T", "kind": 1 }]), json!([{ "position": { "line": big, "character": big }, "label": "T", "kind": 1 }]), json!([{ "position": { "line": 0, "character": 0 }, "label": [{ "x": 1 }], "kind": 1 }])] {
            assert!(dh(&reply, &d, Encoding::Utf16, ALL).is_empty(), "{reply}");
        }
        assert_eq!(Legend::new(&json!({ "tokenModifiers": vec!["mutable"; 80] })).class(0, usize::MAX), None, "no types: nothing");
    }

    #[test]
    fn tokens_overlay_syntax_spans_splitting_them() {
        use Capture::*;
        let syntax = vec![(0..10, Variable), (12..14, Keyword)];
        let tokens = [
            Token { line: 0, start: 2, end: 4, capture: Some(Function), mods: 0 },
            Token { line: 0, start: 6, end: 8, capture: None, mods: DEPRECATED },
            Token { line: 0, start: 15, end: 16, capture: Some(Type), mods: MUTABLE },
        ];
        assert_eq!(overlay(&syntax, &tokens), vec![(0..2, Some(Variable), 0), (2..4, Some(Function), 0), (4..6, Some(Variable), 0), (6..8, Some(Variable), DEPRECATED), (8..10, Some(Variable), 0), (12..14, Some(Keyword), 0), (15..16, Some(Type), MUTABLE)]);
        assert_eq!(overlay(&syntax, &[]), vec![(0..10, Some(Variable), 0), (12..14, Some(Keyword), 0)]);
    }

    #[test]
    #[ignore = "timing; run in release"]
    fn decode_perf() {
        let text: String = (0..10_000).map(|i| format!("    let value_{i} = compute(alpha, beta, gamma); // {i}\n")).collect();
        let d = Document::decode("/big.rs".into(), 1, text.as_bytes());
        let data: Vec<u64> = (0..10_000u64).flat_map(|_| [1, 4, 3, 2, 0, 0, 4, 7, 0, 1, 0, 10, 7, 1, 0, 0, 8, 5, 0, 0, 0, 7, 4, 0, 0, 0, 6, 5, 0, 0]).collect();
        let tokens = json!({ "data": data });
        let hints = Value::Array((0..10_000).flat_map(|l| [json!({ "position": { "line": l, "character": 14 }, "label": ": i32", "kind": 1 }), json!({ "position": { "line": l, "character": 30 }, "label": "a:", "kind": 2, "paddingRight": true })]).collect());
        let t0 = std::time::Instant::now();
        let k = dt(&tokens, &d, Encoding::Utf16, &legend());
        let t1 = std::time::Instant::now();
        let h = dh(&hints, &d, Encoding::Utf16, ALL);
        println!("tokens {} in {:?} ({} B); hints {} in {:?} ({} B)", k.len(), t1 - t0, k.bytes(), h.len(), t1.elapsed(), h.bytes());
    }

    #[test]
    fn a_non_integer_drops_only_the_token_holding_it() {
        let d = doc("let mut v = old();\nfn old() {}\n");
        // A: v mutable; B: a string start (dropped); C: old deprecated, counted from A; D: null modifiers (dropped,
        // its line delta still moves to line 1); E: old on line 1, counted from D.
        let data = json!({ "data": [0, 8, 1, 0, 0b1, 0, "x", 3, 1, 0, 0, 4, 3, 1, 0b10, 1, 3, 3, 1, null, 0, 0, 3, 1, 0] });
        let t = dt(&data, &d, Encoding::Utf16, &legend());
        assert_eq!(t.line(0), &[Token { line: 0, start: 8, end: 9, capture: Some(Capture::Variable), mods: MUTABLE }, Token { line: 0, start: 12, end: 15, capture: Some(Capture::Function), mods: DEPRECATED }]);
        assert_eq!(t.line(1), &[Token { line: 1, start: 3, end: 6, capture: Some(Capture::Function), mods: 0 }]);
        for bad in [json!(-1), json!(1.5), json!(true), json!([1]), json!({ "a": 1 })] {
            let t = dt(&json!({ "data": [0, 8, 1, 0, 0, 0, 4, 3, 1, bad] }), &d, Encoding::Utf16, &legend());
            assert_eq!(t.len(), 1, "{bad}");
        }
    }

    #[test]
    fn a_non_integer_line_delta_ends_the_tokens() {
        let d = doc("let mut v = old();\nfn old() {}\nfn f() {}\n");
        // A on line 0; B's line delta is unknown, so where C (and anything after) sits is unknown too.
        let data = json!({ "data": [0, 8, 1, 0, 0, "x", 0, 3, 1, 0, 1, 3, 1, 1, 0] });
        let t = dt(&data, &d, Encoding::Utf16, &legend());
        assert_eq!(t.len(), 1, "only the token before it");
        assert_eq!(t.line(0).len(), 1);
    }

    #[test]
    fn a_modifier_only_token_over_unstyled_text_keeps_no_colour() {
        let tokens = [Token { line: 0, start: 4, end: 7, capture: None, mods: MUTABLE }];
        assert_eq!(overlay(&[(0..3, Capture::Keyword)], &tokens), vec![(0..3, Some(Capture::Keyword), 0), (4..7, None, MUTABLE)]);
    }

    #[test]
    fn a_hint_after_a_lone_cr_is_on_the_readers_line() {
        // protocol lines: "a", "b", "c"; reader lines: "a\rb", "c"
        let d = doc("a\rb\nc");
        let h = std::panic::catch_unwind(|| dh(&json!([{ "position": { "line": 2, "character": 0 }, "label": "T", "kind": 1 }]), &d, Encoding::Utf16, ALL));
        assert!(h.is_ok(), "decode_hints panicked");
        let h = h.unwrap();
        assert_eq!(h.line(1).len(), 1, "hint on reader line 1 (c)");
    }

    #[test]
    fn a_hint_on_a_later_line_after_a_lone_cr_is_kept() {
        let d = doc("a\rb\nc\nd\n");
        let h = std::panic::catch_unwind(|| dh(&json!([{ "position": { "line": 2, "character": 0 }, "label": "T", "kind": 1 }]), &d, Encoding::Utf16, ALL));
        assert!(h.is_ok(), "decode_hints panicked (underflow)");
    }

    #[test]
    fn a_token_after_a_lone_cr_is_on_the_readers_line() {
        // protocol line 2 is "cc"; reader line 1 is "cc"
        let d = doc("a\rb\ncc\n");
        let t = dt(&json!({ "data": [2, 0, 2, 0, 0] }), &d, Encoding::Utf16, &legend());
        assert_eq!(t.line(1).len(), 1, "token on reader line 1 (cc): got line1={:?} line2={:?}", t.line(1), t.line(2));
    }
}

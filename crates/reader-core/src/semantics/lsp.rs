//! The Language Server Protocol's wire details the reader needs: message framing, file URIs and positions in
//! the negotiated encoding, converted against the exact snapshot the server was sent.

use std::{
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::document::Document;

/// The largest message the reader accepts from a server; anything larger ends the connection.
pub const MAX_MESSAGE: usize = 64 << 20;

/// Read one framed message; `None` at end of stream or on a malformed or oversized frame.
pub fn read_message(input: &mut impl BufRead) -> Option<Value> {
    serde_json::from_slice(&read_frame(input)?).ok()
}

/// The id prefix of decoration requests, whose (large) answers are decoded from their bytes off the UI thread.
pub const DECORATION: &str = "decor-";

/// For an answer to a decoration request: its number and whether it is an error. Its result is skipped, not built.
pub fn decoration_id(body: &[u8]) -> Option<(i64, bool)> {
    #[derive(serde::Deserialize)]
    struct Head {
        id: Option<Value>,
        method: Option<serde::de::IgnoredAny>,
        error: Option<serde::de::IgnoredAny>,
    }
    let head: Head = serde_json::from_slice(body).ok()?;
    let n = head.id.as_ref()?.as_str()?.strip_prefix(DECORATION)?.parse().ok()?;
    head.method.is_none().then_some((n, head.error.is_some()))
}

/// Read one frame's body; `None` at end of stream or on a malformed or oversized frame.
pub fn read_frame(input: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length.filter(|&n| n <= MAX_MESSAGE)?;
    let mut body = vec![0; length];
    input.read_exact(&mut body).ok()?;
    Some(body)
}

pub fn write_message(output: &mut impl Write, message: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(message)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

/// How the server counts columns, as negotiated (UTF-16 when the server names none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
}

impl Encoding {
    pub fn negotiated(capabilities: &Value) -> Self {
        match capabilities.get("positionEncoding").and_then(Value::as_str) {
            Some("utf-8") => Self::Utf8,
            _ => Self::Utf16,
        }
    }
}

/// A protocol position (line, column in `encoding`) for byte `offset` of `doc`. Protocol lines also end at a lone
/// CR, which the reader's lines keep as text (rust-analyzer alone does not, and a lone CR is an error in Rust).
pub fn position(doc: &Document, offset: usize, encoding: Encoding) -> Value {
    let line = line_of(doc, offset);
    let range = line_range(doc, line).expect("the line holding an offset");
    let at = offset.clamp(range.start, range.end);
    serde_json::json!({ "line": line, "character": units(&doc.text()[range.start..at], encoding) })
}

/// The byte offset of protocol position `pos` in `doc`, clamped to its line; `None` past the last line or for a
/// malformed position.
pub fn offset(doc: &Document, pos: &Value, encoding: Encoding) -> Option<usize> {
    let line = usize::try_from(pos.get("line")?.as_u64()?).ok()?;
    let character = usize::try_from(pos.get("character")?.as_u64()?).ok()?;
    offset_at(doc, line, character, encoding)
}

/// The byte offset of `character` units into protocol line `line` of `doc`, clamped to the line; `None` past the last
/// line.
pub fn offset_at(doc: &Document, line: usize, character: usize, encoding: Encoding) -> Option<usize> {
    let range = line_range(doc, line)?;
    Some(range.start + units_in(&doc.text()[range], character, encoding))
}

/// Protocol line `line`'s byte range in `doc` (without its line end); `None` past the last.
pub fn line_range(doc: &Document, line: usize) -> Option<std::ops::Range<usize>> {
    let Some(starts) = doc.protocol_line_starts() else {
        return (line < doc.line_count()).then(|| doc.line_range(line));
    };
    let start = *starts.get(line)?;
    let end = match starts.get(line + 1) {
        Some(&next) => next - if doc.text()[..next].ends_with("\r\n") { 2 } else { 1 },
        None => doc.text().len(),
    };
    Some(start..end)
}

/// The protocol line holding byte `offset` of `doc` (clamped to the text).
pub fn line_of(doc: &Document, offset: usize) -> usize {
    match doc.protocol_line_starts() {
        Some(starts) => starts.partition_point(|&s| s <= offset.min(doc.text().len())) - 1,
        None => doc.line_of(offset),
    }
}

/// `text`'s length in `encoding` units.
fn units(text: &str, encoding: Encoding) -> usize {
    match encoding {
        Encoding::Utf8 => text.len(),
        Encoding::Utf16 => text.encode_utf16().count(),
    }
}

/// The byte offset of `character` units into `text`, clamped to its end.
fn units_in(text: &str, character: usize, encoding: Encoding) -> usize {
    let mut count = 0;
    for (i, c) in text.char_indices() {
        if count >= character {
            return i;
        }
        count += match encoding {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
        };
    }
    text.len()
}

/// The `file:` URI of an absolute path, percent-encoding everything but unreserved characters and `/`.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    let path = path.to_string_lossy();
    if !path.starts_with('/') {
        uri.push('/'); // a Windows drive path
    }
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => uri.push(b as char),
            b'\\' => uri.push('/'),
            b => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

/// The path of a `file:` URI on this machine; `None` for any other scheme, a remote host or bad escapes.
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let mut bytes = Vec::with_capacity(rest.len());
    let mut it = rest.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hex = [it.next()?, it.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        let s = String::from_utf8(bytes).ok()?;
        Some(PathBuf::from(s.trim_start_matches('/')))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> Document {
        Document::decode(PathBuf::from("/t.rs"), 1, text.as_bytes())
    }

    #[test]
    fn frames_round_trip_and_bad_frames_end_the_stream() {
        let mut out = Vec::new();
        write_message(&mut out, &serde_json::json!({"id": 1, "x": "é"})).unwrap();
        write_message(&mut out, &serde_json::json!({"id": 2})).unwrap();
        let mut input = std::io::Cursor::new(out);
        assert_eq!(read_message(&mut input).unwrap()["x"], "é");
        assert_eq!(read_message(&mut input).unwrap()["id"], 2);
        assert!(read_message(&mut input).is_none());
        let huge = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE + 1);
        assert!(read_message(&mut std::io::Cursor::new(huge)).is_none());
        assert!(read_message(&mut std::io::Cursor::new("Content-Length: 5\r\n\r\n{")).is_none());
    }

    // LSP counts a lone CR as a line end, as servers other than rust-analyzer do.
    #[test]
    fn a_lone_cr_ends_a_line_for_the_server() {
        let d = doc("a\rb\nc");
        assert_eq!(offset_at(&d, 1, 0, Encoding::Utf16), Some(2), "line 1 starts at b");
        assert_eq!(position(&d, 4, Encoding::Utf16), serde_json::json!({"line": 2, "character": 0}), "c is on line 2");
    }

    #[test]
    fn positions_count_utf8_bytes_or_utf16_units_per_line() {
        // "é" is 2 bytes and 1 unit; "𝄞" is 4 bytes and 2 units. CRLF line endings are not part of a line.
        let d = doc("fn a() {}\r\nlet é𝄞x = 1;\r\n");
        let x = d.text().find('x').unwrap();
        assert_eq!(position(&d, x, Encoding::Utf16), serde_json::json!({"line": 1, "character": 7}));
        assert_eq!(position(&d, x, Encoding::Utf8), serde_json::json!({"line": 1, "character": 10}));
        for e in [Encoding::Utf8, Encoding::Utf16] {
            assert_eq!(offset(&d, &position(&d, x, e), e), Some(x));
        }
        // Past the line's end clamps to it; inside a surrogate pair rounds up to the next character.
        assert_eq!(offset(&d, &serde_json::json!({"line": 0, "character": 99}), Encoding::Utf16), Some(9));
        assert_eq!(offset(&d, &serde_json::json!({"line": 1, "character": 6}), Encoding::Utf16), Some(x));
        assert_eq!(offset(&d, &serde_json::json!({"line": 9, "character": 0}), Encoding::Utf16), None);
        assert_eq!(Encoding::negotiated(&serde_json::json!({"positionEncoding": "utf-8"})), Encoding::Utf8);
        assert_eq!(Encoding::negotiated(&serde_json::json!({})), Encoding::Utf16);
    }

    #[test]
    fn file_uris_escape_and_unescape_paths() {
        let p = Path::new("/tmp/a b/ü#?.rs");
        let uri = file_uri(p);
        assert_eq!(uri, "file:///tmp/a%20b/%C3%BC%23%3F.rs");
        assert_eq!(uri_path(&uri).as_deref(), Some(p));
        assert_eq!(uri_path("file://localhost/x").as_deref(), Some(Path::new("/x")));
        assert_eq!(uri_path("rust-analyzer://x"), None);
        assert_eq!(uri_path("file://host/x"), None);
        assert_eq!(uri_path("file:///bad%2"), None);
    }

    #[test]
    fn decoration_responses_are_told_apart_without_parsing_their_results() {
        let reply = br#"{"jsonrpc":"2.0","id":"decor-7","result":[{"label":"x"}]}"#;
        assert_eq!(decoration_id(reply), Some((7, false)));
        assert_eq!(decoration_id(br#"{"jsonrpc":"2.0","id":"decor-8","error":{"code":-32801,"message":"m"}}"#), Some((8, true)));
        assert_eq!(decoration_id(br#"{"jsonrpc":"2.0","id":7,"result":null}"#), None, "another request's answer");
        assert_eq!(decoration_id(br#"{"jsonrpc":"2.0","id":"decor-9","method":"x"}"#), None, "a request from the server");
        assert_eq!(decoration_id(b"not json"), None);
        let mut input = std::io::Cursor::new(b"Content-Length: 2\r\n\r\n{}".to_vec());
        assert_eq!(read_frame(&mut input).as_deref(), Some(&b"{}"[..]));
    }

    /// LSP's own reading: `\n`, `\r\n` and a lone `\r` each end a line. The protocol position of byte `x`.
    fn reference(text: &str, x: usize, e: Encoding) -> (usize, usize) {
        let b = text.as_bytes();
        let (mut line, mut start, mut i) = (0, 0, 0);
        while i < x {
            if b[i] == b'\n' {
                (line, start) = (line + 1, i + 1);
            } else if b[i] == b'\r' {
                if b.get(i + 1) == Some(&b'\n') {
                    if i + 1 < x {
                        (line, start, i) = (line + 1, i + 2, i + 2);
                        continue;
                    }
                    return (line, units(&text[start..i], e));
                }
                (line, start) = (line + 1, i + 1);
            }
            i += 1;
        }
        (line, units(&text[start..x], e))
    }

    #[test]
    fn positions_follow_lsp_line_ends_both_ways() {
        for text in ["a\rb\r\nc\rd", "\u{e9}\r\u{1F600}\r\r\nx\r", "\r\r\n\r", "ab\r\ncd\ref\n\rg", "no lone\r\nhere\n", "x\r"] {
            let d = doc(text);
            for e in [Encoding::Utf8, Encoding::Utf16] {
                for x in (0..=text.len()).filter(|&x| text.is_char_boundary(x)) {
                    let (l, c) = reference(text, x, e);
                    assert_eq!(position(&d, x, e), serde_json::json!({"line": l, "character": c}), "{text:?} at {x} {e:?}");
                    // Between a CR and its LF is the end of the line before, which reads back as the CR's offset.
                    let inside_crlf = x > 0 && text.as_bytes()[x - 1] == b'\r' && text.as_bytes().get(x) == Some(&b'\n');
                    if !inside_crlf {
                        assert_eq!(offset_at(&d, l, c, e), Some(x), "{text:?} back from ({l},{c}) {e:?}");
                    }
                }
            }
        }
    }
}

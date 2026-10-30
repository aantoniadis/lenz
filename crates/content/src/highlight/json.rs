//! Keep object keys distinct from string values and make JSON structure visible.
pub(super) const QUERY: &str = r#"
(pair key: (string) @property)
["{" "}" "[" "]" ":" ","] @punctuation
"#;

/// tree-sitter-json 0.24.8's grammar.js accepts only '-' in exponentPart's signedInteger.
/// Feed an equal-length sign to that parser; queries, diagnostics and saved text still use the original bytes.
pub(super) fn exponent_pluses(text: &str, current: &dyn Fn() -> bool) -> Option<Vec<usize>> {
    let bytes = text.as_bytes();
    let mut positions = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if at.is_multiple_of(4096) && !current() { return None; }
        match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() {
                    if at.is_multiple_of(4096) && !current() { return None; }
                    let byte = bytes[at];
                    at += 1;
                    if byte == b'\\' { at += 1; }
                    else if byte == b'"' { break; }
                }
            }
            b'/' if bytes.get(at + 1) == Some(&b'/') => {
                at += 2;
                while at < bytes.len() && !matches!(bytes[at], b'\r' | b'\n') {
                    if at.is_multiple_of(4096) && !current() { return None; }
                    at += 1;
                }
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at < bytes.len() && !(bytes[at] == b'*' && bytes.get(at + 1) == Some(&b'/')) {
                    if at.is_multiple_of(4096) && !current() { return None; }
                    at += 1;
                }
                at = at.saturating_add(2).min(bytes.len());
            }
            byte => {
                if byte == b'+' && at >= 2 && matches!(bytes[at - 1], b'e' | b'E') && bytes[at - 2].is_ascii_digit() && bytes.get(at + 1).is_some_and(u8::is_ascii_digit) { positions.push(at); }
                at += 1;
            }
        }
    }
    current().then_some(positions)
}

pub(super) fn input<'a>(text: &'a str, positions: &[usize], offset: usize) -> &'a [u8] {
    let next = positions.get(positions.partition_point(|&position| position < offset)).copied();
    if next == Some(offset) { return b"-"; }
    let end = next.unwrap_or(text.len()).min(offset.saturating_add(4096));
    text.as_bytes().get(offset..end).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::*;

    #[test]
    fn json_keys_values_escapes_literals_and_structure_have_distinct_captures() {
        let text = r#"{"key":"value\n","number":12,"flags":[true,false,null]}"#;
        let syntax = parse_with(Language::Json, text.into(), Want::default(), || true).unwrap();
        for (token, capture) in [("key", Capture::Property), ("value", Capture::String), (r#"\n"#, Capture::Escape), ("12", Capture::Number), ("true", Capture::ConstantBuiltin), ("false", Capture::ConstantBuiltin), ("null", Capture::ConstantBuiltin), ("{", Capture::Punctuation), (":", Capture::Punctuation), (",", Capture::Punctuation), ("[", Capture::Punctuation), ("]", Capture::Punctuation), ("}", Capture::Punctuation)] {
            assert_eq!(syntax.parsed.capture_at(0, text.find(token).unwrap()), Some(capture), "{token}");
        }
    }

    #[test]
    fn json_positive_exponents_are_whole_numbers_without_false_syntax_errors() {
        for number in ["1e+999", "-1.2500E+7", "0e+0", "1e-999"] {
            let text = format!("{{\"number\":{number},\"text\":\"1e+9\\n\"}}");
            let syntax = parse_with(Language::Json, text.clone().into(), Want::default(), || true).unwrap();
            assert!(syntax.errors.unwrap().is_empty(), "valid number {number}");
            let start = text.find(number).unwrap();
            for byte in start..start + number.len() { assert_eq!(syntax.parsed.capture_at(0, byte), Some(Capture::Number), "{number} at {byte}"); }
            let steps = selection_steps_while(Language::Json, text.clone().into(), start + 1..start + 1, || true).unwrap();
            assert!(steps.contains(&(start..start + number.len())), "the full number is a source selection step: {number}");
            let quoted = text.rfind("1e+9").unwrap();
            for byte in quoted..quoted + 4 { assert_eq!(syntax.parsed.capture_at(0, byte), Some(Capture::String)); }
            assert_eq!(syntax.parsed.capture_at(0, text.find("\\n").unwrap()), Some(Capture::Escape));
        }
        for text in ["[+1]", "[1e+]", "[1e+-2]", "[1e++2]", "[01e+2]", "[1e+2,]"] {
            assert!(!parse_with(Language::Json, text.into(), Want::default(), || true).unwrap().errors.unwrap().is_empty(), "invalid JSON: {text}");
        }
    }

    #[test]
    fn json_parser_proxy_keeps_quoted_text_comments_and_chunk_boundary_coordinates() {
        let text = " ".repeat(4093) + "[1e+99,\"1e+99\\\" 2e+88\",1e-99,2E+88] /* 3e+7 */ // 4e+6\r\n";
        let original = text.clone();
        let positions = exponent_pluses(&text, &|| true).unwrap();
        assert_eq!(positions, [4096, text.find("2E+88").unwrap() + 2]);
        let mut proxy = Vec::new();
        while proxy.len() < text.len() { proxy.extend_from_slice(input(&text, &positions, proxy.len())); }
        assert_eq!(proxy.len(), text.len());
        for (at, (old, new)) in text.bytes().zip(proxy).enumerate() { assert_eq!(new, if positions.contains(&at) { b'-' } else { old }); }
        assert_eq!(text, original);
        assert!(exponent_pluses(&text, &|| false).is_none());
        assert!(input(&text, &positions, text.len()).is_empty());
        let syntax = parse_with(Language::Json, (" ".repeat(4093) + "[1e+99,2E+88]").into(), Want::default(), || true).unwrap();
        assert!(syntax.errors.unwrap().is_empty());
        assert_eq!(syntax.parsed.capture_at(0, 4096), Some(Capture::Number));
    }
}

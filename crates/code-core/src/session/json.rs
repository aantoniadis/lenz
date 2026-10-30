//! JSON formatting changes whitespace only; keys, duplicates and literal spellings remain source bytes.
use super::format_work::ToolResult;
use crate::{document::Document, edit_ops::IndentUnit};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn format(doc: &Document, unit: &IndentUnit, cancelled: &AtomicBool) -> ToolResult {
    match whitespace(doc.text(), &unit.text, cancelled) {
        Ok(text) => ToolResult::Formatted { tool: "Lenz JSON formatter", edit: super::format_work::difference(doc.text(), text) },
        Err(why) => ToolResult::Failed(format!("JSON formatting: {why}")),
    }
}

fn whitespace(text: &str, indent: &str, cancelled: &AtomicBool) -> Result<String, String> {
    let check = || if cancelled.load(Ordering::Relaxed) { Err("cancelled".to_owned()) } else { Ok(()) };
    check()?;
    if text.len() > crate::document::LARGE_BYTES { return Err("large file: not formatted".into()); }
    // IgnoredAny validates the grammar without constructing values or rounding/rejecting large numbers.
    serde_json::from_str::<serde::de::IgnoredAny>(text).map_err(|e| format!("invalid JSON: {e}"))?;
    check()?;
    let bytes = text.as_bytes();
    let mut endings = bytes.iter().enumerate().filter_map(|(i, b)| match b {
        b'\r' if bytes.get(i + 1) != Some(&b'\n') => Some("\r"),
        b'\n' => Some(if i > 0 && bytes[i - 1] == b'\r' { "\r\n" } else { "\n" }),
        _ => None,
    });
    let fallback = endings.clone().next().unwrap_or("\n");
    let mut lines = 1;
    let mut newline = |out: &mut String, depth: usize| -> Result<(), String> {
        check()?;
        let ending = endings.next().unwrap_or(fallback);
        let added = ending.len().saturating_add(depth.saturating_mul(indent.len()));
        if out.len().saturating_add(added) > crate::document::LARGE_BYTES || lines >= crate::document::LARGE_LINES {
            return Err("formatted output would exceed the editor's large-file limit".into());
        }
        out.push_str(ending);
        for _ in 0..depth { out.push_str(indent); }
        lines += 1;
        Ok(())
    };
    let mut out = String::with_capacity(text.len());
    let (mut at, mut depth, mut previous) = (0, 0usize, 0);
    while at < bytes.len() {
        check()?;
        let start = at;
        let byte = bytes[at];
        at += 1;
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            b'{' | b'[' => {
                out.push(char::from(byte));
                depth += 1;
                let next = bytes[at..].iter().find(|b| !b.is_ascii_whitespace());
                if next != Some(&if byte == b'{' { b'}' } else { b']' }) { newline(&mut out, depth)?; }
            }
            b'}' | b']' => {
                depth -= 1;
                if previous != if byte == b'}' { b'{' } else { b'[' } { newline(&mut out, depth)?; }
                out.push(char::from(byte));
            }
            b',' => { out.push(','); newline(&mut out, depth)?; }
            b':' => out.push_str(": "),
            _ => {
                if byte == b'"' {
                    while at < bytes.len() {
                        if at.is_multiple_of(4096) { check()?; }
                        let b = bytes[at];
                        at += 1;
                        if b == b'\\' { at += 1; }
                        else if b == b'"' { break; }
                    }
                } else {
                    while at < bytes.len() && !matches!(bytes[at], b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}') {
                        if at.is_multiple_of(4096) { check()?; }
                        at += 1;
                    }
                }
                if out.len().saturating_add(at - start) > crate::document::LARGE_BYTES { return Err("formatted output would exceed the editor's large-file limit".into()); }
                out.push_str(text.get(start..at).ok_or_else(|| "invalid JSON token boundary".to_owned())?);
            }
        }
        previous = byte;
    }
    let final_ending = if text.ends_with("\r\n") { "\r\n" } else if text.ends_with('\n') { "\n" } else if text.ends_with('\r') { "\r" } else { "" };
    if out.len() + final_ending.len() > crate::document::LARGE_BYTES { return Err("formatted output would exceed the editor's large-file limit".into()); }
    out.push_str(final_ending);
    check()?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::Style;

    fn formatted(text: &str, style: Style, size: u8) -> String {
        let doc = Document::decode("/fixture/data.json".into(), 1, text.as_bytes());
        let ToolResult::Formatted { edit, .. } = format(&doc, &IndentUnit::new(style, size, 4), &AtomicBool::new(false)) else { panic!("valid JSON must format: {text:?}") };
        let mut out = text.to_owned();
        if let Some(edit) = edit { out.replace_range(edit.range, &edit.text); }
        out
    }

    #[test]
    fn json_format_preserves_duplicate_keys_order_numbers_strings_and_empty_containers() {
        let source = r#"{"z":-0,"a":900719925474099312345678901234567890,"z":1.2300e+999,"text":"é\u0061\"{}[,]","empty":[{},[]],"flags":[true,false,null]}"#;
        let expected = "{\n  \"z\": -0,\n  \"a\": 900719925474099312345678901234567890,\n  \"z\": 1.2300e+999,\n  \"text\": \"é\\u0061\\\"{}[,]\",\n  \"empty\": [\n    {},\n    []\n  ],\n  \"flags\": [\n    true,\n    false,\n    null\n  ]\n}";
        assert_eq!(formatted(source, Style::Space, 2), expected);
        assert_eq!(formatted(expected, Style::Space, 2), expected, "formatting is idempotent");
    }

    #[test]
    fn json_format_preserves_crlf_mixed_endings_and_final_newline_choice() {
        assert_eq!(formatted("{\r\n\"x\":[1,2]}\r\n", Style::Tab, 4), "{\r\n\t\"x\": [\r\n\t\t1,\r\n\t\t2\r\n\t]\r\n}\r\n");
        assert_eq!(formatted("{\r\n\"x\":1,\n\"y\":2\r\n}", Style::Space, 4), "{\r\n    \"x\": 1,\n    \"y\": 2\r\n}");
        for source in ["true", "null\n", " -0 ", "\"x\"\r\n", "[ ]", "{}\r"] {
            let expected = source.trim().to_owned() + if source.ends_with("\r\n") { "\r\n" } else if source.ends_with('\n') { "\n" } else if source.ends_with('\r') { "\r" } else { "" };
            assert_eq!(formatted(source, Style::Space, 2), if source == "[ ]" { "[]".into() } else { expected });
        }
    }

    #[test]
    fn json_format_refuses_invalid_cancelled_and_excessive_output_without_an_edit() {
        for text in ["", "{\"a\":}", "[1,]", "{\"a\":01}", "{} true", "// comment\n{}", "{\"a\": NaN}", "\"\\q\""] {
            let doc = Document::decode("/fixture/bad.json".into(), 1, text.as_bytes());
            assert!(matches!(format(&doc, &IndentUnit::new(Style::Space, 2, 4), &AtomicBool::new(false)), ToolResult::Failed(_)), "{text:?}");
        }
        let doc = Document::decode("/fixture/cancel.json".into(), 1, b"{\"a\":1}");
        assert!(matches!(format(&doc, &IndentUnit::new(Style::Space, 2, 4), &AtomicBool::new(true)), ToolResult::Failed(_)));
        let nested = "[".repeat(4000) + "0" + &"]".repeat(4000);
        let doc = Document::decode("/fixture/deep.json".into(), 1, nested.as_bytes());
        assert!(matches!(format(&doc, &IndentUnit::new(Style::Space, 2, 4), &AtomicBool::new(false)), ToolResult::Failed(_)), "indentation expansion stays bounded");
    }

    #[test]
    #[ignore = "release JSON formatter timing/retention: run alone; JSON_STRESS_SECONDS defaults to 10"]
    fn json_formatter_release_cost_and_retention() {
        use std::time::{Duration, Instant};
        let source = "[".to_owned() + &std::iter::repeat_n(r#"{"key":1e+999,"key":-0,"str":"escape\n"}"#, 10_000).collect::<Vec<_>>().join(",") + "]\r\n";
        let unit = IndentUnit::new(Style::Space, 2, 4);
        let cancelled = AtomicBool::new(false);
        let expected = whitespace(&source, &unit.text, &cancelled).unwrap();
        assert_eq!(expected.matches("\"key\"").count(), 20_000);
        assert_eq!(expected.matches("1e+999").count(), 10_000);
        assert_eq!(expected.matches("-0").count(), 10_000);
        let rss = || std::fs::read_to_string("/proc/self/status").ok().and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmRSS:").and_then(|v| v.split_whitespace().next()?.parse::<f64>().ok())).map(|kib| kib / 1024.0));
        let mut samples = Vec::new();
        for i in 0..33 {
            let now = Instant::now();
            let out = whitespace(&source, &unit.text, &cancelled).unwrap();
            let elapsed = now.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(out, expected);
            assert_eq!(whitespace(&out, &unit.text, &cancelled).unwrap(), expected);
            if i >= 3 { samples.push(elapsed); }
        }
        let seconds = std::env::var("JSON_STRESS_SECONDS").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(10).max(1);
        let started = Instant::now();
        let mut timeline = Vec::new();
        let mut iterations = 0usize;
        while started.elapsed() < Duration::from_secs(seconds) {
            assert_eq!(whitespace(&source, &unit.text, &cancelled).unwrap(), expected);
            iterations += 1;
            if iterations.is_multiple_of(100) { timeline.push((started.elapsed().as_secs_f64(), rss())); }
        }
        eprintln!("JSON_PERF {}", serde_json::json!({"source_bytes":source.len(),"formatted_bytes":expected.len(),"format_ms":samples,"stress_seconds":started.elapsed().as_secs_f64(),"iterations":iterations,"rss_timeline_mib":timeline,"scope":"candidate-only direct in-process formatter, three warmups and thirty timing samples, fixed 10k-object input; excludes jobs, adapters, physical display and save"}));
    }
}

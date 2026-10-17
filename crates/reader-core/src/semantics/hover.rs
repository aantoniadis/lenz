//! A language server's hover answer as Markdown text.

use serde_json::Value;

/// The hover result as Markdown: `MarkupContent` (Markdown as is, plain text fenced), a `MarkedString` (a string,
/// or code in a language, fenced) or a list of them, separated by blank lines. `None` when it says nothing.
pub fn markdown(result: &Value) -> Option<String> {
    fn part(v: &Value) -> Option<String> {
        let text = match v {
            Value::String(s) => s.trim().to_owned(),
            Value::Object(o) => {
                let value = o.get("value")?.as_str()?.trim();
                match (o.get("kind").and_then(Value::as_str), o.get("language").and_then(Value::as_str)) {
                    (Some("markdown"), _) => value.to_owned(),
                    _ if value.is_empty() => String::new(),
                    (Some(_), _) => format!("```text\n{value}\n```"),
                    (None, Some(language)) => format!("```{language}\n{value}\n```"),
                    (None, None) => value.to_owned(),
                }
            }
            _ => return None,
        };
        (!text.is_empty()).then_some(text)
    }
    let parts: Vec<String> = match &result["contents"] {
        Value::Array(items) => items.iter().filter_map(part).collect(),
        contents => part(contents).into_iter().collect(),
    };
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_hover_shape_becomes_markdown() {
        assert_eq!(markdown(&json!({ "contents": { "kind": "markdown", "value": "**x**" } })).as_deref(), Some("**x**"));
        assert_eq!(markdown(&json!({ "contents": { "kind": "plaintext", "value": "a < b" } })).as_deref(), Some("```text\na < b\n```"));
        assert_eq!(markdown(&json!({ "contents": { "language": "rust", "value": "fn f()" } })).as_deref(), Some("```rust\nfn f()\n```"));
        assert_eq!(markdown(&json!({ "contents": ["one", { "language": "py", "value": "x" }] })).as_deref(), Some("one\n\n```py\nx\n```"));
        assert_eq!(markdown(&json!({ "contents": "  plain  " })).as_deref(), Some("plain"));
        assert_eq!(markdown(&json!(null)), None);
        assert_eq!(markdown(&json!({ "contents": "" })), None);
        assert_eq!(markdown(&json!({ "contents": [] })), None);
    }
}

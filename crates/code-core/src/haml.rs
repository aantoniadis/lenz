//! Haml source regions. Markup stays Haml; only exact Ruby regions reach Ruby LSP.
use super::*;

/// One Haml line read for its Ruby: regions relative to the line's text after its indentation.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct HamlLine {
    pub(super) spans: Vec<Range<usize>>,
    /// It ends with ` |`: the next lines ending so continue its script.
    pub(super) continues: bool,
    /// It holds content (text, a script, a self-closing tag): nothing may be nested under it.
    inline: bool,
    /// `-#`: it and the lines nested under it are a comment.
    pub(super) comment: bool,
    /// Its Ruby is output (`=`, `#{…}`), not a silent `-` script.
    output: bool,
    block: bool,
    branch: bool,
    hashes: Vec<usize>,
}

impl HamlLine {
    pub(super) fn shape(&self) -> (bool, bool, bool, bool) { (self.block, self.branch, self.comment, self.inline) }
}

pub(super) fn ends_with_pipe(text: &str) -> bool {
    text.strip_suffix('|').is_some_and(|t| t.ends_with([' ', '\t']))
}

/// A line continuing a `|` script: all of it but the `|`.
pub(super) fn haml_continuation(text: &str) -> HamlLine {
    HamlLine { spans: std::iter::once(0..text.len().saturating_sub(usize::from(ends_with_pipe(text)))).collect(), continues: ends_with_pipe(text) || text.trim_end().ends_with(','), inline: true, comment: false, output: false, ..HamlLine::default() }
}

/// Ruby that opens an indentation-controlled block (`if`, `each do |x|`).
fn opens_block(code: &str) -> bool {
    let code = code.trim();
    let word = code.split(|c: char| !(c.is_alphanumeric() || c == '_')).next().unwrap_or_default();
    if matches!(word, "if" | "unless" | "case" | "while" | "until" | "for" | "begin") { return true; }
    let tail = match code.strip_suffix('|').and_then(|c| c.rfind('|').and_then(|i| c.get(..i))) { Some(before) => before.trim_end(), None => code };
    tail == "do" || tail.ends_with(" do") || tail.ends_with(")do") || tail.ends_with('{')
}

/// A `=`/`-` script's Ruby, from `at` in `text` (after its marker and one optional space).
fn haml_script(text: &str, at: usize, line: usize) -> Result<HamlLine, String> {
    let output = text.as_bytes().get(at.saturating_sub(1)) == Some(&b'=');
    let code_start = at + usize::from(text.as_bytes().get(at) == Some(&b' '));
    let code = text.get(code_start..).unwrap_or_default();
    let block = opens_block(code);
    let branch = matches!(code.split_whitespace().next(), Some("else" | "elsif" | "when" | "in" | "rescue" | "ensure"));
    let pipe = !block && ends_with_pipe(code);
    let continues = pipe || code.trim_end().ends_with(',');
    let end = text.len() - usize::from(pipe);
    let _ = line;
    Ok(HamlLine { spans: std::iter::once(code_start..end).collect(), continues, inline: !(block || branch), comment: false, output, block, branch, hashes: Vec::new() })
}

/// Plain text from `at` in `text`: its `#{…}` interpolations' Ruby.
fn haml_text(text: &str, at: usize, line: usize) -> Result<HamlLine, String> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = at;
    while let Some(found) = text.get(i..).and_then(|t| t.find("#{")) {
        let open = i + found;
        if bytes.get(..open).unwrap_or_default().iter().rev().take_while(|&&b| b == b'\\').count() % 2 == 1 { i = open + 2; continue; }
        let inner = open + 2;
        match balanced(text, open + 1) {
            Some(c) => { spans.push(inner..c); i = c + 1; }
            None => return Err(format!("line {line}: unfinished `#{{…}}` interpolation is not mapped yet")),
        }
    }
    if ends_with_pipe(text) { return Err(format!("line {line}: multiline (`|`) text is not mapped yet")); }
    Ok(HamlLine { spans, continues: false, inline: true, comment: false, output: true, ..HamlLine::default() })
}

/// Match (), [] and {} without treating brackets in Ruby strings as delimiters. A depth limit keeps
/// malformed input bounded. Percent literals and heredocs are refused rather than guessed.
fn balanced(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let (close, quoted) = match bytes.get(open)? {
        b'(' => (b')', false), b'[' => (b']', false), b'{' => (b'}', false),
        q @ (b'\'' | b'"' | b'`') => (*q, true), _ => return None,
    };
    let mut stack = vec![(close, quoted)];
    let mut i = open + 1;
    while let (Some(&b), Some(&(close, quoted))) = (bytes.get(i), stack.last()) {
        if quoted {
            if b == b'\\' { i += 2; continue; }
            if b == close { stack.pop(); }
            else if close != b'\'' && b == b'#' && bytes.get(i + 1) == Some(&b'{') { stack.push((b'}', false)); i += 1; }
        } else {
            match b {
                q @ (b'\'' | b'"' | b'`') => stack.push((q, true)),
                // Regex/percent literals need Ruby's lexer to distinguish them from operators.
                b'/' => return None,
                b'%' | b'<' if bytes.get(i + 1).is_some_and(|b| b.is_ascii_alphabetic() || *b == b'<') => return None,
                b'#' => { while bytes.get(i).is_some_and(|b| *b != b'\n') { i += 1; } continue; }
                b'(' => stack.push((b')', false)), b'[' => stack.push((b']', false)), b'{' => stack.push((b'}', false)),
                b')' | b']' | b'}' => { if b != close { return None; } stack.pop(); }
                _ => {}
            }
        }
        if stack.is_empty() { return Some(i); }
        if stack.len() > 128 { return None; }
        i += 1;
    }
    None
}

/// Ruby values of Haml's HTML-style attributes. Quoted static values remain markup; interpolation
/// is mapped inside double quotes. Unquoted expressions end at top-level whitespace.
fn attribute_values(text: &str, start: usize, end: usize, line: usize) -> Result<Vec<Range<usize>>, String> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = start;
    while i < end {
        while i < end && bytes.get(i).copied().unwrap_or_default().is_ascii_whitespace() { i += 1; }
        let name_start = i;
        while i < end && (bytes.get(i).copied().unwrap_or_default().is_ascii_alphanumeric() || matches!(bytes.get(i).copied().unwrap_or_default(), b'_' | b'-' | b':' | b'@' | b'#' | b'.')) { i += 1; }
        if i == name_start && i < end { return Err(format!("line {line}: invalid Haml attribute name")); }
        if i == end { break; }
        while i < end && bytes.get(i).copied().unwrap_or_default().is_ascii_whitespace() { i += 1; }
        if bytes.get(i) != Some(&b'=') { continue; } // Boolean attribute.
        i += 1;
        while i < end && bytes.get(i).copied().unwrap_or_default().is_ascii_whitespace() { i += 1; }
        let from = i;
        if let Some(&q @ (b'\'' | b'"')) = bytes.get(i) {
            i = balanced(text, i).filter(|&i| i < end).ok_or_else(|| format!("line {line}: unfinished quoted attribute"))?;
            if q == b'"' {
                let read = haml_text(text.get(from + 1..i).unwrap_or_default(), 0, line)?;
                spans.extend(read.spans.into_iter().map(|r| r.start + from + 1..r.end + from + 1));
            }
            i += 1;
        } else {
            while i < end && (bytes.get(i).copied().unwrap_or_default().is_ascii_alphanumeric() || matches!(bytes.get(i).copied().unwrap_or_default(), b'_' | b'@' | b'$')) { i += 1; }
            if i == from || (i < end && !bytes.get(i).copied().unwrap_or_default().is_ascii_whitespace()) {
                return Err(format!("line {line}: Haml parenthesized values must be variables or quoted strings; use {{…}} for Ruby expressions"));
            }
            spans.push(from..i);
        }
    }
    Ok(spans)
}

/// One Haml line's Ruby, from its text after the indentation (`line`: its number, for messages).
pub(super) fn haml_line(text: &str, line: usize) -> Result<HamlLine, String> {
    let bytes = text.as_bytes();
    let unsupported = |what: &str| Err(format!("line {line}: {what} not mapped yet"));
    match bytes {
        [b'-', b'#', ..] => Ok(HamlLine { comment: true, ..HamlLine::default() }),
        [b'-', ..] => haml_script(text, 1, line),
        [b'=', b'=', ..] => haml_text(text, 2, line),
        [b'=', ..] => haml_script(text, 1, line),
        [b'&' | b'!', b'=', ..] => haml_script(text, 2, line),
        [b'!', b'!', b'!', ..] => Ok(HamlLine { inline: true, ..HamlLine::default() }),
        [b'~', ..] => haml_script(text, 1, line),
        [b':', ..] => unsupported("filters (`:javascript`, `:markdown`…) are"),
        [b'/', ..] => haml_text(text, 1, line),
        [b'\\', ..] => haml_text(text, 1, line),
        [b'!' | b'&' | b'[', ..] => unsupported("`!`/`&` text and object references are"),
        [b'%', ..] => haml_tag(text, line),
        [b'.' | b'#', next, ..] if next.is_ascii_alphanumeric() || matches!(next, b'_' | b'-') => haml_tag(text, line),
        _ => haml_text(text, 0, line),
    }
}

/// A tag line: `%name`, `.class`/`#id` chains, static `(…)` attributes, then nothing, a script or text.
fn haml_tag(text: &str, line: usize) -> Result<HamlLine, String> {
    let bytes = text.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-');
    let mut i = 0;
    if bytes.first() == Some(&b'%') {
        i = 1;
        while bytes.get(i).is_some_and(|b| word(*b) || *b == b':') { i += 1; }
        if i == 1 { return Err(format!("line {line}: a `%` without a tag name is not mapped")); }
    }
    while matches!(bytes.get(i), Some(b'.' | b'#')) && bytes.get(i + 1).is_some_and(|b| word(*b)) {
        i += 1;
        while bytes.get(i).is_some_and(|b| word(*b)) { i += 1; }
    }
    let mut attrs = Vec::new();
    let mut hashes = Vec::new();
    while matches!(bytes.get(i), Some(b'(' | b'{')) {
        let close = balanced(text, i).ok_or_else(|| format!("line {line}: attributes over several lines or unfinished Ruby are not mapped yet"))?;
        if bytes.get(i) == Some(&b'{') {
            hashes.push(i);
            attrs.push(i..close + 1);
        } else { attrs.extend(attribute_values(text, i + 1, close, line)?); }
        i = close + 1;
    }
    if bytes.get(i) == Some(&b'[') { return Err(format!("line {line}: object references (`[…]`) are not mapped yet")); }
    while matches!(bytes.get(i), Some(b'<' | b'>')) { i += 1; }
    let mut read = match bytes.get(i..).unwrap_or_default() {
        [] => Ok(HamlLine::default()),
        [b'/', rest @ ..] if rest.iter().all(u8::is_ascii_whitespace) => Ok(HamlLine { inline: true, ..HamlLine::default() }),
        [b'=', b'=', ..] => haml_text(text, i + 2, line),
        [b'=', ..] => haml_script(text, i + 1, line),
        [b'&' | b'!', b'=', ..] => haml_script(text, i + 2, line),
        [b' ' | b'\t', ..] => haml_text(text, i + 1, line),
        _ => Err(format!("line {line}: this tag form is not mapped yet")),
    }?;
    attrs.append(&mut read.spans);
    read.spans = attrs;
    read.hashes = hashes;
    Ok(read)
}

/// A Haml template's Ruby, line by line; any construct outside the mapped contexts refuses the whole template.
pub(super) fn haml_ruby(text: &str) -> Result<Projected, String> {
    let mut out = Projected::default();
    let (mut continued, mut comment, mut previous): (bool, Option<usize>, Option<(usize, bool)>) = (false, None, None);
    let mut pipe_continued = false;
    let mut offset = 0;
    let mut blocks: Vec<usize> = Vec::new();
    for (number, raw) in text.split_inclusive('\n').enumerate() {
        let number = number + 1;
        let start = offset;
        offset += raw.len();
        let body = raw.trim_end_matches(['\n', '\r']);
        let spaces = body.len() - body.trim_start_matches([' ', '\t']).len();
        let indent = body.get(..spaces).unwrap_or_default();
        let rest = body.get(spaces..).unwrap_or_default();
        if rest.is_empty() { continue; }
        if indent.contains('\t') { return Err(format!("line {number}: tab indentation is not mapped yet")); }
        if let Some(at) = comment {
            if spaces > at { continue; }
            comment = None;
        }
        let continuation = continued && (!pipe_continued || ends_with_pipe(rest));
        let read = if continuation { haml_continuation(rest) } else {
            if previous.is_some_and(|(at, inline)| inline && spaces > at) {
                return Err(format!("line {number}: lines nested under content or Ruby (blocks) are not mapped yet"));
            }
            haml_line(rest, number)?
        };
        if !continuation {
            while blocks.last().is_some_and(|&at| spaces < at || (spaces == at && !read.branch)) {
                blocks.pop(); out.text.push_str("end\n");
            }
            if read.branch && blocks.last() != Some(&spaces) { return Err(format!("line {number}: Ruby branch has no matching block")); }
            if read.block { blocks.push(spaces); }
        }
        continued = read.continues;
        pipe_continued = continued && ends_with_pipe(rest);
        if read.comment { comment = Some(spaces); previous = None; continue; }
        if !continuation { previous = Some((spaces, read.inline)); }
        for span in read.spans {
            let source = start + spaces + span.start..start + spaces + span.end;
            let hash = read.hashes.contains(&span.start);
            if hash { out.text.push('('); }
            let generated = out.text.len();
            out.text.push_str(text.get(source.clone()).unwrap_or_default());
            if hash { out.text.push(')'); }
            out.text.push('\n');
            out.spans.push(Span { source, generated, unit: start..start + body.len(), continued: continuation, continues: read.continues, output: read.output, locals: false });
        }
    }
    for _ in blocks { out.text.push_str("end\n"); }
    Ok(out)
}


const TAGS: &[&str] = &["a", "article", "aside", "b", "body", "br", "button", "code", "div", "em", "fieldset", "footer", "form", "h1", "h2", "h3", "head", "header", "hr", "html", "img", "input", "label", "li", "link", "main", "meta", "nav", "ol", "option", "p", "pre", "script", "section", "select", "span", "strong", "style", "table", "tbody", "td", "textarea", "th", "thead", "title", "tr", "ul"];
const ATTRIBUTES: &[&str] = &["alt", "aria-label", "class", "disabled", "for", "height", "href", "id", "name", "placeholder", "rel", "role", "src", "style", "target", "title", "type", "value", "width"];

/// A Haml tag or HTML-style attribute name at the caret. Values and Ruby are never markup completion contexts.
pub(crate) fn markup(text: &str, caret: usize) -> Option<(usize, usize, &'static [&'static str], bool)> {
    if !text.is_char_boundary(caret) { return None; }
    let word = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':');
    let bytes = text.as_bytes();
    let mut start = caret;
    while start > 0 && bytes.get(start - 1).is_some_and(|b| word(*b)) { start -= 1; }
    let mut end = caret;
    while bytes.get(end).is_some_and(|b| word(*b)) { end += 1; }
    let indent = text.len() - text.trim_start_matches(' ').len();
    if start == indent + 1 && bytes.get(indent) == Some(&b'%') { return Some((start, end, TAGS, false)); }
    let prefix = text.get(indent..start)?;
    if !prefix.starts_with(['%', '.', '#']) { return None; }
    let open = prefix.find('(')?;
    if !prefix.get(..open)?.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'%' | b'.' | b'#' | b'_' | b'-' | b':')) { return None; }
    let preceding = prefix.as_bytes().last().copied()?;
    if !preceding.is_ascii_whitespace() && preceding != b'(' { return None; }
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for b in prefix.bytes().skip(open + 1) {
        if escaped { escaped = false; continue; }
        if let Some(q) = quote {
            if b == b'\\' { escaped = true; } else if b == q { quote = None; }
        } else {
            match b {
                b'\'' | b'"' => quote = Some(b),
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => { depth = depth.checked_sub(1)?; }
                _ => {}
            }
        }
    }
    if quote.is_some() || depth != 0 { return None; }
    Some((start, end, ATTRIBUTES, true))
}

/// Enter retains indentation and adds one spaces-only level after a nesting construct. An empty
/// indented line dedents by one level. Haml's indentation is structural, so tabs are never inserted.
pub(crate) fn indent(before: &str, step: usize) -> String {
    let spaces = before.len() - before.trim_start_matches(' ').len();
    let body = before.get(spaces..).unwrap_or_default();
    let width = if body.is_empty() { spaces.saturating_sub(step) }
        else if body.starts_with(':') || body.starts_with("-#") || haml_line(body, 0).is_ok_and(|l| !l.inline) { spaces.saturating_add(step) }
        else { spaces };
    " ".repeat(width)
}

/// A Rails view's Ruby is instance code with the application's helpers in scope. Give Ruby LSP
/// that lexical context without booting Rails or running the template. Each URI has its own class,
/// so including a controller's helper cannot leak into another view. No source is mapped to this code.
pub(super) fn rails_context(projected: &mut Projected, path: &Path) {
    let parts: Vec<_> = path.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    let Some(at) = parts.windows(2).position(|p| p.first().is_some_and(|c| c == "app") && p.get(1).is_some_and(|c| c == "views")) else { return };
    let mut hash = 0xcbf29ce484222325u64;
    for b in path.as_os_str().as_encoded_bytes() { hash = hash.wrapping_mul(0x100000001b3) ^ u64::from(*b); }
    let mut prefix = format!("class LenzHamlView{hash:016x} < ::ActionView::Base\ninclude ::ApplicationHelper\n");
    if let Some(controller) = parts.get(at + 2).filter(|s| !s.contains('.')) {
        let mut name = String::new();
        for word in controller.split('_') {
            let mut chars = word.chars();
            if let Some(c) = chars.next() { name.extend(c.to_uppercase()); name.extend(chars); }
        }
        if name.chars().all(|c| c.is_ascii_alphanumeric()) && name.starts_with(|c: char| c.is_ascii_uppercase()) {
            prefix.push_str(&format!("include ::{name}Helper\n"));
        }
    }
    prefix.push_str("def __lenz_haml_render__\n");
    for span in &mut projected.spans { span.generated += prefix.len(); }
    prefix.push_str(&projected.text); prefix.push_str("end\nend\n");
    projected.text = prefix;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markup_suggestions_never_override_ruby_calls_or_attribute_values() {
        for text in ["%p= greet( na", "%a(href=foo( ti", "%a(title=\"not an attr na", "- greet( na"] {
            assert!(markup(text, text.len()).is_none(), "{text}");
        }
        let text = "%a(title=\"𝄞\" hre";
        assert!(markup(text, text.len()).is_some_and(|(_, _, _, attribute)| attribute));
    }

    #[test]
    fn nested_blocks_attributes_and_interpolation_keep_exact_regions() {
        let text = "%ul\r\n  - items.each do |item|\r\n    - if item\r\n      %li{title: item.title}= item.name\r\n    - else\r\n      %li Hello #{item.fetch({name: \"𝄞\"})}\r\n";
        let out = haml_ruby(text).unwrap();
        assert_eq!(out.text.matches("end\n").count(), 2);
        assert!(out.text.contains("else\n"));
        for span in &out.spans {
            assert_eq!(&out.text[span.generated..span.generated + span.source.len()], &text[span.source.clone()]);
        }
        assert!(out.text.contains("item.title"));
        assert!(out.text.contains("item.fetch({name: \"𝄞\"})"));
    }
    #[test]
    fn dynamic_parenthesized_attributes_keep_expression_boundaries() {
        let text = "%a(href=url title=\"#{item.title}\") Link\n";
        let out = haml_ruby(text).unwrap();
        assert!(out.text.contains("url\n"));
        assert!(out.text.contains("item.title"));
        assert!(!out.text.contains("href="));
    }
    #[test]
    fn comma_continuations_preserve_ruby_newlines_without_treating_markup_as_code() {
        let out = haml_ruby("= greet(\"𝄞\",\r\n  excited: true)\r\n%p after\r\n").unwrap();
        assert!(out.text.contains("greet(\"𝄞\",\nexcited: true)\n"));
        assert!(!out.text.contains("%p"));
    }
}



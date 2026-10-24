//! Snippet text: the placeholders of a language server's snippet body, expanded for one insertion.
//!
//! Supported syntax (the language servers' common subset): `$1`, `${1}`, `${1:text}`, `${1|one,two|}`, the same
//! forms for a name (`$name`, `${name:text}`) and `$0`, the final caret. `\$`, `\}` and `\\` are literal
//! characters. Everything else refuses the whole body, so a suggestion never inserts half of its text.
//!
//! Visit order: numbered placeholders in ascending order, then named ones in the order they first appear, then the
//! final caret (`$0`, or the end of the inserted text when the body has none). A placeholder written more than once
//! is one stop that visits every occurrence together.
use std::ops::Range;

use crate::edit_ops::IndentUnit;
use crate::preferences::Style;

/// Where a snippet is inserted: the file's indentation step, the indentation of the line it starts on, and the
/// document's line ending.
pub struct Layout<'a> {
    pub indent: &'a IndentUnit,
    /// The leading whitespace of the line the snippet starts on: every inserted line after the first starts there.
    pub base: &'a str,
    /// The document's existing line ending; inserted lines use it and never mix.
    pub newline: &'a str,
}

/// An expanded body: its text, and its stops in visit order (one range per occurrence of the placeholder).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Expanded {
    pub text: String,
    pub stops: Vec<Vec<Range<usize>>>,
}

/// A parsed snippet body; parsing refuses bodies whose meaning this editor does not implement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    nodes: Vec<Node>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Node {
    Text(String),
    /// A placeholder: its key, and the text it starts with (empty for a bare `$1`).
    Stop { key: Key, text: String },
}

/// What a placeholder is called. The final caret sorts last whatever number it is written with.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Key {
    Number(u32),
    Name(String),
    End,
}

impl Snippet {
    /// Parse a body, or say why this editor will not expand it.
    pub fn parse(body: &str) -> Result<Self, &'static str> {
        let body = body.replace("\r\n", "\n");
        let mut cursor = Cursor { chars: body.char_indices().peekable() };
        let mut nodes = Vec::new();
        let mut text = String::new();
        while let Some(c) = cursor.next() {
            match c {
                '\\' => cursor.literal(&mut text),
                '$' => cursor.placeholder(&mut text, &mut nodes)?,
                c => text.push(c),
            }
        }
        if !text.is_empty() { nodes.push(Node::Text(text)); }
        Ok(Self { nodes })
    }

    /// The body as it is inserted at `layout`, with every placeholder's range in the inserted text.
    pub fn expand(&self, layout: &Layout<'_>) -> Expanded {
        let mut writer = Writer { out: String::new(), layout, first_line: true, prefix_done: false, spaces: String::new() };
        let mut found: Vec<(Key, Vec<Range<usize>>)> = Vec::new();
        for node in &self.nodes {
            match node {
                Node::Text(text) => writer.text(text),
                Node::Stop { key, .. } => {
                    let start = writer.position();
                    writer.text(self.default_text(key));
                    let end = writer.position();
                    add(&mut found, key.clone(), start..end);
                }
            }
        }
        if !found.iter().any(|(key, _)| *key == Key::End) {
            let end = writer.position();
            add(&mut found, Key::End, end..end);
        }
        let mut stops: Vec<Vec<Range<usize>>> = Vec::new();
        let numbers = {
            let mut numbers: Vec<(u32, &Vec<Range<usize>>)> = found.iter().filter_map(|(key, ranges)| match key {
                Key::Number(n) => Some((*n, ranges)),
                _ => None,
            }).collect();
            numbers.sort_by_key(|(n, _)| *n);
            numbers.into_iter().map(|(_, ranges)| ranges.clone()).collect::<Vec<_>>()
        };
        stops.extend(numbers);
        stops.extend(found.iter().filter(|(key, _)| matches!(key, Key::Name(_))).map(|(_, ranges)| ranges.clone()));
        stops.extend(found.iter().filter(|(key, _)| *key == Key::End).map(|(_, ranges)| ranges.clone()));
        for stop in &mut stops {
            stop.sort_by_key(|range| (range.start, range.end));
            stop.dedup();
        }
        Expanded { text: writer.out, stops }
    }
}

impl Snippet {
    /// A placeholder's text: the first one written for its key, so every occurrence of a repeated placeholder is
    /// inserted with the same text (a bare `$1` after `${1:sum}` also says `sum`).
    fn default_text(&self, key: &Key) -> &str {
        self.nodes.iter().filter_map(|node| match node {
            Node::Stop { key: seen, text } if seen == key && !text.is_empty() => Some(text.as_str()),
            _ => None,
        }).next().unwrap_or("")
    }
}

fn add(found: &mut Vec<(Key, Vec<Range<usize>>)>, key: Key, range: Range<usize>) {
    match found.iter_mut().find(|(seen, _)| *seen == key) {
        Some((_, ranges)) => ranges.push(range),
        None => found.push((key, vec![range])),
    }
}

/// The body's characters, and what a `$` or a `\` just read means.
struct Cursor<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
}

impl Cursor<'_> {
    fn next(&mut self) -> Option<char> { self.chars.next().map(|(_, c)| c) }
    fn eat(&mut self, c: char) -> bool {
        if self.chars.peek().is_some_and(|(_, next)| *next == c) { self.chars.next(); true } else { false }
    }
    /// The character after a `\`: the three escapes are literal, anything else keeps its backslash.
    fn literal(&mut self, text: &mut String) {
        match self.next() {
            Some(c @ ('$' | '}' | '\\')) => text.push(c),
            Some(c) => { text.push('\\'); text.push(c); }
            None => text.push('\\'),
        }
    }
    fn placeholder(&mut self, text: &mut String, nodes: &mut Vec<Node>) -> Result<(), &'static str> {
        let braced = self.eat('{');
        let key = match self.chars.peek().map(|(_, c)| *c) {
            Some(c) if c.is_ascii_digit() => {
                let mut number = String::new();
                while let Some((_, c)) = self.chars.peek().copied() {
                    if !c.is_ascii_digit() { break; }
                    number.push(c);
                    self.chars.next();
                }
                // `$0` is the final caret, whatever its number sorts like.
                match number.parse().unwrap_or(u32::MAX) {
                    0 => Key::End,
                    number => Key::Number(number),
                }
            }
            Some(c) if is_name_start(c) => Key::Name(self.name()?),
            _ if braced => return Err("A placeholder needs a number or name"),
            // A `$` that names nothing is itself, as is one before a space or a punctuation mark.
            _ => { text.push('$'); return Ok(()); }
        };
        if !braced {
            // `$1` and `$name` have no default text; whatever follows them is ordinary text.
            flush(text, nodes);
            nodes.push(Node::Stop { key, text: String::new() });
            return Ok(());
        }
        let body = match self.chars.peek().map(|(_, c)| *c) {
            Some('}') => { self.chars.next(); String::new() }
            Some(':') => { self.chars.next(); self.default_text()? }
            Some('|') => { self.chars.next(); self.choice()? }
            Some('/') => return Err("Snippet placeholder transforms are not supported"),
            _ => return Err("The snippet ends inside a placeholder"),
        };
        if key == Key::End && !body.is_empty() { return Err("The final placeholder $0 cannot hold text"); }
        flush(text, nodes);
        nodes.push(Node::Stop { key, text: body });
        Ok(())
    }
    fn name(&mut self) -> Result<String, &'static str> {
        let mut name = String::new();
        while let Some((_, c)) = self.chars.peek().copied() {
            if !(c.is_alphanumeric() || c == '_') { break; }
            name.push(c);
            self.chars.next();
        }
        if name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
            return Err("Snippet variables are not supported");
        }
        Ok(name)
    }
    /// The text after `${1:`, up to its closing brace. A placeholder inside it is not supported.
    fn default_text(&mut self) -> Result<String, &'static str> {
        let mut text = String::new();
        loop {
            match self.next() {
                None => return Err("The snippet ends inside a placeholder"),
                Some('\\') => self.literal(&mut text),
                Some('$') => {
                    let nested = self.chars.peek().is_some_and(|(_, c)| *c == '{' || c.is_ascii_digit() || is_name_start(*c));
                    if nested { return Err("Snippet placeholders cannot contain another placeholder"); }
                    text.push('$');
                }
                Some('}') => return Ok(text),
                Some(c) => text.push(c),
            }
        }
    }
    /// The choices `${1|one,two|}`; the first one is what is inserted.
    fn choice(&mut self) -> Result<String, &'static str> {
        let mut first = String::new();
        let mut chosen = true;
        loop {
            match self.next() {
                None => return Err("The snippet ends inside a choice"),
                Some('\\') => match self.next() {
                    Some(c @ (',' | '|' | '\\')) => { if chosen { first.push(c); } }
                    Some(c) => { if chosen { first.push('\\'); first.push(c); } }
                    None => return Err("The snippet ends inside a choice"),
                },
                Some('|') if self.eat('}') => return Ok(first),
                Some('|') => return Err("The snippet ends inside a choice"),
                Some(',') => chosen = false,
                Some(c) => { if chosen { first.push(c); } }
            }
        }
    }
}

fn is_name_start(c: char) -> bool { c.is_alphabetic() || c == '_' }

/// The text so far, before a stop node: the writer emits it in order.
fn flush(text: &mut String, nodes: &mut Vec<Node>) {
    if !text.is_empty() { nodes.push(Node::Text(std::mem::take(text))); }
}

/// Emits body text with the file's indentation: a new line starts at the snippet line's own indentation plus the
/// body's leading indentation converted to the file's step, and an empty line stays empty.
struct Writer<'a> {
    out: String,
    layout: &'a Layout<'a>,
    first_line: bool,
    /// Whether the current line's indentation has been written.
    prefix_done: bool,
    /// The current line's leading whitespace, held back until its first other character (an empty line stays empty).
    spaces: String,
}

impl Writer<'_> {
    fn text(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => {
                    self.spaces.clear();
                    self.prefix_done = false;
                    self.first_line = false;
                    self.out.push_str(self.layout.newline);
                }
                c @ (' ' | '\t') if !self.prefix_done => self.spaces.push(c),
                c => {
                    self.flush();
                    self.out.push(c);
                }
            }
        }
    }
    fn flush(&mut self) {
        if self.prefix_done { return; }
        if !self.first_line { self.out.push_str(self.layout.base); }
        self.out.push_str(&step(&self.spaces, self.layout.indent));
        self.spaces.clear();
        self.prefix_done = true;
    }
    fn position(&mut self) -> usize {
        self.flush();
        self.out.len()
    }
}

/// The body's leading whitespace in the file's own step: a tab is one step, `size` spaces are one step, and spaces
/// that do not fill a step stay spaces (alignment).
fn step(spaces: &str, unit: &IndentUnit) -> String {
    let (mut levels, mut extra) = (0, 0);
    for c in spaces.chars() {
        if c == '\t' { levels += 1 } else { extra += 1 }
    }
    let size = unit.size.max(1);
    levels += extra / size;
    extra %= size;
    let mut out = match unit.style {
        Style::Tab => "\t".repeat(levels),
        Style::Space => " ".repeat(levels * size),
    };
    out.push_str(&" ".repeat(extra));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preferences::Style;

    fn layout<'a>(indent: &'a IndentUnit, base: &'a str) -> Layout<'a> {
        Layout { indent, base, newline: "\n" }
    }

    fn expand(body: &str, indent: &IndentUnit, base: &str) -> Expanded {
        Snippet::parse(body).expect("the body parses").expand(&layout(indent, base))
    }

    fn spaces(size: u8) -> IndentUnit { IndentUnit::new(Style::Space, size, size) }

    #[test]
    fn numbered_placeholders_are_visited_in_order_and_the_final_caret_last() {
        let unit = spaces(4);
        let expanded = expand("fn ${2:name}(${1:x}) { $0 }", &unit, "");
        assert_eq!(expanded.text, "fn name(x) {  }");
        assert_eq!(expanded.stops, vec![vec![8..9], vec![3..7], vec![13..13]], "x, then name, then the end");
    }

    #[test]
    fn a_repeated_placeholder_is_one_stop_with_every_occurrence() {
        let expanded = expand("$1 + $1 = ${1:sum}", &spaces(4), "");
        assert_eq!(expanded.text, "sum + sum = sum");
        assert_eq!(expanded.stops, vec![vec![0..3, 6..9, 12..15], vec![15..15]], "one stop visits all three, then the implicit end");
    }

    #[test]
    fn named_placeholders_follow_the_numbered_ones_in_first_appearance_order() {
        let expanded = expand("a ${b:beta} $1 ${a:alpha}", &spaces(4), "");
        assert_eq!(expanded.text, "a beta  alpha", "a bare $1 inserts nothing of its own");
        assert_eq!(expanded.stops, vec![vec![7..7], vec![2..6], vec![8..13], vec![13..13]]);
    }

    #[test]
    fn a_body_without_placeholders_ends_at_the_caret_after_it() {
        let expanded = expand("log()", &spaces(4), "");
        assert_eq!(expanded.text, "log()");
        assert_eq!(expanded.stops, vec![vec![5..5]]);
    }

    #[test]
    fn inserted_lines_take_the_start_lines_indentation_and_the_files_step() {
        let body = "if (x) {\n\tbody();\n\tif (y) {\n\t\tinner();\n\t}\n}";
        let expanded = expand(body, &spaces(2), "    ");
        assert_eq!(expanded.text, "if (x) {\n      body();\n      if (y) {\n        inner();\n      }\n    }");
        // A tab-style file keeps tabs, after the start line's own indentation.
        let tabs = IndentUnit::new(Style::Tab, 4, 4);
        assert_eq!(expand(body, &tabs, "\t").text, "if (x) {\n\t\tbody();\n\t\tif (y) {\n\t\t\tinner();\n\t\t}\n\t}");
        // An empty line stays empty: no trailing indentation is invented.
        assert_eq!(expand("a();\n\nb();", &spaces(2), "  ").text, "a();\n\n  b();");
    }

    #[test]
    fn alignment_spaces_beyond_a_step_stay_as_written() {
        let expanded = expand("call(\n\t one,\n\t two);", &spaces(4), "");
        assert_eq!(expanded.text, "call(\n     one,\n     two);");
    }

    #[test]
    fn line_breaks_follow_the_document() {
        let unit = spaces(4);
        let expanded = Snippet::parse("a();\nb();").unwrap().expand(&Layout { indent: &unit, base: "", newline: "\r\n" });
        assert_eq!(expanded.text, "a();\r\nb();");
    }

    #[test]
    fn escapes_are_literal_and_only_real_placeholders_are_stops() {
        let expanded = expand(r"a \$1 b \} c \\ d $ 5", &spaces(4), "");
        assert_eq!(expanded.text, r"a $1 b } c \ d $ 5");
        assert_eq!(expanded.stops, vec![vec![18..18]], "only the implicit end");
    }

    #[test]
    fn a_choice_inserts_its_first_value() {
        let expanded = expand("${1|one,two|}", &spaces(4), "");
        assert_eq!(expanded.text, "one");
        assert_eq!(expanded.stops, vec![vec![0..3], vec![3..3]]);
        let escaped = expand(r"${1|one\,two|}${2|x\|y|}", &spaces(4), "");
        assert_eq!(escaped.text, "one,twox|y");
    }

    #[test]
    fn a_multi_line_default_keeps_its_placeholders_and_heeds_the_step() {
        // A `}` inside a default is escaped; the unescaped one closes the placeholder.
        let expanded = expand("${1:if (x) {\n\ty();\n\\}}", &spaces(2), "");
        assert_eq!(expanded.text, "if (x) {\n  y();\n}");
        assert_eq!(expanded.stops, vec![vec![0..17], vec![17..17]]);
    }

    #[test]
    fn unsupported_or_malformed_bodies_say_why_and_expand_nothing() {
        assert_eq!(Snippet::parse("${1:outer ${2:inner}}").unwrap_err(), "Snippet placeholders cannot contain another placeholder");
        assert_eq!(Snippet::parse("${1:x").unwrap_err(), "The snippet ends inside a placeholder");
        assert_eq!(Snippet::parse("${0:text}").unwrap_err(), "The final placeholder $0 cannot hold text");
        assert_eq!(Snippet::parse("${1|one,two").unwrap_err(), "The snippet ends inside a choice");
        assert_eq!(Snippet::parse("${1/a/b/}").unwrap_err(), "Snippet placeholder transforms are not supported");
        assert_eq!(Snippet::parse("${TM_FILENAME}").unwrap_err(), "Snippet variables are not supported");
        assert_eq!(Snippet::parse("${}").unwrap_err(), "A placeholder needs a number or name");
    }
}

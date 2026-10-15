//! In-document find: a query (text and options) matched line by line. Matches never cross a line break.

use std::{fmt, ops::Range, sync::OnceLock};

use regex::{Regex, RegexBuilder};

/// Longer queries are refused (merge find already capped its input here).
pub const MAX_QUERY: usize = 4096;

/// How the query text is matched; remembered by each product while it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    /// Match case (off: ignore case, simple case folding).
    pub case: bool,
    /// Only matches with no letter, digit or `_` touching either end.
    pub word: bool,
    /// The text is a regular expression (off: literal).
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindOption {
    Case,
    Word,
    Regex,
}

impl FindOptions {
    pub fn get(self, option: FindOption) -> bool {
        match option {
            FindOption::Case => self.case,
            FindOption::Word => self.word,
            FindOption::Regex => self.regex,
        }
    }
    pub fn toggled(mut self, option: FindOption) -> Self {
        match option {
            FindOption::Case => self.case = !self.case,
            FindOption::Word => self.word = !self.word,
            FindOption::Regex => self.regex = !self.regex,
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub options: FindOptions,
}

/// The same query for a ripgrep (`grep-regex`) search: plain data, so this crate needs no grep dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrepSpec {
    pub pattern: String,
    pub case_insensitive: bool,
    pub word: bool,
    pub fixed_strings: bool,
}

impl Query {
    pub fn grep_spec(&self) -> GrepSpec {
        GrepSpec { pattern: self.text.clone(), case_insensitive: !self.options.case, word: self.options.word, fixed_strings: !self.options.regex }
    }
}

/// Why a query cannot be matched (a bad regular expression, or too long).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryError(String);

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for QueryError {}

/// A compiled query. An empty query matches nothing.
#[derive(Clone, Debug)]
pub struct Matcher {
    /// The pattern (escaped for literal text) and whether case is ignored; `None` for an empty query.
    pattern: Option<(String, bool)>,
    /// Compiled up front for a regular expression (it may be invalid); for literal text only when a line needs it
    /// (a Unicode case-insensitive expression costs about a millisecond to build).
    regex: OnceLock<Regex>,
    word: bool,
    /// An ASCII literal query, matched without the expression on ASCII lines (same matches, less work per line).
    literal: Option<(Box<[u8]>, bool)>,
}

impl Matcher {
    pub fn new(query: &Query) -> Result<Self, QueryError> {
        if query.text.len() > MAX_QUERY {
            return Err(QueryError(format!("the query is longer than {MAX_QUERY} bytes")));
        }
        if query.text.is_empty() {
            return Ok(Self { pattern: None, regex: OnceLock::new(), word: false, literal: None });
        }
        let pattern = if query.options.regex { query.text.clone() } else { regex::escape(&query.text) };
        let ignore_case = !query.options.case;
        let regex = OnceLock::new();
        if query.options.regex {
            let _ = regex.set(build(&pattern, ignore_case).map_err(|e| QueryError(reason(&e)))?);
        }
        let literal = (!query.options.regex && query.text.is_ascii()).then(|| (query.text.as_bytes().into(), query.options.case));
        Ok(Self { pattern: Some((pattern, ignore_case)), regex, word: query.options.word, literal })
    }

    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
    }

    /// The expression, built on first use (an escaped literal always builds).
    fn regex(&self) -> Option<&Regex> {
        if let Some(r) = self.regex.get() {
            return Some(r);
        }
        let (pattern, ignore_case) = self.pattern.as_ref()?;
        let built = build(pattern, *ignore_case).ok()?;
        Some(self.regex.get_or_init(|| built))
    }

    /// Non-overlapping, non-empty matches in `line`, as byte ranges on character boundaries.
    pub fn ranges<'a>(&'a self, line: &'a str) -> Ranges<'a> {
        let inner = match &self.literal {
            Some((needle, case)) if line.is_ascii() => Inner::Ascii { needle, case: *case, from: 0 },
            _ => match self.regex() {
                Some(regex) => Inner::Regex { regex, from: 0 },
                None => Inner::Done,
            },
        };
        Ranges { line, word: self.word, inner }
    }

    /// The expression's matches, without the ASCII fast path.
    #[cfg(test)]
    fn regex_ranges<'a>(&'a self, line: &'a str) -> impl Iterator<Item = Range<usize>> + 'a {
        self.regex().into_iter().flat_map(move |r| r.find_iter(line)).map(|m| m.range()).filter(|r| !r.is_empty())
    }
}

/// A line's matches (see [`Matcher::ranges`]).
pub struct Ranges<'a> {
    line: &'a str,
    word: bool,
    inner: Inner<'a>,
}

enum Inner<'a> {
    /// An ASCII literal in an ASCII line: compared byte by byte, ignoring ASCII case unless `case`.
    Ascii { needle: &'a [u8], case: bool, from: usize },
    /// The expression, searched again from `from` (keeps this iterator small on the ASCII path).
    Regex { regex: &'a Regex, from: usize },
    Done,
}

impl Iterator for Ranges<'_> {
    type Item = Range<usize>;
    fn next(&mut self) -> Option<Range<usize>> {
        loop {
            let r = match &mut self.inner {
                Inner::Ascii { needle, case, from } => {
                    let rest = self.line.as_bytes().get(*from..)?;
                    let at = if *case { rest.windows(needle.len()).position(|w| w == *needle) } else { rest.windows(needle.len()).position(|w| w.eq_ignore_ascii_case(needle)) }?;
                    let start = *from + at;
                    *from = start + needle.len();
                    start..*from
                }
                Inner::Regex { regex, from } => {
                    let m = regex.find_at(self.line, *from)?;
                    // An empty match moves on by one character; it is never reported.
                    *from = if m.is_empty() { m.end() + self.line.get(m.end()..)?.chars().next()?.len_utf8() } else { m.end() };
                    m.range()
                }
                Inner::Done => return None,
            };
            if !r.is_empty() && (!self.word || whole_word(self.line, &r)) {
                return Some(r);
            }
        }
    }
}

fn build(pattern: &str, ignore_case: bool) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern).case_insensitive(ignore_case).build()
}

/// The parser's own words, without its picture of the pattern: "unclosed group".
fn reason(e: &regex::Error) -> String {
    match e {
        regex::Error::Syntax(text) => text.lines().last().map_or(text.as_str(), |l| l.trim_start_matches("error: ")).to_owned(),
        regex::Error::CompiledTooBig(_) => "the regular expression is too big".into(),
        _ => e.to_string(),
    }
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn whole_word(line: &str, r: &Range<usize>) -> bool {
    let before = line.get(..r.start).and_then(|s| s.chars().next_back());
    let after = line.get(r.end..).and_then(|s| s.chars().next());
    !before.is_some_and(word_char) && !after.is_some_and(word_char)
}


#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str, options: FindOptions, line: &str) -> Vec<std::ops::Range<usize>> {
        Matcher::new(&Query { text: text.into(), options }).unwrap().ranges(line).collect()
    }
    const PLAIN: FindOptions = FindOptions { case: false, word: false, regex: false };

    #[test]
    fn literal_text_ignores_case_by_default_and_reports_byte_ranges() {
        assert_eq!(found("foo", PLAIN, "Foo foo FOO"), vec![0..3, 4..7, 8..11]);
        assert_eq!(found("a.b", PLAIN, "axb a.b"), vec![4..7], "literal: the dot is not a wildcard");
        assert_eq!(found("café", PLAIN, "CAFÉ café"), vec![0..5, 6..11]);
        assert!(found("GRÖSSE", PLAIN, "Größe").is_empty(), "simple case folding only, as before");
    }

    #[test]
    fn match_case_and_whole_word_narrow_the_matches() {
        let case = FindOptions { case: true, ..PLAIN };
        assert_eq!(found("foo", case, "Foo foo FOO"), vec![4..7]);
        let word = FindOptions { word: true, ..PLAIN };
        assert_eq!(found("foo", word, "foo foobar _foo foo_ (foo)"), vec![0..3, 22..25]);
        assert_eq!(found("ä", word, "äb ä"), vec![4..6], "letters beyond ASCII are word characters");
    }

    #[test]
    fn regular_expressions_match_within_one_line() {
        let re = FindOptions { regex: true, ..PLAIN };
        assert_eq!(found(r"v\d+", re, "v1 V22 x3"), vec![0..2, 3..6]);
        assert_eq!(found("a.c", re, "abc"), vec![0..3]);
        let word_re = FindOptions { regex: true, word: true, ..PLAIN };
        assert_eq!(found(r"\w+_id", word_re, "user_id xuser_idx"), vec![0..7]);
    }

    #[test]
    fn empty_matches_are_skipped() {
        let re = FindOptions { regex: true, ..PLAIN };
        assert!(found("a*", re, "bbb").is_empty());
        assert_eq!(found("a*", re, "baab"), vec![1..3]);
        assert!(found("^", re, "text").is_empty());
        assert!(found("", PLAIN, "text").is_empty());
        assert!(Matcher::new(&Query::default()).unwrap().is_empty());
    }

    #[test]
    fn bad_and_oversized_queries_say_why() {
        let re = FindOptions { regex: true, ..PLAIN };
        let err = Matcher::new(&Query { text: "(ab".into(), options: re }).err().unwrap();
        assert_eq!(err.to_string(), "unclosed group");
        assert!(Matcher::new(&Query { text: "(ab".into(), options: PLAIN }).is_ok(), "literal text never fails");
        let long = Query { text: "x".repeat(MAX_QUERY + 1), options: PLAIN };
        assert_eq!(Matcher::new(&long).err().unwrap().to_string(), "the query is longer than 4096 bytes");
    }

    #[test]
    fn the_ascii_fast_path_agrees_with_the_expression() {
        let lines = ["Foo foo FOO", "aaaa", "x_foo foo_x (foo)", "", "fo", "FOOfoo"];
        for text in ["foo", "FOO", "aa", "o", "x_"] {
            for (case, word) in [(false, false), (true, false), (false, true), (true, true)] {
                let m = Matcher::new(&Query { text: text.into(), options: FindOptions { case, word, regex: false } }).unwrap();
                assert!(m.literal.is_some(), "ASCII literal queries take the fast path");
                for line in lines {
                    let fast: Vec<_> = m.ranges(line).collect();
                    let slow: Vec<_> = m.regex_ranges(line).filter(|r| !word || whole_word(line, r)).collect();
                    assert_eq!(fast, slow, "{text:?} case={case} word={word} in {line:?}");
                }
            }
        }
        let unicode = Matcher::new(&Query { text: "k".into(), options: FindOptions::default() }).unwrap();
        assert_eq!(unicode.ranges("\u{212A} k").collect::<Vec<_>>(), vec![0..3, 4..5], "a non-ASCII line keeps Unicode folding");
    }

    #[test]
    fn a_literal_query_compiles_its_expression_only_for_a_non_ascii_line() {
        let m = Matcher::new(&Query { text: "needle".into(), options: FindOptions::default() }).unwrap();
        assert_eq!(m.ranges("a needle").count(), 1);
        assert!(m.regex.get().is_none(), "ASCII lines need no expression");
        assert_eq!(m.ranges("é needle").count(), 1);
        assert!(m.regex.get().is_some());
        let re = Matcher::new(&Query { text: "n.e".into(), options: FindOptions { regex: true, ..Default::default() } }).unwrap();
        assert!(re.regex.get().is_some(), "an expression is compiled up front: it may be invalid");
    }

    #[test]
    fn options_toggle_one_at_a_time() {
        let o = PLAIN.toggled(FindOption::Case).toggled(FindOption::Regex);
        assert_eq!(o, FindOptions { case: true, word: false, regex: true });
        assert!(o.get(FindOption::Case) && !o.get(FindOption::Word));
    }

    #[test]
    fn grep_spec_carries_the_same_query() {
        let q = Query { text: "a.b".into(), options: FindOptions { case: false, word: true, regex: false } };
        assert_eq!(q.grep_spec(), GrepSpec { pattern: "a.b".into(), case_insensitive: true, word: true, fixed_strings: true });
    }
}

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

impl FindOption {
    pub const ALL: [Self; 3] = [Self::Case, Self::Word, Self::Regex];
    pub fn label(self) -> &'static str {
        match self { Self::Case => "Match case", Self::Word => "Whole word", Self::Regex => "Regular expression" }
    }
    pub fn key(self) -> char {
        match self { Self::Case => 'c', Self::Word => 'w', Self::Regex => 'r' }
    }
    pub fn symbol(self) -> &'static str {
        match self { Self::Case => "Aa", Self::Word => "W", Self::Regex => ".*" }
    }
}

/// In-document searches retain at most this many matches; a further match marks truncation.
pub const MATCH_LIMIT: usize = 10_000;

pub type Matches<L> = (Vec<(L, Range<usize>)>, bool);

/// Collect in document order without copying lines. `None` discards obsolete work.
/// Check cancellation on sparse documents and within a line containing many matches.
pub fn collect_matches<'a, L: Copy>(matcher: &Matcher, lines: impl IntoIterator<Item = (L, &'a str)>, mut current: impl FnMut() -> bool) -> Option<Matches<L>> {
    let mut hits = Vec::new();
    for (index, (line, text)) in lines.into_iter().enumerate() {
        if index % 1024 == 0 && !current() { return None; }
        for range in matcher.ranges(text) {
            if hits.len() % 1024 == 0 && !current() { return None; }
            if hits.len() == MATCH_LIMIT { return Some((hits, true)); }
            hits.push((line, range));
        }
    }
    if !current() { return None; }
    Some((hits, false))
}

/// First match at or after the anchor, wrapping to the beginning.
pub fn at_or_after<L: Copy + Ord>(hits: &[(L, Range<usize>)], line: L, byte: usize) -> Option<usize> {
    if hits.is_empty() { return None; }
    let at = hits.partition_point(|(l, range)| (*l, range.start) < (line, byte));
    Some(if at < hits.len() { at } else { 0 })
}

/// Next selection and whether navigation wrapped. Ignore a stale selection.
pub fn next_index(count: usize, current: Option<usize>, forward: bool) -> Option<(usize, bool)> {
    if count == 0 { return None; }
    let Some(at) = current.filter(|&i| i < count) else { return Some((0, false)); };
    let next = if forward { if at + 1 == count { 0 } else { at + 1 } } else if at == 0 { count - 1 } else { at - 1 };
    Some((next, if forward { next <= at } else { next >= at }))
}

/// Shared find status, including pending, invalid and truncated searches.
pub fn counter(query: &str, count: usize, current: Option<usize>, capped: bool, pending: bool, error: Option<&str>) -> String {
    if let Some(error) = error { return error.to_owned(); }
    if count == 0 {
        return if query.is_empty() { String::new() } else if pending { "…".into() } else { "No matches".into() };
    }
    let total = if capped { "10,000+".to_owned() } else { count.to_string() };
    format!("{}/{total}", current.filter(|&i| i < count).map_or(0, |i| i + 1))
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

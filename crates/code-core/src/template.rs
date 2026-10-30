//! Templates' embedded languages (ticket 57). An ERB or Haml template is never sent to a language server as it is:
//! each embedded language gets a projection of the template, the template's Ruby (or, for an HTML ERB template, its
//! markup) with an exact map back to the template's own bytes. A server is sent the projection under the template's
//! own URI and version, so its answers carry the original document's identity; positions are converted in the
//! server's negotiated encoding against the projection and mapped back through the map. A provider's edit applies
//! only when every part of it lies inside one mapped region and reading each touched construct again finds the
//! template's own tags, delimiters and lines as they were; otherwise the whole transaction is refused, with why.
//!
//! ERB is read as Rails reads it (Erubi): `<%` code, `<%=`/`<%==` output, `<%-` and a `-`/`=` before `%>` trim
//! markers, `<%#` comments and `<%%` literals; `%%>` (which ERB and Erubi read differently) is refused. Its Ruby runs
//! inside a method, as Action View, Tilt and `ERB#def_method` compile a template (ticket 58), and a Rails view's inside
//! a class of its own that is an `ActionView::Base` with the application's and its controller's helpers and its
//! strict `locals:` as the method's parameters: the view context, told to Ruby LSP in the projection's own lines
//! ([`scaffolding`] names, never shown, never edited). Haml is read
//! by its indentation, scripts, attributes and interpolation. Ruby blocks get unmapped closing code, and Rails
//! views get unmapped helper context. Unsupported constructs leave the Ruby unmapped with a reason.
use std::{ops::Range, path::Path, sync::Arc};

use lenz_content::highlight::Language;

use crate::{document::Document, text::TextEdit};

#[path = "haml.rs"]
mod haml;
use haml::{haml_line, haml_ruby, haml_continuation};
pub(crate) use haml::{markup as haml_markup, indent as haml_indent};

/// A template language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Erb,
    Haml,
}

impl Kind {
    /// The template language a file name names by its last extension (`.erb`, `.html.erb`, `.rhtml`, `.haml`).
    pub fn of(path: &Path) -> Option<Self> {
        // Asked on every position conversion: no allocation.
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("erb") || extension.eq_ignore_ascii_case("rhtml") { return Some(Self::Erb); }
        extension.eq_ignore_ascii_case("haml").then_some(Self::Haml)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Erb => "ERB",
            Self::Haml => "Haml",
        }
    }
}

const RUBY: &[Language] = &[Language::Ruby];
const RUBY_HTML: &[Language] = &[Language::Ruby, Language::Html];

/// The languages `path`'s projections are served as, the primary (whole-document questions) first; empty when it is
/// not a template. An ERB template whose host text is HTML (`.html.erb`, `.htm.erb`, `.rhtml`) has its markup served
/// too; Haml markup is not HTML and is never sent to an HTML server.
pub fn languages(path: &Path) -> &'static [Language] {
    match Kind::of(path) {
        Some(Kind::Erb) if html_host(path) => RUBY_HTML,
        Some(_) => RUBY,
        None => &[],
    }
}

/// Whether an ERB template's host text is HTML: `.html.erb`, `.htm.erb`, a Rails variant of them (`.html+phone.erb`),
/// `.rhtml`.
fn html_host(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_ascii_lowercase();
    if name.ends_with(".rhtml") { return true; }
    let Some(stem) = name.strip_suffix(".erb") else { return false };
    let Some((_, format)) = stem.rsplit_once('.') else { return false };
    matches!(format.split_once('+').map_or(format, |(f, _)| f), "html" | "htm")
}

/// Where Lenz's own lines in a projection are named: the method a template's Ruby runs in, a Rails view's class.
const SCAFFOLDING: &str = "LenzTemplate";

/// Whether a name a server reports (a suggestion, a symbol) is one of the projections' own: never shown. Ruby LSP
/// indexes an open template's projection, so these names reach answers about other files too.
pub fn scaffolding(name: &str) -> bool { name.starts_with(SCAFFOLDING) || name.starts_with("LenzHamlView") || name == "__lenz_haml_render__" }

/// How a Haml Rails view's projection starts, declaring its own class (see `haml::rails_context`).
const HAML_VIEW: &str = "class LenzHamlView";

/// A Rails view (a template under an application's `app/views`): what its Ruby runs as.
#[derive(Debug, PartialEq, Eq)]
struct RailsView {
    /// The class of its own the projection declares (from the template's path, as Rails names its compiled method).
    class: String,
    /// The helper modules its controller's views include: the application's (a mailer's: the mail helper) and the one
    /// its folder names (`posts/` → `PostsHelper`), when the folder names a constant.
    helpers: Vec<String>,
}

impl RailsView {
    fn of(path: &Path) -> Option<Self> {
        let parts: Vec<&str> = path.iter().map(|p| p.to_str()).collect::<Option<_>>()?;
        let at = parts.windows(2).rposition(|w| w == ["app", "views"])?;
        let rest = parts.get(at + 2..)?;
        let (_, folders) = rest.split_last()?;
        // As Action View's `identifier_method_name`: the path from `app`, everything but `a-z` and `_` made `_`.
        let class = format!("{SCAFFOLDING}::View_{}", parts[at..].join("/").chars().map(|c| if c.is_ascii_lowercase() || c == '_' { c } else { '_' }).collect::<String>());
        let constant = |folder: &str| -> Option<String> {
            let ok = folder.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && folder.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            ok.then(|| folder.split('_').filter(|w| !w.is_empty()).map(|w| { let mut c = w.chars(); c.next().map_or_else(String::new, |f| f.to_ascii_uppercase().to_string() + c.as_str()) }).collect())
        };
        let mailer = folders.last().is_some_and(|f| f.ends_with("_mailer"));
        let mut helpers = vec![if mailer { "ActionMailer::MailHelper".to_owned() } else { "ApplicationHelper".to_owned() }];
        if !folders.is_empty() && let Some(names) = folders.iter().map(|f| constant(f)).collect::<Option<Vec<String>>>() {
            helpers.push(format!("{}Helper", names.join("::")));
        }
        Some(Self { class, helpers })
    }
}

/// Action View's strict locals comment (`STRICT_LOCALS_REGEX`, Rails 8.1: `# locals: (…)` the first time a `)` is
/// followed by the tag's end or a line's): the whole match and its parameter list without trailing spaces.
fn strict_locals(text: &str) -> Option<(Range<usize>, Range<usize>)> {
    let space = |c: u8| matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c);
    let bytes = text.as_bytes();
    let skip = |mut i: usize| { while bytes.get(i).copied().is_some_and(space) { i += 1; } i };
    let mut from = 0;
    while let Some(found) = text.get(from..).and_then(|t| t.find('#')) {
        let hash = from + found;
        from = hash + 1;
        let after = skip(hash + 1);
        if after == hash + 1 || !text.get(after..).is_some_and(|t| t.starts_with("locals:")) { continue; }
        let open = skip(after + "locals:".len());
        if open == after + "locals:".len() || bytes.get(open) != Some(&b'(') { continue; }
        let mut close = open + 1;
        while let Some(found) = text.get(close..).and_then(|t| t.find(')')) {
            close += found;
            let end = skip(close + 1);
            let rest = text.get(end..).unwrap_or_default();
            let ends = end == text.len() || text.get(close + 1..end).is_some_and(|w| w.contains('\n')) || rest.starts_with("%>") || rest.starts_with("-%>");
            if ends {
                let group = text.get(open + 1..close).unwrap_or_default();
                let kept = group.trim_end_matches(|c: char| c.is_ascii() && space(c as u8)).len();
                return Some((hash..close + 1, open + 1..open + 1 + kept));
            }
            close += 1;
        }
    }
    None
}

/// Which projection: a template language and the embedded language it presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub kind: Kind,
    pub language: Language,
}

/// One mapped region: template bytes copied unchanged into the projection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Its bytes in the template.
    pub source: Range<usize>,
    /// Where its copy starts in the projection.
    pub generated: usize,
    /// The template construct holding it (an ERB tag from `<%` to `%>`, a stretch of host text, a Haml line): an edit
    /// is checked by reading that construct again.
    unit: Range<usize>,
    /// Haml: the line continues a multiline (`|`) script.
    continued: bool,
    /// Haml: the line ends with ` |`, going on into the next.
    continues: bool,
    /// Its value is output (`<%= … %>`, Haml `=` and `#{…}`): never a statement in void context.
    output: bool,
    /// A Rails view's strict locals (the parameters of the method its Ruby runs in): never edited.
    locals: bool,
}

/// A projection's text and regions, in template order.
#[derive(Debug, Default, PartialEq, Eq)]
struct Projected {
    text: String,
    spans: Vec<Span>,
}

/// The template text one embedded language is served: its own document (lines in the server's sense) and the map.
#[derive(Debug)]
pub struct Projection {
    route: Route,
    generated: Document,
    spans: Vec<Span>,
    /// It declares a Rails view's class: its server must index it before that class is known (see [`Self::indexed`]).
    view: bool,
}

impl Projection {
    pub fn route(&self) -> Route { self.route }
    /// The projected text as a document of its own: what positions are counted in.
    pub fn generated(&self) -> &Document { &self.generated }
    pub fn spans(&self) -> &[Span] { &self.spans }
    /// Whether its server must index it for its context to apply: a Rails view's Ruby declares the class its helpers
    /// are included in, which Ruby LSP learns only by indexing the document (it does so when asked for its symbols).
    pub fn indexed(&self) -> bool { self.view }

    /// Projection offset of template byte `at`; `None` outside every mapped region (host text, delimiters, comments).
    pub fn to_generated(&self, at: usize) -> Option<usize> {
        let i = self.spans.partition_point(|s| s.source.start <= at).checked_sub(1)?;
        let span = self.spans.get(i)?;
        (at <= span.source.end).then(|| span.generated + (at - span.source.start))
    }

    /// Template byte of projection offset `at`; `None` in the projection's own scaffolding.
    pub fn to_source(&self, at: usize) -> Option<usize> {
        let i = self.spans.partition_point(|s| s.generated <= at).checked_sub(1)?;
        let span = self.spans.get(i)?;
        (at - span.generated <= span.source.len()).then(|| span.source.start + (at - span.generated))
    }

    /// For display only (a problem's range): the template byte of `at`, or of the nearest mapped place before it.
    pub fn near_source(&self, at: usize) -> usize {
        self.to_source(at).unwrap_or_else(|| {
            let i = self.spans.partition_point(|s| s.generated <= at);
            match i.checked_sub(1).and_then(|i| self.spans.get(i)) {
                Some(span) => span.source.end,
                None => self.spans.first().map_or(0, |s| s.source.start),
            }
        })
    }

    /// For a request position: the projection offset of `at`, or of the nearest mapped place before it.
    pub fn near_generated(&self, at: usize) -> usize {
        self.to_generated(at).unwrap_or_else(|| {
            let i = self.spans.partition_point(|s| s.source.start <= at);
            match i.checked_sub(1).and_then(|i| self.spans.get(i)) {
                Some(span) => span.generated + span.source.len(),
                None => 0,
            }
        })
    }

    /// Whether a problem a server reported about the projection (starting at projection offset `at`) describes the
    /// template: not a Ruby style linter's (Ruby LSP's RuboCop or Standard Ruby), which would check the projection's own
    /// layout (its line ends, the spaces inside `<%= … %>`), and not Ruby's "useless use … in void context" about an
    /// output (`<%= x %>` renders `x`; the projection has it as a statement of its own).
    pub fn keeps_problem(&self, source: Option<&str>, message: &str, at: Option<usize>) -> bool {
        if self.route.language != Language::Ruby { return true; }
        if matches!(source, Some("RuboCop" | "Standard Ruby")) { return false; }
        let output = at.and_then(|at| self.spans.get(self.spans.partition_point(|s| s.generated <= at).checked_sub(1)?)).is_some_and(|s| s.output);
        !(output && message.contains("in void context"))
    }

    fn language_name(&self) -> &'static str {
        match self.route.language {
            Language::Ruby => "Ruby",
            _ => "markup",
        }
    }

    /// Why applying `edits` (sorted, non-overlapping byte edits of the template as it is now) through this projection
    /// is refused. Every edit must lie inside one mapped region, and reading each construct it touches again must find
    /// the template's own tags, delimiters and lines unchanged (an edit adding `%>`, turning `<%` into `<%#`, or a
    /// Haml line into another kind, is refused). `typed`: edit sets made since this projection's version, in order
    /// (typing that continued a completion); `read`: the template's current text in a range. Nothing is copied but the
    /// touched constructs.
    pub fn check(&self, edits: &[TextEdit], typed: &[Vec<TextEdit>], read: &dyn Fn(Range<usize>) -> Option<String>) -> Result<(), String> {
        let outside = || format!("it reaches outside the template's embedded {}", self.language_name());
        let changed = || "it would change the template's own tags, delimiters or lines".to_owned();
        let carried = |r: &Range<usize>| carry(r, typed);
        // Each edit's region, by index; edits of one construct are checked together.
        let mut groups: Vec<(usize, Vec<&TextEdit>)> = Vec::new();
        for edit in edits {
            let found = self.spans.iter().position(|s| {
                let at = carried(&s.source);
                at.start <= edit.range.start && edit.range.end <= at.end
            });
            let Some(i) = found else { return Err(outside()) };
            if self.spans.get(i).is_some_and(|s| s.locals) { return Err("it is inside the template's `locals:` comment, which Lenz does not edit through a language server".into()); }
            let unit = self.spans.get(i).map_or(0, |s| s.unit.start);
            match groups.iter_mut().find(|(u, _)| *u == unit) {
                Some((_, list)) => list.push(edit),
                None => groups.push((unit, vec![edit])),
            }
        }
        for (unit, list) in groups {
            let members: Vec<&Span> = self.spans.iter().filter(|s| s.unit.start == unit).collect();
            let Some(first) = members.first() else { return Err(outside()) };
            let at = carried(&first.unit);
            let Some(mut text) = read(at.clone()) else { return Err(changed()) };
            let haml_shape = (self.route.kind == Kind::Haml && !first.continued)
                .then(|| haml_line(text.trim_start_matches(' '), 0).ok().map(|l| l.shape())).flatten();
            let local: Vec<TextEdit> = list.iter().map(|e| TextEdit { range: e.range.start - at.start..e.range.end - at.start, text: e.text.clone() }).collect();
            for e in local.iter().rev() {
                if !(text.is_char_boundary(e.range.start) && text.is_char_boundary(e.range.end) && e.range.end <= text.len()) { return Err(changed()); }
                text.replace_range(e.range.clone(), &e.text);
            }
            let expected: Vec<Range<usize>> = members.iter().map(|s| {
                let r = carried(&s.source);
                let r = r.start - at.start..r.end - at.start;
                carry(&r, std::slice::from_ref(&local))
            }).collect();
            let again = match (self.route.kind, self.route.language) {
                (Kind::Erb, Language::Ruby) => erb_tags(&text).ok().and_then(|tags| match tags.as_slice() {
                    // An unclosed tag (being typed) runs to the end of the template: its unit ends with its code.
                    [tag] if tag.start == 0 && tag.end == text.len() && tag.closed == (first.unit.end > first.source.end) => Some(tag.ruby().into_iter().collect::<Vec<_>>()),
                    _ => None,
                }),
                (Kind::Erb, _) => erb_tags(&text).ok().filter(|tags| tags.is_empty()).map(|_| std::iter::once(0..text.len()).collect()),
                (Kind::Haml, _) => {
                    let indent = text.len() - text.trim_start_matches(' ').len();
                    let body = text.get(indent..).unwrap_or_default();
                    let ends = text.contains(['\n', '\r']);
                    let line = if first.continued { Some(haml_continuation(body)) } else { haml_line(body, 0).ok() };
                    line.filter(|l| !ends && !l.comment && l.continues == first.continues && haml_shape.is_none_or(|shape| shape == l.shape()))
                        .map(|l| l.spans.iter().map(|r| r.start + indent..r.end + indent).collect())
                }
            };
            if again.as_deref() != Some(expected.as_slice()) { return Err(changed()); }
        }
        Ok(())
    }
}

/// `r` after the edit sets in `typed`, in order: its start stays before text inserted at it, its end moves after it.
fn carry(r: &Range<usize>, typed: &[Vec<TextEdit>]) -> Range<usize> {
    let (mut start, mut end) = (r.start, r.end);
    for edits in typed {
        start = shifted(start, edits, false);
        end = shifted(end, edits, true).max(start);
    }
    start..end
}

/// `byte` after `edits` (sorted): inside a replaced range it goes to the replacement's end (`after`) or start.
fn shifted(byte: usize, edits: &[TextEdit], after: bool) -> usize {
    let mut delta: isize = 0;
    for edit in edits {
        let inserted_here = edit.range.start == byte && edit.range.is_empty();
        if byte < edit.range.start || (inserted_here && !after) { break; }
        if byte < edit.range.end || (inserted_here && after) {
            let start = edit.range.start.saturating_add_signed(delta);
            return if after || byte >= edit.range.end { start + edit.text.len() } else { start };
        }
        delta += edit.text.len() as isize - edit.range.len() as isize;
    }
    byte.saturating_add_signed(delta)
}

/// A template's projections, built once per version on the worker that materializes it.
#[derive(Debug)]
pub struct Views {
    kind: Kind,
    /// A Rails view's context, for Server Details.
    view: Option<String>,
    /// Its Ruby, or why it is not mapped.
    ruby: Result<Arc<Document>, String>,
    /// Its HTML markup (an HTML ERB template).
    markup: Option<Result<Arc<Document>, String>>,
}

impl Views {
    pub(crate) fn new(doc: &Document) -> Option<Self> {
        let kind = Kind::of(doc.source())?;
        let rails = (kind == Kind::Erb).then(|| RailsView::of(doc.source())).flatten();
        let view = |language: Language| {
            let route = Route { kind, language };
            project(route, doc.text(), rails.as_ref()).map(|mut p| {
                if kind == Kind::Haml && language == Language::Ruby { haml::rails_context(&mut p, doc.source()); }
                // Its own name: positions are converted in it, and a template itself is never converted.
                let mut name = doc.source().as_os_str().to_owned();
                name.push(".projection");
                // A Rails view's class (an ERB view's, or ticket 59's Haml view's) is known to Ruby LSP once indexed.
                let view = language == Language::Ruby && (rails.is_some() || p.text.starts_with(HAML_VIEW));
                let generated = Document::from_snapshot_parts(Arc::new(doc.source_meta().renamed(name.into())), doc.version(), p.text.into());
                Arc::new(doc.embedded_view(Arc::new(Projection { route, generated, spans: p.spans, view })))
            })
        };
        let markup = (kind == Kind::Erb && html_host(doc.source())).then(|| view(Language::Html));
        let context = rails.as_ref().map(|r| {
            let locals = strict_locals(doc.text()).map_or_else(String::new, |(_, l)| format!(" and the strict locals `{}`", doc.text().get(l).unwrap_or_default()));
            format!("ERB template view context: a Rails view (Action View), with {}{locals}; controller instance variables and route helpers are not known to Ruby LSP's index", r.helpers.join(" and "))
        });
        Some(Self { kind, view: context, ruby: view(Language::Ruby), markup })
    }

    pub fn kind(&self) -> Kind { self.kind }

    /// The view serving byte `at`: the Ruby region holding it, else the markup holding it, else why none does.
    pub fn at(&self, at: usize) -> Result<&Arc<Document>, String> {
        let holds = |d: &&Arc<Document>| d.embedded().is_some_and(|p| p.to_generated(at).is_some());
        if let Some(ruby) = self.ruby.as_ref().ok().filter(holds) { return Ok(ruby); }
        if let Some(markup) = self.markup.as_ref().and_then(|m| m.as_ref().ok()).filter(holds) { return Ok(markup); }
        let name = self.kind.name();
        Err(match (&self.ruby, &self.markup) {
            (Err(why), None) => format!("this {name} template's Ruby is not mapped: {why}"),
            (Err(why), Some(_)) if !self.in_markup(at) => format!("this {name} template's Ruby is not mapped: {why}"),
            (_, Some(Ok(_))) => format!("the caret is on one of the {name} template's own delimiters or comments, which no language server is asked about"),
            (_, Some(Err(why))) => format!("this {name} template's markup is not mapped: {why}"),
            (Ok(_), None) if self.kind == Kind::Haml => format!("the caret is in the {name} template's markup; Lenz serves only its embedded Ruby there (Haml markup is not HTML)"),
            (Ok(_), None) => format!("the caret is in the {name} template's text; Lenz serves only its embedded Ruby here"),
        })
    }

    fn in_markup(&self, at: usize) -> bool {
        matches!(&self.markup, Some(Ok(m)) if m.embedded().is_some_and(|p| p.to_generated(at).is_some()))
    }

    /// The view whole-document questions (formatting, file symbols) are asked of: its Ruby.
    pub fn primary(&self) -> Result<&Arc<Document>, String> {
        self.ruby.as_ref().map_err(|why| format!("this {} template's Ruby is not mapped: {why}", self.kind.name()))
    }

    /// The views a server is sent.
    pub fn served(&self) -> impl Iterator<Item = &Arc<Document>> {
        self.ruby.as_ref().ok().into_iter().chain(self.markup.as_ref().and_then(|m| m.as_ref().ok()))
    }

    /// The view presenting `language`.
    pub fn language(&self, language: Language) -> Option<&Arc<Document>> {
        self.served().find(|d| d.embedded().is_some_and(|p| p.route.language == language))
    }

    /// Why none of its languages can be served (`None`: one is).
    pub fn unmapped(&self) -> Option<String> {
        if self.served().next().is_some() { return None; }
        self.primary().err()
    }

    /// One row per embedded language, for Server Details.
    pub fn summary(&self) -> Vec<String> {
        let name = self.kind.name();
        let row = |what: &str, view: &Result<Arc<Document>, String>, to: &str| match view {
            Ok(d) => format!("{name} template {what}: {} regions mapped exactly to the template, served by {to}; edits apply only inside them", d.embedded().map_or(0, |p| p.spans.len())),
            Err(why) => format!("{name} template {what}: not mapped ({why}); no language server is asked about it"),
        };
        let mut rows = vec![row("Ruby", &self.ruby, "the Ruby provider")];
        rows.extend(self.view.clone());
        match &self.markup {
            Some(markup) => rows.push(row("markup", markup, "the HTML provider (Ruby tags hidden)")),
            None if self.kind == Kind::Haml => rows.push("Haml markup: local tag and parenthesized attribute suggestions; never sent to an HTML server".into()),
            None => rows.push(format!("{name} template markup: not served (only HTML ERB templates' markup goes to the HTML provider)")),
        }
        rows
    }
}

/// `text` projected for `route` (`rails`: a Rails view's context), or why it cannot be mapped exactly.
fn project(route: Route, text: &str, rails: Option<&RailsView>) -> Result<Projected, String> {
    match (route.kind, route.language) {
        (Kind::Erb, Language::Ruby) => erb_ruby(text, rails),
        (Kind::Erb, Language::Html) => erb_markup(text),
        (Kind::Haml, Language::Ruby) => haml_ruby(text),
        _ => Err("no projection for that language".into()),
    }
}

/// The 1-based line holding byte `at`.
fn line_number(text: &str, at: usize) -> usize {
    text.as_bytes().iter().take(at).filter(|b| **b == b'\n').count() + 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Indicator {
    /// `<%`, `<%-`: code.
    Code,
    /// `<%=`, `<%==`: output.
    Output,
    /// `<%#`.
    Comment,
    /// `<%%`: literal text.
    Literal,
}

/// One ERB tag: `start..end` from `<%` to after `%>` (or to the end of an unclosed one), and its code.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Tag {
    start: usize,
    indicator: Indicator,
    code: Range<usize>,
    end: usize,
    closed: bool,
}

impl Tag {
    /// Its Ruby, when it has any.
    fn ruby(&self) -> Option<Range<usize>> {
        matches!(self.indicator, Indicator::Code | Indicator::Output).then(|| self.code.clone())
    }
}

/// Every tag of `text`, as Erubi scans it (`<%(={1,2}|-|#|%)?(.*?)([-=])?%>`). A tag left open runs to the end (one
/// being typed). `%%>` is refused: ERB reads it as an escaped `%>`, Erubi as the end of the tag.
fn erb_tags(text: &str) -> Result<Vec<Tag>, String> {
    let bytes = text.as_bytes();
    let mut tags = Vec::new();
    let mut at = 0;
    while let Some(found) = text.get(at..).and_then(|t| t.find("<%")) {
        let start = at + found;
        let after = start + 2;
        let rest = bytes.get(after..).unwrap_or_default();
        let (indicator, len) = match rest {
            [b'=', b'=', ..] => (Indicator::Output, 2),
            [b'=', ..] => (Indicator::Output, 1),
            [b'-', ..] => (Indicator::Code, 1),
            [b'#', ..] => (Indicator::Comment, 1),
            [b'%', ..] => (Indicator::Literal, 1),
            _ => (Indicator::Code, 0),
        };
        let code_start = after + len;
        match text.get(code_start..).and_then(|t| t.find("%>")) {
            Some(close) => {
                let close = code_start + close;
                let before = close.checked_sub(1).filter(|b| *b >= code_start).and_then(|b| bytes.get(b).copied());
                if before == Some(b'%') && matches!(indicator, Indicator::Code | Indicator::Output) {
                    return Err(format!("line {}: `%%>` is read as an escaped `%>` by ERB and as the end of the tag by Erubi (Rails)", line_number(text, close)));
                }
                // A `-` or `=` right before `%>` is a trim marker, not code (Erubi's `tailch`).
                let code_end = if matches!(before, Some(b'-' | b'=')) { close - 1 } else { close };
                tags.push(Tag { start, indicator, code: code_start..code_end, end: close + 2, closed: true });
                at = close + 2;
            }
            None => {
                tags.push(Tag { start, indicator, code: code_start..text.len(), end: text.len(), closed: false });
                break;
            }
        }
    }
    Ok(tags)
}

/// An ERB template's Ruby: each code and output tag's code, each on lines of its own (a newline after each), so
/// statements stay apart and multiline Ruby keeps its own lines; all of it in a method, as templates are compiled (so
/// `yield` is valid and a class or constant definition is not). A Rails view's method is declared as Action View
/// declares it (`local_assigns, output_buffer`, the strict locals, a block) in its own `ActionView::Base` class with
/// its helpers; its strict locals are mapped when they come before its Ruby (where Rails' comment goes).
fn erb_ruby(text: &str, rails: Option<&RailsView>) -> Result<Projected, String> {
    let tags = erb_tags(text)?;
    let mut out = Projected::default();
    match rails {
        None => out.text.push_str("def LenzTemplate_render(&_)\n"),
        Some(view) => {
            out.text.push_str(&format!("class {} < ActionView::Base\n", view.class));
            for helper in &view.helpers { out.text.push_str(&format!("include {helper}\n")); }
            out.text.push_str("def LenzTemplate_render(local_assigns, output_buffer");
            let first = tags.iter().find_map(Tag::ruby).map_or(text.len(), |c| c.start);
            match strict_locals(text) {
                Some((_, group)) if group.is_empty() => out.text.push_str(", **nil, &_"),
                Some((whole, group)) => {
                    out.text.push_str(", ");
                    let locals = text.get(group.clone()).unwrap_or_default();
                    if whole.end <= first {
                        out.spans.push(Span { source: group.clone(), generated: out.text.len(), unit: whole, continued: false, continues: false, output: false, locals: true });
                    }
                    out.text.push_str(locals);
                    if !locals.contains('&') { out.text.push_str(", &_"); }
                }
                None => out.text.push_str(", &_"),
            }
            out.text.push_str(")\n");
        }
    }
    for tag in tags {
        let Some(code) = tag.ruby() else { continue };
        let generated = out.text.len();
        out.text.push_str(text.get(code.clone()).unwrap_or_default());
        out.text.push('\n');
        out.spans.push(Span { source: code, generated, unit: tag.start..tag.end, continued: false, continues: false, output: tag.indicator == Indicator::Output, locals: false });
    }
    out.text.push_str(if rails.is_some() { "end\nend\n" } else { "end\n" });
    Ok(out)
}

/// An HTML ERB template's markup: its host text at the same bytes, every tag (delimiters included) replaced by `_`
/// (line ends kept), so a page's scripts and attributes still parse where a tag's output goes.
fn erb_markup(text: &str) -> Result<Projected, String> {
    let tags = erb_tags(text)?;
    let mut out = Projected { text: String::with_capacity(text.len()), spans: Vec::with_capacity(tags.len() + 1) };
    let mut host = 0;
    for tag in tags.iter().map(|t| t.start..t.end).chain(std::iter::once(text.len()..text.len())) {
        out.spans.push(Span { source: host..tag.start, generated: host, unit: host..tag.start, continued: false, continues: false, output: false, locals: false });
        out.text.push_str(text.get(host..tag.start).unwrap_or_default());
        out.text.extend(text.as_bytes().get(tag.clone()).unwrap_or_default().iter().map(|b| if matches!(b, b'\n' | b'\r') { *b as char } else { '_' }));
        host = tag.end;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc(name: &str, text: &str) -> Document { Document::decode(PathBuf::from(format!("/p/{name}")), 1, text.as_bytes()) }

    fn regions(p: &Projected, text: &str) -> Vec<String> {
        p.spans.iter().map(|s| text.get(s.source.clone()).unwrap().to_owned()).collect()
    }

    #[test]
    fn template_files_are_known_by_their_last_extension_and_only_html_erb_serves_markup() {
        assert_eq!(Kind::of(Path::new("/a/show.html.erb")), Some(Kind::Erb));
        assert_eq!(Kind::of(Path::new("/a/mail.text.ERB")), Some(Kind::Erb));
        assert_eq!(Kind::of(Path::new("/a/page.html.haml")), Some(Kind::Haml));
        assert_eq!(Kind::of(Path::new("/a/erb.rb")), None);
        assert_eq!(languages(Path::new("/a/show.html.erb")), &[Language::Ruby, Language::Html]);
        assert_eq!(languages(Path::new("/a/show.js.erb")), &[Language::Ruby]);
        assert_eq!(languages(Path::new("/a/show.html.haml")), &[Language::Ruby]);
        assert!(languages(Path::new("/a/show.html")).is_empty());
        // Rails variants: `show.html+phone.erb` is an HTML template.
        assert_eq!(languages(Path::new("/a/show.html+phone.erb")), &[Language::Ruby, Language::Html]);
        assert_eq!(languages(Path::new("/a/show.HTM+tablet.ERB")), &[Language::Ruby, Language::Html]);
        assert_eq!(languages(Path::new("/a/show.json+phone.erb")), &[Language::Ruby]);
        assert_eq!(languages(Path::new("/a/html+x.erb")), &[Language::Ruby], "no format extension");
    }

    #[test]
    fn erb_regions_follow_erubi_and_refuse_escaped_closers() {
        let text = "<p>𝄞 <%= greet(\"é\") %></p>\r\n<%- if a -%>\r\n<%# note %><%% raw %>\r\n<%== b =%>\r\n<% x = [1,\r\n  2] %>tail <%= open";
        let p = erb_ruby(text, None).unwrap();
        assert_eq!(regions(&p, text), [" greet(\"é\") ", " if a ", " b ", " x = [1,\r\n  2] ", " open"]);
        assert_eq!(p.text, "def LenzTemplate_render(&_)\n greet(\"é\") \n if a \n b \n x = [1,\r\n  2] \n open\nend\n");
        for s in &p.spans { assert_eq!(p.text.get(s.generated..s.generated + s.source.len()), text.get(s.source.clone())); }
        let why = erb_ruby("<% a %>\n<%= 5 %%>", None).unwrap_err();
        assert!(why.starts_with("line 2: `%%>`"), "{why}");
        // Comments and literals may end so; their text is never Ruby.
        assert!(erb_ruby("<%# 5 %%>", None).is_ok());
    }

    #[test]
    fn erb_markup_keeps_host_bytes_and_hides_tags_line_for_line() {
        let text = "<a href=\"<%= url %>\">𝄞</a>\n<script>var d = <%= raw j %>;</script><%\n x %>";
        let p = erb_markup(text).unwrap();
        assert_eq!(p.text.len(), text.len());
        assert_eq!(p.text, "<a href=\"__________\">𝄞</a>\n<script>var d = ____________;</script>__\n_____");
        assert_eq!(regions(&p, text), ["<a href=\"", "\">𝄞</a>\n<script>var d = ", ";</script>", ""]);
    }

    #[test]
    fn projection_offsets_map_both_ways_and_scaffolding_maps_to_nothing() {
        let d = doc("v.html.erb", "a<%= bc %>d<% e %>");
        let views = d.templates().unwrap();
        let ruby = views.language(Language::Ruby).unwrap().embedded().unwrap();
        assert_eq!(ruby.generated().text().as_ref(), "def LenzTemplate_render(&_)\n bc \n e \nend\n");
        let b = d.text().find('b').unwrap();
        let at = "def LenzTemplate_render(&_)\n ".len();
        assert_eq!(ruby.to_generated(b), Some(at));
        assert_eq!(ruby.to_source(at), Some(b));
        assert_eq!(ruby.to_generated(0), None, "host text");
        assert_eq!(ruby.to_generated(d.text().find("%>").unwrap() + 1), None, "a delimiter");
        assert_eq!(ruby.to_source(ruby.generated().text().len()), None, "the projection's last newline");
        assert_eq!(ruby.to_source(at - 2), None, "the method's own line");
        assert_eq!(ruby.near_source(ruby.generated().text().len()), d.text().len() - 2);
        assert_eq!(views.at(b).unwrap().embedded().unwrap().route().language, Language::Ruby);
        assert_eq!(views.at(0).unwrap().embedded().unwrap().route().language, Language::Html);
        assert!(views.at(2).unwrap_err().contains("delimiters"), "inside `<%=`");
        let plain = doc("v.text.erb", "a<%= b %>");
        assert!(plain.templates().unwrap().at(0).unwrap_err().contains("serves only its embedded Ruby"));
    }

    #[test]
    fn haml_maps_scripts_interpolation_and_continuations_and_refuses_the_rest() {
        let text = "!!!\r\n%html\r\n  %body.page#main(title=\"😀\" class=\"x\")\r\n    -# a comment\r\n      %p= nothing\r\n    %p= greet(\"é\", |\r\n        excited: true) |\r\n    %p Hi #{name}, 𝄞 #{other}\r\n    - total = 1\r\n    .note= total\r\n";
        let p = haml_ruby(text).unwrap();
        assert_eq!(regions(&p, text), ["greet(\"é\", ", "excited: true) ", "name", "other", "total = 1", "total"]);
        for (bad, why) in [
            (":javascript\n  x\n", "filters"),
            ("%p #{unfinished\n", "interpolation"), ("%p= a\n  %b c\n", "nested under"), ("\t%p\n", "tab"),

        ] {
            let found = haml_ruby(bad).unwrap_err();
            assert!(found.contains(why), "{bad:?}: {found}");
        }
        assert!(haml_ruby("%p text \\#{not}\n").unwrap().spans.is_empty(), "escaped interpolation");
    }

    #[test]
    fn problems_about_the_projection_rather_than_the_template_are_dropped() {
        let d = doc("v.html.erb", "<% x = 1 %><%= x %>");
        let views = d.templates().unwrap();
        let ruby = views.language(Language::Ruby).unwrap().embedded().unwrap();
        let void = "possibly useless use of a variable in void context";
        assert!(!ruby.keeps_problem(Some("Prism"), void, Some(ruby.spans()[1].generated + 1)), "an output is rendered, not void");
        assert!(ruby.keeps_problem(Some("Prism"), void, Some(1)), "a statement may be void");
        assert!(!ruby.keeps_problem(Some("RuboCop"), "Layout/TrailingWhitespace: Trailing whitespace detected.", Some(1)));
        assert!(ruby.keeps_problem(Some("Prism"), "unexpected ']'", Some(1)));
        assert!(views.language(Language::Html).unwrap().embedded().unwrap().keeps_problem(Some("css"), void, Some(0)));
    }

    fn edit(text: &str, needle: &str, skip: usize, len: usize, new: &str) -> TextEdit {
        let at = text.find(needle).unwrap() + skip;
        TextEdit { range: at..at + len, text: new.into() }
    }

    #[test]
    fn edits_apply_only_inside_one_region_and_never_change_the_templates_own_syntax() {
        let text = "<p><%= greet %></p>\n<% if ok %>\n";
        let d = doc("v.html.erb", text);
        let views = d.templates().unwrap();
        let ruby = views.language(Language::Ruby).unwrap().embedded().unwrap();
        let read = |r: Range<usize>| text.get(r).map(str::to_owned);
        let check = |e: TextEdit| ruby.check(&[e], &[], &read);
        assert_eq!(check(edit(text, "greet", 0, 5, "greeting(name)")), Ok(()));
        assert_eq!(check(edit(text, "greet", 5, 1, "")), Ok(()), "the region's own space");
        for bad in [edit(text, "greet", 0, 0, "%> x <%= "), edit(text, " if", 0, 0, "#"), edit(text, " if", 0, 0, "="), edit(text, " ok ", 4, 0, "-"), edit(text, " ok ", 4, 0, "%")] {
            assert_eq!(check(bad), Err("it would change the template's own tags, delimiters or lines".into()));
        }
        assert_eq!(check(edit(text, "%></p>", 0, 2, "")), Err("it reaches outside the template's embedded Ruby".into()));
        assert_eq!(check(edit(text, "greet %></p>\n<%", 0, 16, "x")), Err("it reaches outside the template's embedded Ruby".into()), "two regions");
        // Every caret of a transaction, or none.
        let both = [edit(text, "greet", 0, 0, "a"), edit(text, "<p>", 0, 0, "b")];
        assert!(ruby.check(&both, &[], &read).is_err());
        // Markup: host text only, and never a new tag.
        let markup = views.language(Language::Html).unwrap().embedded().unwrap();
        assert_eq!(markup.check(&[edit(text, "</p>", 0, 0, "<b></b>")], &[], &read), Ok(()));
        assert!(markup.check(&[edit(text, "</p>", 0, 0, "<% x %>")], &[], &read).is_err());
        assert!(markup.check(&[edit(text, "greet", 0, 1, "")], &[], &read).is_err());
    }

    #[test]
    fn typing_since_the_projection_carries_its_regions() {
        let text = "<%= gr %>";
        let d = doc("v.erb", text);
        let ruby = d.templates().unwrap().primary().unwrap().embedded().unwrap().clone();
        // `e` typed at the end of `gr`, then the suggestion replaces the name.
        let typed = vec![vec![TextEdit { range: 6..6, text: "e".into() }]];
        let now = "<%= gre %>";
        let read = |r: Range<usize>| now.get(r).map(str::to_owned);
        assert_eq!(ruby.check(&[TextEdit { range: 4..7, text: "greet".into() }], &typed, &read), Ok(()));
        assert!(ruby.check(&[TextEdit { range: 4..9, text: "greet".into() }], &typed, &read).is_err(), "over `%>`");
    }

    #[test]
    fn erb_ruby_runs_inside_a_method_as_templates_are_compiled() {
        let d = doc("v.text.erb", "<%= yield %>\n<% x = 1 %>");
        let ruby = d.templates().unwrap().primary().unwrap().embedded().unwrap().clone();
        assert_eq!(ruby.generated().text().as_ref(), "def LenzTemplate_render(&_)\n yield \n x = 1 \nend\n");
        let y = d.text().find("yield").unwrap();
        assert_eq!(ruby.to_generated(y), Some("def LenzTemplate_render(&_)\n ".len()));
        assert_eq!(ruby.to_source(0), None, "the method's own line is scaffolding");
        assert_eq!(ruby.near_source(ruby.generated().text().len() - 1), d.text().len() - 2, "its `end` is placed after the last region");
        assert!(scaffolding("LenzTemplate_render") && scaffolding("LenzTemplate::View_app_views_a_html_erb") && !scaffolding("link_to"));
    }

    #[test]
    fn a_rails_view_runs_as_action_view_with_its_helpers_and_strict_locals() {
        let text = "<%# locals: (post:, title: \"é\") -%>\r\n<p>𝄞 <%= link_to title, post %></p>\r\n<%= yield %>";
        let d = Document::decode(PathBuf::from("/srv/blog/app/views/admin/posts/_form.html+phone.erb"), 1, text.as_bytes());
        let views = d.templates().unwrap();
        let ruby = views.language(Language::Ruby).unwrap().embedded().unwrap();
        assert_eq!(ruby.generated().text().as_ref(), concat!(
            "class LenzTemplate::View_app_views_admin_posts__form_html_phone_erb < ActionView::Base\n",
            "include ApplicationHelper\ninclude Admin::PostsHelper\n",
            "def LenzTemplate_render(local_assigns, output_buffer, post:, title: \"é\", &_)\n",
            " link_to title, post \n yield \nend\nend\n"));
        let p = Projected { text: ruby.generated().text().to_string(), spans: ruby.spans().to_vec() };
        assert_eq!(regions(&p, text), ["post:, title: \"é\"", " link_to title, post ", " yield "]);
        for s in &p.spans { assert_eq!(p.text.get(s.generated..s.generated + s.source.len()), text.get(s.source.clone())); }
        // An `.html+variant.erb` view's markup is HTML too.
        assert!(views.language(Language::Html).is_some());
        // The locals are Ruby (their problems land on them) but never edited through the projection.
        let title = text.find("title:").unwrap();
        assert_eq!(views.at(title).unwrap().embedded().unwrap().route().language, Language::Ruby);
        let read = |r: Range<usize>| text.get(r).map(str::to_owned);
        assert_eq!(ruby.check(&[edit(text, "title:", 0, 5, "name")], &[], &read), Err("it is inside the template's `locals:` comment, which Lenz does not edit through a language server".into()));
        assert_eq!(ruby.check(&[edit(text, "link_to", 0, 7, "link_to_if")], &[], &read), Ok(()));
        // Summary: the context Ruby LSP is told, and what it is not.
        let rows = views.summary();
        assert!(rows.iter().any(|r| r == "ERB template view context: a Rails view (Action View), with ApplicationHelper and Admin::PostsHelper and the strict locals `post:, title: \"é\"`; controller instance variables and route helpers are not known to Ruby LSP's index"), "{rows:?}");
    }

    #[test]
    fn strict_locals_are_read_as_action_view_reads_them() {
        let locals = |text: &str| strict_locals(text).map(|(whole, group)| (text[whole].to_owned(), text[group].to_owned()));
        assert_eq!(locals("<%# locals: (a:, b: 1) %>"), Some(("# locals: (a:, b: 1)".into(), "a:, b: 1".into())));
        assert_eq!(locals("<%#  locals:  (a: f(1))  -%>"), Some(("#  locals:  (a: f(1))".into(), "a: f(1)".into())), "the first `)` the tag's end follows");
        assert_eq!(locals("<%# locals: (a:)\n other %>"), Some(("# locals: (a:)".into(), "a:".into())), "a `)` ending a line");
        assert_eq!(locals("<%# locals: (a: ) %>").map(|l| l.1), Some("a:".into()), "trailing spaces dropped");
        assert_eq!(locals("<%# locals: (a:) b %>"), None);
        assert_eq!(locals("<%# locals:(a:) %>"), None, "a space after `locals:`");
        assert_eq!(locals("<%# locals: (a) b %>\n<%# locals: (c:) %>").map(|l| l.1), Some("a) b %>\n<%# locals: (c:".into()), "lazily on to a `)` the tag's end follows, as the regex reads it");
        assert_eq!(locals("<%# locals: (a: \"x)\" ) %>").map(|l| l.1), Some("a: \"x)\"".into()));
        let ruby = |name: &str, text: &str| Document::decode(PathBuf::from(format!("/r/app/views/{name}")), 1, text.as_bytes()).templates().unwrap().primary().unwrap().embedded().unwrap().generated().text().to_string();
        assert!(ruby("posts/a.html.erb", "<%# locals: () %>").contains("(local_assigns, output_buffer, **nil, &_)\n"), "no locals at all");
        assert!(ruby("posts/a.html.erb", "<%# locals: (a:, &blk) %>").contains("(local_assigns, output_buffer, a:, &blk)\n"), "its own block");
        // After Ruby already projected (not where Rails' comment goes): told to Ruby LSP, not mapped.
        let late = "<%= a %><%# locals: (a:) %>";
        let d = Document::decode(PathBuf::from("/r/app/views/posts/late.html.erb"), 1, late.as_bytes());
        let p = d.templates().unwrap().primary().unwrap().embedded().unwrap().clone();
        assert!(p.generated().text().contains("output_buffer, a:, &_)\n"));
        assert_eq!(p.spans().len(), 1);
        // A mailer's views have its helper and the mail helper, not the application's.
        assert!(ruby("user_mailer/welcome.text.erb", "").contains("include ActionMailer::MailHelper\ninclude UserMailerHelper\n"));
        // A folder name that is not a constant's: no controller helper.
        assert!(ruby("my-posts/a.html.erb", "").starts_with("class LenzTemplate::View_app_views_my_posts_a_html_erb < ActionView::Base\ninclude ApplicationHelper\ndef "));
    }

    #[test]
    fn haml_edits_keep_their_line_kind() {
        let text = "%p= greet\n%p Hi #{name}\n";
        let d = doc("v.haml", text);
        let ruby = d.templates().unwrap().primary().unwrap().embedded().unwrap().clone();
        let read = |r: Range<usize>| text.get(r).map(str::to_owned);
        assert_eq!(ruby.check(&[edit(text, "greet", 0, 5, "greet(\"😀\")")], &[], &read), Ok(()));
        assert_eq!(ruby.check(&[edit(text, "name", 0, 4, "full_name")], &[], &read), Ok(()));
        for bad in [edit(text, "greet", 5, 0, "\n%b x"), edit(text, "name", 4, 0, "}"), edit(text, "greet", 5, 0, " |")] {
            assert!(ruby.check(std::slice::from_ref(&bad), &[], &read).is_err(), "{bad:?}");
        }
    }
}

#[cfg(test)]
mod haml_safety_tests {
    use super::*;
    #[test]
    fn haml_block_shape_and_synthetic_rails_context_are_never_editable() {
        let source = "- if user\r\n  %p{title: user.name}= user.name\r\n";
        let doc = Document::decode("/project/app/views/users/show.html.haml".into(), 7, source.as_bytes());
        let view = doc.templates().unwrap().primary().unwrap().embedded().unwrap();
        let generated = view.generated().text();
        assert!(generated.contains("include ::UsersHelper\n"));
        assert!(view.to_source(0).is_none());
        let read = |range| source.get(range).map(str::to_owned);
        let at = source.find("if user").unwrap();
        assert!(view.check(&[TextEdit { range: at..at + 7, text: "other".into() }], &[], &read).is_err());
        let at = source.find("user.name").unwrap();
        assert_eq!(view.check(&[TextEdit { range: at..at + 9, text: "user.title".into() }], &[], &read), Ok(()));
        assert!(view.check(&[TextEdit { range: at..at + 9, text: "user.name}\n%p bad".into() }], &[], &read).is_err());
    }
}

#[cfg(test)]
mod haml_symbol_tests {
    use super::*;
    #[test]
    fn generated_rails_context_never_appears_as_a_source_symbol() {
        let doc = Document::decode("/project/app/views/posts/show.html.haml".into(), 1, b"%p= user.name\n");
        let view = doc.templates().unwrap().primary().unwrap().clone();
        let symbols = serde_json::json!([{ "name": "synthetic", "kind": 5, "selectionRange": {
            "start": { "line": 0, "character": 6 }, "end": { "line": 0, "character": 20 }
        } }]);
        let found = crate::semantics::file_symbols(&symbols, &view, crate::semantics::lsp::Encoding::Utf16, 100);
        assert!(found.found.is_empty() && found.failed.is_empty());
    }
}

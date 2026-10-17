//! The PR reading tabs as pages of rows and card boxes, laid out on a column grid both frontends draw:
//! Conversation (description, comments, reviews with their threads, side panel) and Commits.
use crate::pr::{Comment, DiscussionComment, PrCommit, PullRequest, Review, State, Thread};
use crate::rich::{self, Fill, Row, Span, Style, Target, Tone};
use std::collections::HashSet;

/// A card's border around rows `top..bottom` (border rows included) and columns `left..right`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardBox { pub top: usize, pub bottom: usize, pub left: usize, pub right: usize, pub tone: Tone,
    /// The card's comment, for Copy comment / Copy link.
    pub body: String, pub url: String }

/// A laid-out tab. `side` rows sit at column `side_at` beside the main rows (0: stacked above them instead).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page { pub rows: Vec<Row>, pub boxes: Vec<CardBox>, pub side: Vec<Row>, pub side_at: usize, pub markdown: Vec<MarkdownRegion> }

/// A body rendered from one shared Markdown document; the GUI uses native typography to reflow its rows.
#[derive(Clone, Debug)]
pub struct MarkdownRegion { pub rows: std::ops::Range<usize>, pub columns: std::ops::Range<usize>, pub document: std::sync::Arc<lenz_content::markdown::MarkdownDocument> }
impl PartialEq for MarkdownRegion {
    fn eq(&self, other: &Self) -> bool { self.rows == other.rows && self.columns == other.columns && self.document.source == other.document.source }
}
impl Eq for MarkdownRegion {}

impl Page {
    /// What a click at a row and column of the page does.
    pub fn target_at(&self, row: usize, column: usize) -> Option<&Target> {
        let side = (self.side_at > 0 && column >= self.side_at).then(|| self.side.get(row)).flatten();
        side.or_else(|| self.rows.get(row))?.span_at(column)?.target.as_ref()
    }
    /// The innermost card at a row and column.
    pub fn card_at(&self, row: usize, column: usize) -> Option<&CardBox> {
        self.boxes.iter().filter(|b| (b.top..b.bottom).contains(&row) && (b.left..b.right).contains(&column)).min_by_key(|b| b.right - b.left)
    }
    /// The row a target first appears on, to scroll to it.
    pub fn row_of(&self, target: &Target) -> Option<usize> {
        self.rows.iter().position(|r| r.spans.iter().any(|s| s.target.as_ref() == Some(target)))
    }
    /// The text between two (row, column) cells, rows joined with newlines; the side panel's when `from` is in it.
    pub fn text(&self, from: (usize, usize), to: (usize, usize)) -> String {
        let ((r0, c0), (r1, c1)) = if from <= to { (from, to) } else { (to, from) };
        let side = self.side_at > 0 && from.1.min(to.1) >= self.side_at;
        let rows = if side { &self.side } else { &self.rows };
        (r0..=r1).filter_map(|r| rows.get(r)).enumerate().map(|(i, row)| {
            let text = row.text();
            let end = if r0 + i == r1 { c1 } else { usize::MAX };
            let start = if i == 0 { c0 } else { row.indent };
            let mut column = row.indent;
            let mut previous = false;
            text.chars().filter(|c| {
                let width = rich::width(c.encode_utf8(&mut [0; 4]));
                let selected = if width == 0 { previous } else { column >= start && column < end };
                column += width;
                previous = selected;
                selected
            }).collect::<String>().trim_end().to_string()
        }).collect::<Vec<_>>().join("\n")
    }
    pub fn height(&self) -> usize { if self.side_at > 0 { self.rows.len().max(self.side.len()) } else { self.rows.len() } }
}

/// The narrowest a page is laid out: three nested cards and a word.
const MIN_WIDTH: usize = 24;

/// What loading the comments and reviews has reached.
pub enum Loaded<'a> { Ready(&'a Review), Loading, Failed(&'a str) }

/// Unix seconds of an ISO 8601 UTC time (`2026-10-08T10:00:00Z`); `None` when it is not one.
pub fn unix(iso: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| iso.get(r)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19).unwrap_or(0));
    // Howard Hinnant's days_from_civil
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

/// "3 days ago", as GitHub says it; the date itself past a month.
pub fn ago(iso: &str, now: i64) -> String {
    let Some(at) = unix(iso) else { return iso.to_string() };
    let s = (now - at).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match s {
        0..60 => "just now".into(),
        60..3600 => plural(s / 60, "minute"),
        3600..86_400 => plural(s / 3600, "hour"),
        86_400..2_592_000 => plural(s / 86_400, "day"),
        _ => format!("on {}", iso.get(..10).unwrap_or(iso)),
    }
}

/// A thread's page key, for folding it.
pub fn thread_key(t: &Thread) -> String {
    format!("thread:{}:{}:{}", t.path, t.line.or(t.original_line).unwrap_or(0), t.comments.first().map_or("", |c| c.created_at.as_str()))
}

fn login(c: &Comment) -> &str { c.author.as_ref().map_or("ghost", |a| a.login.as_str()) }

/// A review verdict's badge.
pub fn verdict(state: &str) -> (&'static str, Tone) {
    match state {
        "APPROVED" => ("✓ Approved", Tone::Good),
        "CHANGES_REQUESTED" => ("✗ Changes requested", Tone::Bad),
        "DISMISSED" => ("Dismissed", Tone::Dim),
        _ => ("Commented", Tone::Dim),
    }
}

/// The PR header both tabs start with.
fn header(pr: &PullRequest, commits: u64, width: usize, rows: &mut Vec<Row>) {
    let (state, tone) = match (pr.state, pr.is_draft) {
        (_, true) => ("○ Draft", Tone::Dim),
        (State::Open, _) => ("● Open", Tone::Good),
        (State::Merged, _) => ("● Merged", Tone::Accent),
        (State::Closed, _) => ("● Closed", Tone::Bad),
    };
    let mut title = Vec::new();
    rich::wrap(&[rich::Block { spans: vec![Span::new(format!("{}  ", pr.title), Style { bold: true, ..Style::default() }), Span::tone(format!("#{}", pr.number), Tone::Dim)], ..Default::default() }], width, 0, &mut title);
    rows.extend(title);
    let commits = if commits == 1 { "1 commit".to_string() } else { format!("{commits} commits") };
    rows.push(Row { spans: vec![Span::tone(state, tone), Span::plain("  "), Span::bold(pr.author.login.clone()), Span::tone(format!(" wants to merge {commits} into "), Tone::Dim),
        Span::tone(pr.base_ref_name.clone(), Tone::Accent), Span::tone(" from ", Tone::Dim), Span::tone(pr.head_ref_name.clone(), Tone::Accent)], ..Row::default() });
    rows.push(Row { spans: vec![Span::tone(format!("+{}", pr.additions), Tone::Added), Span::plain(" "), Span::tone(format!("−{}", pr.deletions), Tone::Removed),
        Span::tone(format!(" · {} files · ", pr.changed_files), Tone::Dim), Span::tone("Open on GitHub ↗", Tone::Accent).to(Target::Url(pr.url.clone()))], ..Row::default() });
    rows.push(Row { fill: Fill::Rule, ..Row::default() });
    rows.push(Row::default());
}

/// Lays cards out on the grid: a border row, a header row, the body, a border row.
struct Cards<'a> { page: &'a mut Page, now: i64, toggled: &'a HashSet<String> }

impl Cards<'_> {
    /// A card across columns `left..right`; `title` fills its header row, `badge` sits at its right end.
    fn card(&mut self, (left, right): (usize, usize), title: Vec<Span>, badge: Option<Span>, tone: Tone, comment: &Comment, fill: impl FnOnce(&mut Self, usize, usize)) {
        let top = self.page.rows.len();
        self.page.rows.push(Row::default());
        let mut spans = title;
        if let Some(badge) = badge {
            let used = rich::width(&spans.iter().map(|s| s.text.as_str()).collect::<String>()) + rich::width(&badge.text);
            spans.push(Span::plain(" ".repeat(right.saturating_sub(left).saturating_sub(4 + used).max(1))));
            spans.push(badge);
        }
        self.page.rows.push(Row { spans, indent: left + 2, fill: Fill::None });
        fill(self, left + 2, right.saturating_sub(2).max(left + 3));
        self.page.rows.push(Row::default());
        self.page.boxes.push(CardBox { top, bottom: self.page.rows.len(), left, right, tone, body: comment.body.clone(), url: comment.url.clone() });
    }
    fn body(&mut self, text: &str, left: usize, right: usize) {
        if text.trim().is_empty() {
            self.page.rows.push(Row { spans: vec![Span::tone("No description provided.", Tone::Dim)], indent: left, fill: Fill::None });
            return;
        }
        let markdown = rich::markdown(text);
        let start = self.page.rows.len();
        rich::wrap(&markdown, right, left, &mut self.page.rows);
        if let Some(document) = markdown.document { self.page.markdown.push(MarkdownRegion { rows: start..self.page.rows.len(), columns: left..right, document }); }
    }
    fn who(&self, c: &Comment, verb: &str) -> Vec<Span> {
        vec![Span::bold(login(c).to_string()), Span::tone(format!(" {verb}{}", ago(&c.created_at, self.now)), Tone::Dim)]
    }
    /// A review thread: `path:line ↗`, the code it is on, its replies. Resolved and outdated threads start folded.
    fn thread(&mut self, t: &Thread, left: usize, right: usize) {
        let Some(first) = t.comments.first() else { return };
        let key = thread_key(t);
        let folded = (t.is_resolved || t.is_outdated) != self.toggled.contains(&key);
        let line = t.line.or(t.original_line);
        let mut title = vec![Span::tone(if folded { "▸ " } else { "▾ " }, Tone::Dim).to(Target::Toggle(key.clone()))];
        let place = line.filter(|_| t.subject_type != "FILE").map_or_else(|| t.path.clone(), |l| format!("{}:{l}", t.path));
        title.push(match (t.line, t.is_outdated) {
            (Some(l), false) => Span::tone(format!("{place} ↗"), Tone::Accent).to(Target::Line { path: t.path.clone(), line: l, left: t.diff_side == crate::view::Side::Left }),
            _ => Span::bold(place),
        });
        let state = [(t.is_outdated, " · outdated"), (t.is_resolved, " · resolved")].iter().filter(|(on, _)| *on).map(|(_, s)| *s).collect::<String>();
        let replies = t.comments.len();
        title.push(Span::tone(format!("{state}{}", if folded { format!(" · {replies} comment{}", if replies == 1 { "" } else { "s" }) } else { String::new() }), Tone::Dim).to(Target::Toggle(key)));
        let tone = if folded { Tone::Dim } else { Tone::Normal };
        self.card((left, right), title, None, tone, first, |cards, l, r| {
            if folded { return; }
            // GitHub shows the last lines of the hunk, down to the commented line
            let hunk: Vec<&str> = first.diff_hunk.lines().filter(|l| !l.starts_with("@@")).collect();
            for code in &hunk[hunk.len().saturating_sub(4)..] {
                rich::wrap(&[rich::code_line(code, 0, true)], r.saturating_sub(l), l, &mut cards.page.rows);
            }
            for (i, c) in t.comments.iter().enumerate() {
                if i > 0 { cards.page.rows.push(Row::default()); }
                cards.page.rows.push(Row { spans: cards.who(c, ""), indent: l, fill: Fill::None });
                cards.body(&c.body, l, r);
            }
        });
    }
}

/// The Conversation tab at `width` columns. `now` is Unix seconds, for "3 days ago".
#[cold]
pub fn conversation(pr: &PullRequest, loaded: Loaded, toggled: &HashSet<String>, width: usize, now: i64) -> Page {
    // narrower than this, the page is cut at the pane's edge rather than squeezed
    let width = width.max(MIN_WIDTH);
    let mut page = Page::default();
    let review = match loaded { Loaded::Ready(r) => Some(r), _ => None };
    let commits = review.map_or(0, |r| r.overview.commit_total);
    // a side panel beside the timeline when there is room, else above it
    let (main, side_w) = if width >= 100 { (width - 32, 30) } else { (width, 0) };
    header(pr, commits, main, &mut page.rows);
    if side_w > 0 { page.side_at = main + 2; page.side = side_panel(review, side_w, main + 2); }
    else if let Some(review) = review {
        // no room beside the timeline: the panel's facts on a line under the header
        rich::wrap(&[rich::Block { spans: side_summary(&review.overview), ..Default::default() }], width, 0, &mut page.rows);
        page.rows.push(Row::default());
    }
    let author = Comment { author: Some(pr.author.clone()), body: pr.body.clone(), created_at: review.map_or_else(String::new, |r| r.overview.created_at.clone()), url: pr.url.clone(), ..Comment::default() };
    let mut cards = Cards { page: &mut page, now, toggled };
    cards.card((0, main), cards.who(&author, "opened "), None, Tone::Normal, &author, |c, l, r| c.body(&pr.body, l, r));
    let review = match loaded {
        Loaded::Ready(r) => r,
        Loaded::Loading => { page.rows.push(Row { spans: vec![Span::tone("Loading comments and reviews…", Tone::Dim)], ..Row::default() }); return page; }
        Loaded::Failed(e) => { page.rows.push(Row { spans: vec![Span::tone(format!("Could not load comments and reviews: {e}"), Tone::Bad)], ..Row::default() }); return page; }
    };
    let threads: Vec<&Thread> = review.placed.values().chain(review.listed.values()).flatten().filter(|t| !t.comments.is_empty()).collect();
    let in_review = |id: &str| -> Vec<&Thread> { threads.iter().copied().filter(|t| t.comments[0].pull_request_review.as_ref().is_some_and(|r| r.id == id)).collect() };
    let reviewed: HashSet<&str> = review.discussion.iter().filter_map(|d| d.review_id.as_deref()).collect();
    // the timeline: comments, reviews (their threads inside), and threads whose review is not listed
    enum Event<'a> { Said(&'a DiscussionComment), Lone(&'a Thread) }
    let mut events: Vec<(&str, Event)> = review.discussion.iter().map(|d| (d.comment.created_at.as_str(), Event::Said(d))).collect();
    events.extend(threads.iter().filter(|t| t.comments[0].pull_request_review.as_ref().is_none_or(|r| !reviewed.contains(r.id.as_str())))
        .map(|t| (t.comments[0].created_at.as_str(), Event::Lone(t))));
    events.sort_by_key(|(at, _)| *at);
    for (_, event) in events {
        match event {
            Event::Said(d) => match (&d.review_state, &d.review_id) {
                (Some(state), id) => {
                    let mine: Vec<&Thread> = id.as_deref().map_or_else(Vec::new, in_review);
                    // GitHub leaves out an empty "commented" review with nothing under it
                    if state == "COMMENTED" && d.comment.body.trim().is_empty() && mine.is_empty() { continue; }
                    let (badge, tone) = verdict(state);
                    let verb = match state.as_str() { "APPROVED" => "approved ", "CHANGES_REQUESTED" => "requested changes ", _ => "reviewed " };
                    let title = cards.who(&d.comment, verb);
                    cards.card((0, main), title, Some(Span::tone(badge, tone)), tone, &d.comment, |c, l, r| {
                        if !d.comment.body.trim().is_empty() { c.body(&d.comment.body, l, r); }
                        for t in mine { c.thread(t, l, r); }
                    });
                }
                (None, _) => {
                    let title = cards.who(&d.comment, "commented ");
                    cards.card((0, main), title, None, Tone::Normal, &d.comment, |c, l, r| c.body(&d.comment.body, l, r));
                }
            },
            Event::Lone(t) => cards.thread(t, 0, main),
        }
    }
    page
}

/// The side panel as one line: reviewers, the checks' outcome, labels.
fn side_summary(o: &crate::pr::Overview) -> Vec<Span> {
    let mut spans = vec![Span::tone("Reviewers ", Tone::Dim)];
    for (who, state) in &o.reviewers {
        let (mark, tone) = match state.as_str() { "APPROVED" => ("✓", Tone::Good), "CHANGES_REQUESTED" => ("✗", Tone::Bad), _ => ("💬", Tone::Dim) };
        spans.extend([Span::tone(mark, tone), Span::plain(format!("{who} "))]);
    }
    for who in o.requested.iter().filter(|w| !o.reviewers.iter().any(|(r, _)| r == *w)) { spans.extend([Span::tone("○", Tone::Warn), Span::plain(format!("{who} "))]); }
    if o.reviewers.is_empty() && o.requested.is_empty() { spans.push(Span::tone("none ", Tone::Dim)); }
    let failing = o.check_runs.iter().filter(|(_, s)| matches!(s.as_str(), "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED")).count();
    spans.push(Span::tone("· Checks ", Tone::Dim));
    spans.push(match o.checks.as_deref() {
        None => Span::tone("none ", Tone::Dim),
        Some("SUCCESS") => Span::tone(format!("✓ {} passing ", o.check_runs.len()), Tone::Good),
        Some(_) if failing > 0 => Span::tone(format!("✗ {failing} failing "), Tone::Bad),
        Some(_) => Span::tone("● pending ", Tone::Warn),
    });
    if !o.labels.is_empty() {
        spans.push(Span::tone("· Labels ", Tone::Dim));
        spans.push(Span::tone(o.labels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", "), Tone::Accent));
    }
    spans
}

/// Reviewers, checks, labels: the Conversation's side panel, its rows starting at column `at`.
fn side_panel(review: Option<&Review>, width: usize, at: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let row = |spans: Vec<Span>| Row { spans, indent: at, fill: Fill::None };
    let Some(o) = review.map(|r| &r.overview) else { return rows };
    rows.push(row(vec![Span::bold("Reviewers")]));
    if o.reviewers.is_empty() && o.requested.is_empty() { rows.push(row(vec![Span::tone("No reviews", Tone::Dim)])); }
    for (who, state) in &o.reviewers {
        let (mark, tone) = match state.as_str() { "APPROVED" => ("✓ ", Tone::Good), "CHANGES_REQUESTED" => ("✗ ", Tone::Bad), _ => ("💬 ", Tone::Dim) };
        rows.push(row(vec![Span::tone(mark, tone), Span::plain(who.clone())]));
    }
    for who in o.requested.iter().filter(|w| !o.reviewers.iter().any(|(r, _)| r == *w)) {
        rows.push(row(vec![Span::tone("○ ", Tone::Warn), Span::plain(who.clone()), Span::tone(" (requested)", Tone::Dim)]));
    }
    rows.push(row(Vec::new()));
    rows.push(row(vec![Span::bold("Checks")]));
    match &o.checks {
        None => rows.push(row(vec![Span::tone("No checks", Tone::Dim)])),
        Some(state) => {
            let failed: Vec<&(String, String)> = o.check_runs.iter().filter(|(_, s)| matches!(s.as_str(), "FAILURE" | "ERROR" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED")).collect();
            let (mark, tone) = match state.as_str() { "SUCCESS" => ("✓ ", Tone::Good), "FAILURE" | "ERROR" => ("✗ ", Tone::Bad), _ => ("● ", Tone::Warn) };
            rows.push(row(vec![Span::tone(mark, tone), Span::plain(format!("{} checks · {} failing", o.check_runs.len(), failed.len()))]));
            for (name, _) in failed { rich::wrap(&[rich::Block { spans: vec![Span::tone("✗ ", Tone::Bad), Span::plain(name.clone())], indent: 2, ..Default::default() }], width, at, &mut rows); }
        }
    }
    rows.push(row(Vec::new()));
    rows.push(row(vec![Span::bold("Labels")]));
    if o.labels.is_empty() { rows.push(row(vec![Span::tone("None yet", Tone::Dim)])); }
    for l in &o.labels { rows.push(row(vec![Span::tone("● ", Tone::Accent), Span::plain(l.name.clone())])); }
    rows
}

/// The Commits tab: the PR's commits, oldest first, each opening to its message.
#[cold]
pub fn commits(pr: &PullRequest, loaded: Loaded, toggled: &HashSet<String>, shown: Option<&str>, width: usize, now: i64) -> Page {
    let width = width.max(MIN_WIDTH);
    let mut page = Page::default();
    let review = match loaded { Loaded::Ready(r) => Some(r), _ => None };
    header(pr, review.map_or(0, |r| r.overview.commit_total), width, &mut page.rows);
    let Some(o) = review.map(|r| &r.overview) else {
        page.rows.push(Row { spans: vec![Span::tone("Loading commits…", Tone::Dim)], ..Row::default() });
        return page;
    };
    if shown.is_some() { page.rows.push(Row { spans: vec![Span::tone("Showing one commit in Files changed · ", Tone::Dim), Span::tone("Show all commits", Tone::Accent).to(Target::Commit(None))], ..Row::default() }); page.rows.push(Row::default()); }
    for c in &o.commits { commit(c, toggled, shown, width, now, &mut page.rows); }
    if o.commit_total as usize > o.commits.len() { page.rows.push(Row { spans: vec![Span::tone(format!("and {} more on GitHub", o.commit_total as usize - o.commits.len()), Tone::Dim)], ..Row::default() }); }
    page
}

fn commit(c: &PrCommit, toggled: &HashSet<String>, shown: Option<&str>, width: usize, now: i64, rows: &mut Vec<Row>) {
    let key = format!("commit:{}", c.sha);
    let open = toggled.contains(&key);
    let subject = c.message.lines().next().unwrap_or("");
    let here = shown == Some(c.sha.as_str());
    let mut spans = vec![Span::tone(if open { "▾ " } else { "▸ " }, Tone::Dim).to(Target::Toggle(key.clone())),
        Span::new(subject.to_string(), Style { bold: here, ..Style::default() }).to(Target::Toggle(key))];
    spans.push(Span::tone(format!("  {} committed {} · ", c.author, ago(&c.date, now)), Tone::Dim));
    spans.push(Span::tone(format!("{} ↗", c.sha.get(..7).unwrap_or(&c.sha)), Tone::Accent).to(Target::Commit(Some(c.sha.clone()))));
    rich::wrap(&[rich::Block { spans, indent: 2, marker: None, ..Default::default() }], width, 0, rows);
    if open {
        let body: Vec<&str> = c.message.trim_end().lines().skip(1).skip_while(|l| l.trim().is_empty()).collect();
        let blocks: Vec<rich::Block> = body.iter().map(|l| rich::Block { spans: vec![Span::tone(l.to_string(), Tone::Dim)], indent: 6, ..Default::default() }).collect();
        rich::wrap(&blocks, width, 0, rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::{Account, Id, Overview, PrCommit};

    fn pr() -> PullRequest {
        PullRequest { number: 42, title: "Add panel".into(), author: Account { login: "octocat".into() }, state: State::Open, is_draft: false,
            body: "## Summary\n\n- one".into(), base_ref_name: "main".into(), head_ref_name: "feat".into(), head_ref_oid: "h".into(),
            url: "https://github.com/o/r/pull/42".into(), additions: 3, deletions: 1, changed_files: 2, is_cross_repository: false,
            head_repository: None, head_repository_owner: None }
    }
    fn comment(who: &str, body: &str, at: &str) -> Comment {
        Comment { author: Some(Account { login: who.into() }), body: body.into(), created_at: at.into(), ..Comment::default() }
    }
    fn review() -> Review {
        let mut nit = comment("bob", "nit: rename x", "2026-10-09T00:00:01Z");
        nit.diff_hunk = "@@ -1,2 +1,2 @@\n a\n-let x = 1;\n+let x = 2;".into();
        nit.pull_request_review = Some(Id { id: "R1".into() });
        let thread = Thread { path: "a.rs".into(), diff_side: crate::view::Side::Right, line: Some(2), start_line: None, original_line: Some(2),
            is_outdated: false, is_resolved: false, comments: vec![nit, comment("octocat", "Done.", "2026-10-09T01:00:00Z")] , subject_type: String::new() };
        let mut resolved = thread.clone();
        resolved.is_resolved = true;
        resolved.comments[0].pull_request_review = None;
        resolved.comments[0].created_at = "2026-10-09T05:00:00Z".into();
        let mut r = Review { placed: [(("a.rs".into(), crate::view::Side::Right, 2), vec![thread, resolved])].into(), ..Review::default() };
        r.discussion = vec![
            DiscussionComment { comment: comment("carol", "First!", "2026-10-08T12:00:00Z"), ..Default::default() },
            DiscussionComment { comment: comment("bob", "Looks good", "2026-10-09T00:00:00Z"), review_state: Some("APPROVED".into()), review_id: Some("R1".into()) },
            DiscussionComment { comment: comment("dan", "", "2026-10-09T02:00:00Z"), review_state: Some("COMMENTED".into()), review_id: Some("R2".into()) },
        ];
        r.overview = Overview { created_at: "2026-10-08T10:00:00Z".into(), reviewers: vec![("bob".into(), "APPROVED".into())], requested: vec!["erin".into()],
            commit_total: 1, commits: vec![PrCommit { sha: "abcdef1234".into(), message: "Fix it\n\nBecause.".into(), author: "octocat".into(), date: "2026-10-08T09:00:00Z".into() }],
            checks: Some("FAILURE".into()), check_runs: vec![("build".into(), "FAILURE".into())], ..Overview::default() };
        r
    }
    const NOW: i64 = 1_791_590_400; // 2026-10-10T00:00:00Z

    fn text(rows: &[Row]) -> Vec<String> { rows.iter().map(|r| format!("{}{}", " ".repeat(r.indent), r.text()).trim_end().to_string()).collect() }

    #[test]
    fn times_read_like_github() {
        assert_eq!(unix("2026-10-10T00:00:00Z"), Some(NOW));
        assert_eq!(ago("2026-10-09T23:59:30Z", NOW), "just now");
        assert_eq!(ago("2026-10-09T22:00:00Z", NOW), "2 hours ago");
        assert_eq!(ago("2026-10-07T00:00:00Z", NOW), "3 days ago");
        assert_eq!(ago("2026-01-07T00:00:00Z", NOW), "on 2026-01-07");
    }

    #[test]
    fn copy_uses_the_same_display_columns_as_adapter_selection() {
        let page = Page { rows: vec![Row { spans: vec![Span::plain("界abc")], indent: 2, fill: Fill::None }], ..Default::default() };
        assert_eq!(page.text((0, 4), (0, 5)), "a");
        let page = Page { rows: vec![Row { spans: vec![Span::plain("\t界a\u{301}b")], ..Default::default() }], ..Default::default() };
        assert_eq!(page.text((0, 0), (0, 8)), "\t界a\u{301}b");
        assert_eq!(page.text((0, 6), (0, 7)), "a\u{301}");
    }

    #[test]
    fn the_conversation_nests_threads_in_their_review_and_skips_empty_reviews() {
        let r = review();
        let page = conversation(&pr(), Loaded::Ready(&r), &HashSet::new(), 80, NOW);
        let t = text(&page.rows);
        let at = |needle: &str| t.iter().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{needle}: {t:#?}"));
        assert!(t[0].starts_with("Add panel  #42"));
        assert!(t[1].contains("octocat wants to merge 1 commit into main from feat"));
        assert!(at("octocat opened 1 day ago") < at("Summary"));
        assert!(at("carol commented") < at("bob approved") && at("bob approved") < at("a.rs:2 ↗"), "time order, the thread inside its review");
        assert!(t[at("bob approved")].ends_with("✓ Approved"), "the badge sits at the card's right end");
        assert!(at("-let x = 1;") < at("nit: rename x") && at("nit: rename x") < at("Done."));
        assert!(!t.iter().any(|l| l.contains("dan")), "an empty comment-only review is left out");
        // the resolved thread is a lone card, folded
        let folded = at("· resolved");
        assert!(t[folded].contains("▸ a.rs:2") && t[folded].contains("2 comments"));
        // the side panel: a line under the header at 80 columns, beside the timeline at 120
        assert!(t[at("Reviewers")].starts_with("Reviewers ✓bob ○erin · Checks ✗ 1 failing"), "{}", t[at("Reviewers")]);
        assert!(at("Reviewers") < at("octocat opened"));
        let wide = conversation(&pr(), Loaded::Ready(&r), &HashSet::new(), 120, NOW);
        assert_eq!(wide.side_at, 90);
        assert!(text(&wide.side)[0].trim() == "Reviewers");
        assert!(text(&wide.side).iter().any(|l| l.contains("○ erin (requested)")) && text(&wide.side).iter().any(|l| l.contains("1 checks · 1 failing")));
        // the review card holds the thread card
        let outer = wide.boxes.iter().find(|b| b.tone == Tone::Good).unwrap();
        let inner = wide.boxes.iter().find(|b| b.left > outer.left && b.top > outer.top).unwrap();
        assert!(inner.bottom < outer.bottom && inner.right < outer.right);
        assert_eq!(wide.card_at(inner.top + 1, inner.left + 2).map(|b| b.top), Some(inner.top));
    }

    #[test]
    fn targets_jump_toggle_and_open() {
        let r = review();
        let page = conversation(&pr(), Loaded::Ready(&r), &HashSet::new(), 80, NOW);
        let jump = Target::Line { path: "a.rs".into(), line: 2, left: false };
        let row = page.row_of(&jump).unwrap();
        let col = page.rows[row].indent + page.rows[row].text().find("a.rs").unwrap();
        assert_eq!(page.target_at(row, col), Some(&jump));
        // a folded thread opens when toggled
        let Some(Target::Toggle(key)) = page.rows.iter().find(|r| r.text().contains("resolved")).and_then(|r| r.spans[0].target.clone()) else { panic!() };
        let open = conversation(&pr(), Loaded::Ready(&r), &[key].into(), 80, NOW);
        assert!(text(&open.rows).iter().filter(|l| l.contains("nit: rename x")).count() == 2, "both threads show their comments");
        assert!(text(&conversation(&pr(), Loaded::Loading, &HashSet::new(), 80, NOW).rows).iter().any(|l| l.contains("Loading comments")));
    }

    #[test]
    fn commits_open_to_their_message_and_into_files_changed() {
        let r = review();
        let page = commits(&pr(), Loaded::Ready(&r), &HashSet::new(), None, 80, NOW);
        let t = text(&page.rows);
        let row = t.iter().position(|l| l.contains("▸ Fix it")).unwrap();
        assert!(t[row].contains("octocat committed 1 day ago · abcdef1 ↗"));
        assert!(page.rows[row].spans.iter().any(|s| s.target == Some(Target::Commit(Some("abcdef1234".into())))));
        let open = commits(&pr(), Loaded::Ready(&r), &["commit:abcdef1234".to_string()].into(), Some("abcdef1234"), 80, NOW);
        let t = text(&open.rows);
        assert!(t.iter().any(|l| l.trim() == "Because."));
        assert!(open.row_of(&Target::Commit(None)).is_some(), "Show all commits");
        let row = t.iter().position(|l| l.trim() == "Because.").unwrap();
        assert_eq!(open.text((row - 1, 4), (row, 80)), "Fix it  octocat committed 1 day ago · abcdef1 ↗\nBecause.", "a selection across rows, indents left out");
    }
}

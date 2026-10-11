//! The commit log (gpui-free): a streamed `git log` held in memory, branch-graph lanes assigned as rows arrive,
//! what a selection compares, filters, ref labels, file history and git's commit-graph file.

#![allow(dead_code)] // ponytail: wired into the UI in ticket 03

use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read},
    ops::Range,
    path::Path,
    process::{Child, Stdio},
    sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc, Condvar, Mutex, PoisonError, Weak},
};

use crate::git;

/// Edges spanning this many rows or more are drawn as an arrow stub at each end, as IntelliJ does.
pub const STUB_SPAN: u32 = 30;
const NONE: u32 = u32::MAX;

/// A commit id as bytes; sha1 ids fill the first 20.
type Oid = [u8; 32];

/// The index key: an id's first 8 bytes.
fn prefix(id: &Oid) -> u64 {
    u64::from_be_bytes(id[..8].try_into().unwrap())
}

fn oid(hex: &str) -> Oid {
    let nibble = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
    let mut id = [0; 32];
    for (byte, pair) in id.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = nibble(pair[0]) << 4 | pair.get(1).map_or(0, |&b| nibble(b));
    }
    id
}

/// One commit as `git log` printed it.
#[derive(Debug, Clone, PartialEq)]
pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub email: String,
    /// Committer time, Unix seconds.
    pub time: i64,
    /// The committer's UTC offset, minutes.
    pub offset: i16,
    pub subject: String,
}

/// Which tips the log walks: the branch dropdown.
#[derive(Debug, Clone, PartialEq)]
pub enum Branches {
    /// HEAD, every local branch and every remote branch; tags are labels only.
    All,
    Current,
    /// One branch, by full ref name (`refs/heads/x`, `refs/remotes/origin/x`).
    One(String),
}

impl Branches {
    fn args(&self) -> Vec<&str> {
        match self {
            Branches::All => vec!["HEAD", "--branches", "--remotes"],
            Branches::Current => vec!["HEAD"],
            Branches::One(name) => vec!["--end-of-options", name],
        }
    }
}

/// A running `git log --topo-order`: commits arrive on `batches` in display order, the first ones as soon as
/// git prints them; the channel closes when git ends. A failing git sends its stderr last. Dropping it stops git.
/// The reader stops at the wanted number of commits: git then blocks on its full pipe, costing no CPU and no
/// further memory, until [`Stream::want`] asks for more.
pub struct Stream {
    child: Arc<Mutex<Child>>,
    pub batches: mpsc::Receiver<Result<Vec<Commit>, String>>,
    demand: Arc<Demand>,
    reader: Weak<()>,
}

struct Demand {
    limit: Mutex<u64>,
    more: Condvar,
    paused: AtomicBool,
}

impl Stream {
    /// Raise the number of commits to read; never lowers it.
    pub fn want(&self, commits: u64) {
        let mut limit = self.demand.limit.lock().unwrap_or_else(PoisonError::into_inner);
        if commits > *limit {
            *limit = commits;
            // Cleared here, not by the reader on waking, so a poll right after asking never sees a stale pause.
            self.demand.paused.store(false, Ordering::Release);
            self.demand.more.notify_one();
        }
    }

    /// The reader has delivered every wanted commit and waits for more demand; git has more to give.
    pub fn paused(&self) -> bool {
        self.demand.paused.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn reader_alive(&self) -> Weak<()> {
        self.reader.clone()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap_or_else(PoisonError::into_inner).kill();
        // A paused reader would otherwise wait forever; let it reach git's end and exit.
        self.want(u64::MAX);
    }
}

/// Starts the whole log of `branches` in the repository around `cwd`.
pub fn stream(cwd: &Path, branches: &Branches) -> Result<Stream, String> {
    stream_limited(cwd, branches, u64::MAX)
}

/// Starts the log of `branches`, reading `limit` commits until [`Stream::want`] asks for more.
pub fn stream_limited(cwd: &Path, branches: &Branches, limit: u64) -> Result<Stream, String> {
    let mut args = vec!["log", "-z", "--topo-order", "--parents", "--format=%H %P%x1f%an%x1f%ae%x1f%ct %ci%x1f%s"];
    args.extend(branches.args());
    let mut child = git::command(cwd)?
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("lenz: git log: {e}"))?;
    let mut reader = BufReader::with_capacity(1 << 16, child.stdout.take().expect("piped stdout"));
    let mut stderr = child.stderr.take().expect("piped stderr");
    let child = Arc::new(Mutex::new(child));
    let waited = child.clone();
    let (tx, batches) = mpsc::channel();
    let demand = Arc::new(Demand { limit: Mutex::new(limit), more: Condvar::new(), paused: AtomicBool::new(false) });
    let wanted = demand.clone();
    let alive = Arc::new(());
    let reader_ref = Arc::downgrade(&alive);
    std::thread::spawn(move || {
        let _alive = alive;
        let (mut record, mut batch, mut delivered) = (Vec::new(), Vec::new(), 0u64);
        loop {
            let mut limit = wanted.limit.lock().unwrap_or_else(PoisonError::into_inner);
            if delivered >= *limit {
                // Hand over what has arrived before waiting, so the screen shows it.
                if !batch.is_empty() && tx.send(Ok(std::mem::take(&mut batch))).is_err() {
                    break;
                }
                wanted.paused.store(true, Ordering::Release);
                while delivered >= *limit {
                    limit = wanted.more.wait(limit).unwrap_or_else(PoisonError::into_inner);
                }
            }
            drop(limit);
            record.clear();
            match reader.read_until(0, &mut record) {
                Ok(0) => break,
                Ok(_) => {
                    let parsed = parse_record(record.strip_suffix(&[0]).unwrap_or(&record));
                    delivered += u64::from(parsed.is_some());
                    batch.extend(parsed);
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("lenz: git log: {e}")));
                    break;
                }
            }
            // Send whatever has arrived whenever git's output runs dry, so the first screen shows early.
            if (batch.len() >= 4096 || !batch.is_empty() && reader.buffer().is_empty())
                && tx.send(Ok(std::mem::take(&mut batch))).is_err()
            {
                break; // the Stream was dropped, killing git
            }
        }
        if !batch.is_empty() {
            let _ = tx.send(Ok(batch));
        }
        // Reaped however it ended.
        // ponytail: stderr is read only after stdout ends, as in blame; a thread if git ever floods it.
        let mut err = String::new();
        let _ = stderr.read_to_string(&mut err);
        let status = waited.lock().unwrap_or_else(PoisonError::into_inner).wait();
        if !status.is_ok_and(|s| s.success()) {
            let _ = tx.send(Err(err.trim_end().to_string()));
        }
    });
    Ok(Stream { child, batches, demand, reader: reader_ref })
}

/// `%H %P<US>%an<US>%ae<US>%ct %ci<US>%s`; `%ci` ends with the committer's UTC offset.
fn parse_record(record: &[u8]) -> Option<Commit> {
    let text = String::from_utf8_lossy(record);
    let mut fields = text.splitn(5, '\x1f');
    let mut ids = fields.next()?.split(' ').filter(|s| !s.is_empty()).map(String::from);
    let (id, parents, author, email) = (ids.next()?, ids.collect(), fields.next()?.into(), fields.next()?.into());
    let stamp = fields.next()?;
    let (time, zone) = (stamp.split(' ').next()?, stamp.rsplit(' ').next()?);
    Some(Commit { id, parents, author, email, time: time.parse().ok()?, offset: zone_minutes(zone)?, subject: fields.next()?.into() })
}

/// `+0130` / `-0800` as minutes east of UTC.
fn zone_minutes(zone: &str) -> Option<i16> {
    let (sign, digits) = zone.split_at_checked(1)?;
    let (h, m) = digits.split_at_checked(2)?;
    let minutes = h.parse::<i16>().ok()? * 60 + m.parse::<i16>().ok()?;
    match sign { "+" => Some(minutes), "-" => Some(-minutes), _ => None }
}

/// The log held in memory: rows in topo order (children above parents) with each row's graph column and parent
/// links. Lanes are assigned as rows arrive; edges are worked out per visible range, never snapshotted per row.
#[derive(Default)]
pub struct Log {
    /// Ids back to back, `hex_len / 2` bytes each.
    ids: Vec<u8>,
    hex_len: usize,
    rows: Vec<Row>,
    /// Subjects back to back; a row's runs to the next row's start.
    subjects: String,
    authors: Vec<(String, String)>,
    author_index: HashMap<(String, String), u32>,
    links: Vec<Link>,
    /// Rows by id prefix (a third of the memory of whole ids); a row whose prefix is taken goes in `clashes`.
    index: HashMap<u64, u32>,
    clashes: HashMap<Oid, u32>,
    /// Links whose parent hasn't arrived yet, by parent id: (child row, link).
    waiting: HashMap<Oid, Vec<(u32, u32)>>,
    /// Lanes holding a link that waits for its parent. Every lane waits for one commit.
    busy: Vec<bool>,
    /// Links spanning STUB_SPAN rows or more, in parent-row order: (parent row, child row, link).
    long: Vec<(u32, u32, u32)>,
}

/// A row's time keeps the committer's UTC offset in its low bits, in quarter hours (every real zone is a whole
/// number of them; -16h..+15h45 fits), so the zone costs no memory per row.
const ZONE_BITS: u32 = 7;
const ZONE_BIAS: i64 = 64;

fn pack_time(time: i64, offset: i16) -> i64 {
    let quarters = (i64::from(offset) as f64 / 15.0).round() as i64;
    (time << ZONE_BITS) | (quarters.clamp(-ZONE_BIAS, ZONE_BIAS - 1) + ZONE_BIAS)
}

fn unpack_offset(packed: i64) -> i16 {
    (((packed & ((1 << ZONE_BITS) - 1)) - ZONE_BIAS) * 15) as i16
}

struct Row {
    col: u32,
    /// First of this row's links; they run to the next row's first.
    links: u32,
    subject: u32,
    author: u32,
    time: i64,
}

/// A row's edge to one parent, which travels down `lane` and is resolved when the parent arrives.
struct Link {
    parent: u32,
    lane: u32,
}

/// One edge of the graph, from a child's dot to a parent's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    pub child: u32,
    pub child_col: u32,
    /// The column the edge runs down between its ends: it leaves the child towards it in the first row gap
    /// and joins the parent from it in the last.
    pub lane: u32,
    /// `None` until the parent arrives (or when it is outside the log): the edge runs off the bottom.
    pub parent: Option<u32>,
    pub parent_col: u32,
    /// Spans STUB_SPAN rows or more: draw only an arrow at each end.
    pub stub: bool,
}

impl Log {
    /// Appends rows in topo order and lays them out.
    pub fn extend(&mut self, commits: impl IntoIterator<Item = Commit>) {
        for commit in commits {
            self.push(commit);
        }
    }

    fn push(&mut self, c: Commit) {
        let r = self.rows.len() as u32;
        let id = oid(&c.id);
        self.hex_len = c.id.len();
        // Every lane waiting for this commit ends here; it sits in the leftmost of them.
        let arrived = self.waiting.remove(&id).unwrap_or_default();
        for &(child, link) in &arrived {
            let link_ = &mut self.links[link as usize];
            link_.parent = r;
            self.busy[link_.lane as usize] = false;
            if r - child >= STUB_SPAN {
                self.long.push((r, child, link));
            }
        }
        let col = arrived.iter().map(|&(_, l)| self.links[l as usize].lane).min().unwrap_or_else(|| self.free_lane());
        let author = self.author_id(c.author, c.email);
        self.rows.push(Row { col, links: self.links.len() as u32, subject: self.subjects.len() as u32, author, time: pack_time(c.time, c.offset) });
        // The first parent continues straight down; another parent joins a lane already waiting for it, or opens one.
        for (i, parent) in c.parents.iter().enumerate() {
            let parent = oid(parent);
            let lane = match self.waiting.get(&parent) {
                _ if i == 0 => col,
                Some(waiting) => self.links[waiting[0].1 as usize].lane,
                None => self.free_lane(),
            };
            self.busy[lane as usize] = true;
            self.waiting.entry(parent).or_default().push((r, self.links.len() as u32));
            self.links.push(Link { parent: NONE, lane });
        }
        self.subjects.push_str(&c.subject);
        if *self.index.entry(prefix(&id)).or_insert(r) != r {
            self.clashes.insert(id, r);
        }
        self.ids.extend_from_slice(&id[..self.hex_len.div_ceil(2)]);
    }

    fn free_lane(&mut self) -> u32 {
        let lane = self.busy.iter().position(|b| !b).unwrap_or(self.busy.len());
        if lane == self.busy.len() {
            self.busy.push(false);
        }
        lane as u32
    }

    fn author_id(&mut self, name: String, email: String) -> u32 {
        let key = (name, email);
        if let Some(&id) = self.author_index.get(&key) {
            return id;
        }
        self.authors.push(key.clone());
        self.author_index.insert(key, self.authors.len() as u32 - 1);
        self.authors.len() as u32 - 1
    }

    pub fn len(&self) -> u32 {
        self.rows.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The graph's width in columns so far.
    pub fn width(&self) -> u32 {
        self.busy.len() as u32
    }

    pub fn id(&self, row: u32) -> String {
        let width = self.hex_len.div_ceil(2);
        let bytes = &self.ids[row as usize * width..][..width];
        let digits = bytes.iter().flat_map(|b| [b >> 4, b & 15]).take(self.hex_len);
        digits.map(|n| char::from_digit(n.into(), 16).unwrap()).collect()
    }

    /// The row of a full commit id.
    pub fn row_of(&self, id: &str) -> Option<u32> {
        if id.len() != self.hex_len {
            return None;
        }
        let full = oid(id);
        let row = *self.index.get(&prefix(&full))?;
        if self.id(row) == id.to_ascii_lowercase() { Some(row) } else { self.clashes.get(&full).copied() }
    }

    pub fn subject(&self, row: u32) -> &str {
        let end = self.rows.get(row as usize + 1).map_or(self.subjects.len(), |r| r.subject as usize);
        &self.subjects[self.rows[row as usize].subject as usize..end]
    }

    /// Name and email.
    pub fn author(&self, row: u32) -> (&str, &str) {
        let (name, email) = &self.authors[self.rows[row as usize].author as usize];
        (name, email)
    }

    pub fn time(&self, row: u32) -> i64 {
        self.rows[row as usize].time >> ZONE_BITS
    }

    /// Commit date and time in the committer's own time zone, as `git log` shows it: `2026-10-08 14:32`.
    pub fn stamp(&self, row: u32) -> String {
        let packed = self.rows[row as usize].time;
        crate::blame::date_time(packed >> ZONE_BITS, unpack_offset(packed))
    }

    pub fn column(&self, row: u32) -> u32 {
        self.rows[row as usize].col
    }

    /// Each parent's row, in parent order; `None` while it hasn't arrived.
    pub fn parents(&self, row: u32) -> Vec<Option<u32>> {
        self.link_range(row).map(|l| Some(self.links[l].parent).filter(|&p| p != NONE)).collect()
    }

    fn link_range(&self, row: u32) -> Range<usize> {
        let end = self.rows.get(row as usize + 1).map_or(self.links.len(), |r| r.links as usize);
        self.rows[row as usize].links as usize..end
    }

    /// The edges to paint for `rows`: those passing through them, and stubs with an end inside them.
    pub fn edges(&self, rows: Range<u32>) -> Vec<Edge> {
        let n = self.len();
        let (top, bottom) = (rows.start.min(n), rows.end.min(n));
        let visible = |r: u32| (top..bottom).contains(&r);
        let edge = |child: u32, link: &Link, stub: bool| {
            let parent = Some(link.parent).filter(|&p| p != NONE);
            let parent_col = parent.map_or(link.lane, |p| self.column(p));
            Edge { child, child_col: self.column(child), lane: link.lane, parent, parent_col, stub }
        };
        // An edge reaching `top` that isn't a stub starts fewer than STUB_SPAN rows above it.
        let first = top.saturating_sub(STUB_SPAN - 1);
        let mut out = Vec::new();
        for child in first..bottom {
            for link in &self.links[self.link_range(child)] {
                let end = if link.parent == NONE { n } else { link.parent };
                let stub = end - child >= STUB_SPAN;
                if end >= top && (!stub || visible(child) || visible(end)) {
                    out.push(edge(child, link, stub));
                }
            }
        }
        // Stubs ending in view whose child is further up.
        let from = self.long.partition_point(|&(parent, ..)| parent < top);
        for &(_, child, link) in self.long[from..].iter().take_while(|&&(parent, ..)| parent < bottom) {
            if child < first {
                out.push(edge(child, &self.links[link as usize], true));
            }
        }
        out
    }

    /// The `revs` of a `changeset::Spec::Git` showing `selection`; `empty_tree` is [`empty_tree`]'s answer.
    pub fn revs(&self, selection: Selection, empty_tree: &str) -> Vec<String> {
        // Against a parent; a root commit against the empty tree.
        let base = |row: u32, parent: usize| match self.link_range(row).len() {
            0 => empty_tree.to_string(),
            n => format!("{}^{}", self.id(row), parent.min(n - 1) + 1),
        };
        let (a, b, range) = match selection {
            Selection::One { pick: Pick::Uncommitted, .. } => {
                return vec![if self.is_empty() { empty_tree } else { "HEAD" }.to_string()];
            }
            Selection::One { pick: Pick::Commit(row), parent } => return vec![base(row, parent), self.id(row)],
            Selection::Two(a, b) => (a, b, false),
            Selection::Range(a, b) => (a, b, true),
        };
        // Uncommitted sits above every commit, and a lower row is older.
        let height = |p: Pick| match p {
            Pick::Uncommitted => -1,
            Pick::Commit(row) => i64::from(row),
        };
        let (newer, older) = if height(a) <= height(b) { (a, b) } else { (b, a) };
        let Pick::Commit(older) = older else {
            return self.revs(Selection::One { pick: Pick::Uncommitted, parent: 0 }, empty_tree);
        };
        let left = if range { base(older, 0) } else { self.id(older) };
        match newer {
            Pick::Uncommitted => vec![left], // ⟷ the working tree
            Pick::Commit(newer) => vec![left, self.id(newer)],
        }
    }
}

/// A row of the log as the user picks it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pick {
    /// The "Uncommitted changes" row above HEAD: the working tree.
    Uncommitted,
    Commit(u32),
}

/// What is selected in the log.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Selection {
    /// One row: a commit against its parent (`parent` 0 is the first; "vs parent 2" is 1), or HEAD ⟷ the
    /// working tree.
    One { pick: Pick, parent: usize },
    /// Two rows (ctrl-click), in either order: older ⟷ newer.
    Two(Pick, Pick),
    /// The ends of a run of rows (shift-click), in either order: the oldest's first parent ⟷ the newest.
    Range(Pick, Pick),
}

/// The empty tree's id in this repository's hash (sha1 or sha256): what a root commit compares with.
pub fn empty_tree(cwd: &Path) -> Result<String, String> {
    let out = git::run_with_input(cwd, &["hash-object", "-t", "tree", "--stdin"], Vec::new())?;
    Ok(String::from_utf8_lossy(&out).trim_end().to_string())
}

/// The filter box, parsed: plain text plus `author:`, `path:`, `after:` and `before:` tokens.
/// ponytail: values end at whitespace (no quoting) and dates are `YYYY-MM-DD` in UTC; add quoting and local time
/// when someone needs them.
#[derive(Debug, Default, PartialEq)]
pub struct Filter {
    /// The plain words, lowercased: matched against subject, author and hash prefix; Enter greps full messages.
    pub text: String,
    /// Lowercased; matched against name and email.
    pub author: Option<String>,
    /// Not matched in memory: run [`path_commits`].
    pub path: Option<String>,
    /// Commit time bounds in Unix seconds: `after` inclusive (that day's start), `before` exclusive (the end of
    /// that day), so `after:2024-01-01 before:2024-01-31` is January.
    pub after: Option<i64>,
    pub before: Option<i64>,
}

impl Filter {
    pub fn parse(input: &str) -> Result<Filter, String> {
        let mut filter = Filter::default();
        let mut words = Vec::new();
        for token in input.split_whitespace() {
            match token.split_once(':') {
                Some(("author" | "path" | "after" | "before", "")) => {} // still being typed
                Some(("author", v)) => filter.author = Some(v.to_lowercase()),
                Some(("path", v)) => filter.path = Some(v.to_string()),
                Some(("after", v)) => filter.after = Some(day_start(v)?),
                Some(("before", v)) => filter.before = Some(day_start(v)? + 86_400),
                _ => words.push(token),
            }
        }
        filter.text = words.join(" ").to_lowercase();
        Ok(filter)
    }

    /// Does `row` pass the in-memory part of the filter (all but `path`)?
    /// ponytail: lowercases each field per call, ~50 ms a pass over a million rows; cache lowercased text if slow.
    pub fn matches(&self, log: &Log, row: u32) -> bool {
        let time = log.time(row);
        if self.after.is_some_and(|t| time < t) || self.before.is_some_and(|t| time >= t) {
            return false;
        }
        let has = |hay: &str, needle: &str| hay.to_lowercase().contains(needle);
        let (name, email) = log.author(row);
        if self.author.as_ref().is_some_and(|a| !has(name, a) && !has(email, a)) {
            return false;
        }
        let text = &self.text;
        text.is_empty()
            || has(log.subject(row), text)
            || has(name, text)
            || text.bytes().all(|b| b.is_ascii_hexdigit()) && log.id(row).starts_with(text.as_str())
    }
}

/// `YYYY-MM-DD` → that day's first second, UTC.
fn day_start(date: &str) -> Result<i64, String> {
    let bad = || format!("lenz: expected a date as YYYY-MM-DD, not {date:?}");
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (Some(Some(y)), Some(Some(m @ 1..=12)), Some(Some(d @ 1..=31)), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    // Days from 1970-01-01 (Howard Hinnant's days_from_civil).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Ok((era * 146_097 + doe - 719_468) * 86_400)
}

fn ids(cwd: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    Ok(String::from_utf8_lossy(&git::run(cwd, args)?).lines().map(String::from).collect())
}

/// Ids of the commits on `branches` that touch `path` (git's path-limited log), newest first.
pub fn path_commits(cwd: &Path, branches: &Branches, path: &str) -> Result<Vec<String>, String> {
    let mut args = vec!["log", "--format=%H"];
    args.extend(branches.args());
    args.extend(["--", path]);
    ids(cwd, &args)
}

/// Ids of the commits on `branches` whose full message contains `text`, ignoring case, newest first.
/// ponytail: waits for the whole walk (~9 s on Linux); stream like [`stream`] if first matches should show early.
pub fn grep_commits(cwd: &Path, branches: &Branches, text: &str) -> Result<Vec<String>, String> {
    let grep = format!("--grep={text}");
    let mut args = vec!["log", "--format=%H", "-i", "--fixed-strings", &grep];
    args.extend(branches.args());
    ids(cwd, &args)
}

/// The commits on `branches` that changed `path`, following renames, newest first, each with the file's path in
/// that commit.
pub fn file_history(cwd: &Path, branches: &Branches, path: &str) -> Result<Vec<(String, String)>, String> {
    let mut args = vec!["log", "-z", "--follow", "--name-only", "--format=%H"];
    args.extend(branches.args());
    args.extend(["--", path]);
    let out = git::run(cwd, &args)?;
    // `<id>\0\n<path>\0` per commit.
    let mut list: Vec<(String, String)> = Vec::new();
    for field in out.split(|&b| b == 0).filter(|f| !f.is_empty()) {
        let field = String::from_utf8_lossy(field);
        match (field.strip_prefix('\n'), list.last_mut()) {
            (Some(path), Some(last)) => last.1 = path.to_string(),
            (Some(_), None) => {}
            (None, _) => list.push((field.into_owned(), String::new())),
        }
    }
    Ok(list)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RefKind {
    Head,
    Branch,
    Remote,
    Tag,
}

/// A ref label on a commit.
#[derive(Debug, Clone, PartialEq)]
pub struct Ref {
    pub kind: RefKind,
    /// Short name: `HEAD`, `main`, `origin/main`, `v1.0`.
    pub name: String,
    /// The branch HEAD is on.
    pub current: bool,
}

/// Ref labels by commit id.
pub struct Refs(HashMap<String, Vec<Ref>>);

impl Refs {
    /// The labels on a commit: HEAD, then branches, remotes and tags, each by name.
    pub fn at(&self, id: &str) -> &[Ref] {
        self.0.get(id).map_or(&[], Vec::as_slice)
    }
    /// The commit HEAD is on.
    pub fn head(&self) -> Option<&str> {
        self.0.iter().find(|(_, refs)| refs.iter().any(|r| r.kind == RefKind::Head)).map(|(id, _)| id.as_str())
    }
    #[cfg(test)]
    pub(crate) fn from_map(map: HashMap<String, Vec<Ref>>) -> Self { Self(map) }
}

/// HEAD, local branches, remote branches and tags (annotated ones peeled to their commit).
pub fn refs(cwd: &Path) -> Result<Refs, String> {
    let mut map: HashMap<String, Vec<Ref>> = HashMap::new();
    if let Ok(head) = git::run(cwd, &["rev-parse", "-q", "--verify", "HEAD"]) {
        let head = String::from_utf8_lossy(&head).trim_end().to_string();
        map.entry(head).or_default().push(Ref { kind: RefKind::Head, name: "HEAD".into(), current: false });
    }
    let format = "--format=%(objectname)%00%(*objectname)%00%(symref)%00%(HEAD)%00%(refname)";
    let out = git::run(cwd, &["for-each-ref", format, "refs/heads", "refs/remotes", "refs/tags"])?;
    for line in String::from_utf8_lossy(&out).lines() {
        let [id, peeled, symref, head, name] = line.split('\0').collect::<Vec<_>>()[..] else { continue };
        let kind = [("refs/heads/", RefKind::Branch), ("refs/remotes/", RefKind::Remote), ("refs/tags/", RefKind::Tag)]
            .into_iter()
            .find_map(|(prefix, kind)| name.strip_prefix(prefix).map(|short| (kind, short)));
        // `origin/HEAD` and other symbolic refs point at a branch that is labelled already.
        let (Some((kind, short)), "") = (kind, symref) else { continue };
        let target = if peeled.is_empty() { id } else { peeled };
        map.entry(target.to_string()).or_default().push(Ref { kind, name: short.into(), current: head == "*" });
    }
    Ok(Refs(map))
}

/// Does the repository have git's commit-graph file (single or split chain)? Without it the log's first rows
/// wait for the whole walk.
pub fn has_commit_graph(cwd: &Path) -> bool {
    let Ok(out) = git::run(cwd, &["rev-parse", "--git-path", "objects/info"]) else { return false };
    let info = cwd.join(String::from_utf8_lossy(&out).trim_end());
    info.join("commit-graph").is_file() || info.join("commit-graphs/commit-graph-chain").is_file()
}

/// Writes git's commit-graph file: Lenz's one write to the repository, run only when the user asks.
pub fn write_commit_graph(cwd: &Path) -> Result<(), String> {
    git::run(cwd, &["commit-graph", "write", "--reachable"]).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::{commit_all, git, repo, write};

    fn h(n: u32) -> String {
        format!("{n:040x}")
    }

    /// Commit `n` with parents, as a hand-built DAG node.
    fn c(n: u32, parents: &[u32]) -> Commit {
        let parents = parents.iter().map(|&p| h(p)).collect();
        Commit { id: h(n), parents, author: "Ann".into(), email: "ann@x".into(), time: 0, offset: 0, subject: format!("c{n}") }
    }

    fn log(commits: Vec<Commit>) -> Log {
        let mut log = Log::default();
        log.extend(commits);
        log
    }

    fn columns(log: &Log) -> Vec<u32> {
        (0..log.len()).map(|r| log.column(r)).collect()
    }

    /// (child, child_col, lane, parent, parent_col, stub), sorted.
    type E = (u32, u32, u32, Option<u32>, u32, bool);
    fn edges(log: &Log, rows: Range<u32>) -> Vec<E> {
        let mut out: Vec<E> =
            log.edges(rows).iter().map(|e| (e.child, e.child_col, e.lane, e.parent, e.parent_col, e.stub)).collect();
        out.sort_by_key(|e| (e.0, e.2));
        out
    }

    #[test]
    fn the_time_zone_costs_no_memory_per_row() {
        assert_eq!(std::mem::size_of::<Row>(), 24);
        let mut log = Log::default();
        for (time, offset) in [(1_791_489_600, 345), (-60, -480), (0, 0), (4_102_444_800, 840), (1, -720)] {
            log.push(Commit { id: format!("{:040x}", log.len() + 1), parents: vec![], author: "A".into(), email: "a@x".into(), time, offset, subject: "s".into() });
            let row = log.len() - 1;
            assert_eq!((log.time(row), log.stamp(row)), (time, crate::blame::date_time(time, offset)), "{time} {offset}");
        }
    }

    #[test]
    fn records_carry_the_committer_time_zone() {
        let c = parse_record(b"abc def\x1fAnn\x1fann@x\x1f1791489600 2026-10-09 01:45:00 +0545\x1fSubject").unwrap();
        assert_eq!((c.time, c.offset, c.subject.as_str()), (1_791_489_600, 345, "Subject"));
        assert_eq!(parse_record(b"abc\x1fAnn\x1fann@x\x1f1 1969-12-31 16:00:01 -0800\x1fS").unwrap().offset, -480);
        assert!(parse_record(b"abc\x1fAnn\x1fann@x\x1f1\x1fS").is_none(), "a record without a zone is malformed");
        let mut log = Log::default();
        log.push(c);
        assert_eq!(log.stamp(0), "2026-10-09 01:45");
    }

    #[test]
    fn a_linear_history_is_one_lane_and_a_freed_lane_is_reused() {
        // two unrelated histories one after the other
        let log = log(vec![c(0, &[1]), c(1, &[2]), c(2, &[]), c(3, &[4]), c(4, &[])]);
        assert_eq!(columns(&log), [0, 0, 0, 0, 0]);
        assert_eq!(log.width(), 1);
        assert_eq!(
            edges(&log, 0..5),
            [(0, 0, 0, Some(1), 0, false), (1, 0, 0, Some(2), 0, false), (3, 0, 0, Some(4), 0, false)]
        );
        assert_eq!(log.parents(0), [Some(1)]);
        assert_eq!(log.parents(2), []);
        // these ids share their first 8 bytes
        assert_eq!(log.row_of(&h(3)), Some(3));
        assert_eq!(log.row_of(&h(9)), None);
    }

    #[test]
    fn a_branch_and_merge_open_a_lane_that_converges_on_the_fork_point() {
        let log = log(vec![c(0, &[1, 2]), c(1, &[3]), c(2, &[3]), c(3, &[])]);
        assert_eq!(columns(&log), [0, 0, 1, 0]);
        assert_eq!(
            edges(&log, 0..4),
            [
                (0, 0, 0, Some(1), 0, false),
                (0, 0, 1, Some(2), 1, false),
                (1, 0, 0, Some(3), 0, false),
                (2, 1, 1, Some(3), 0, false),
            ]
        );
        // a window in the middle gets the edges passing through or ending in it
        assert_eq!(
            edges(&log, 1..2),
            [(0, 0, 0, Some(1), 0, false), (0, 0, 1, Some(2), 1, false), (1, 0, 0, Some(3), 0, false)]
        );
    }

    #[test]
    fn an_octopus_merge_opens_a_lane_per_extra_parent() {
        let log = log(vec![c(0, &[1, 2, 3]), c(1, &[4]), c(2, &[4]), c(3, &[4]), c(4, &[])]);
        assert_eq!(columns(&log), [0, 0, 1, 2, 0]);
        assert_eq!(log.width(), 3);
        let lanes: Vec<u32> = edges(&log, 0..1).iter().map(|e| e.2).collect();
        assert_eq!(lanes, [0, 1, 2]);
        assert_eq!(
            edges(&log, 4..5),
            [(1, 0, 0, Some(4), 0, false), (2, 1, 1, Some(4), 0, false), (3, 2, 2, Some(4), 0, false)]
        );
    }

    #[test]
    fn a_second_parent_joins_the_lane_already_waiting_for_it() {
        // tips 0 and 1; 1 merges 2, which 0's lane is already heading for
        let log = log(vec![c(0, &[2]), c(1, &[3, 2]), c(2, &[3]), c(3, &[])]);
        assert_eq!(columns(&log), [0, 1, 0, 0]);
        assert_eq!(log.width(), 2);
        assert!(edges(&log, 0..4).contains(&(1, 1, 0, Some(2), 0, false)));
        assert!(edges(&log, 0..4).contains(&(1, 1, 1, Some(3), 0, false)));
    }

    /// Row 0 is a tip whose parent is row 41; rows 1..=40 are a chain in lane 1 above it.
    fn long_edge() -> Vec<Commit> {
        let mut commits = vec![c(0, &[41])];
        commits.extend((1..=40).map(|i| c(i, &[i + 1])));
        commits.push(c(41, &[]));
        commits
    }

    #[test]
    fn edges_of_thirty_rows_or_more_are_stubs_shown_only_near_their_ends() {
        let log = log(long_edge());
        assert_eq!(log.column(0), 0);
        assert_eq!(log.column(20), 1);
        assert_eq!(log.column(41), 0);
        let stub = (0, 0, 0, Some(41), 0, true);
        assert!(edges(&log, 0..5).contains(&stub), "child end in view");
        assert!(edges(&log, 38..45).contains(&stub), "parent end in view");
        assert!(edges(&log, 41..42).contains(&stub), "parent end alone in view");
        assert!(!edges(&log, 10..20).iter().any(|e| e.0 == 0), "neither end in view");
        // the chain's short edges passing through 10..20
        let middle = edges(&log, 10..20);
        assert_eq!(middle.len(), 11);
        assert!(middle.iter().all(|e| !e.5 && (9..20).contains(&e.0)));
        // the chain's last edge spans one row and is no stub
        assert!(edges(&log, 41..42).contains(&(40, 1, 1, Some(41), 0, false)));
    }

    #[test]
    fn rows_streamed_in_batches_lay_out_as_if_loaded_at_once() {
        let whole = log(long_edge());
        let mut streamed = Log::default();
        let mut commits = long_edge().into_iter();
        streamed.extend(commits.by_ref().take(20));
        // the long edge's parent hasn't arrived: it runs off the bottom, not yet a stub
        assert!(edges(&streamed, 0..20).contains(&(0, 0, 0, None, 0, false)));
        streamed.extend(commits.by_ref().take(15));
        assert!(edges(&streamed, 0..5).contains(&(0, 0, 0, None, 0, true)));
        assert!(!edges(&streamed, 10..20).iter().any(|e| e.0 == 0));
        streamed.extend(commits);
        assert_eq!(columns(&streamed), columns(&whole));
        assert_eq!(edges(&streamed, 0..42), edges(&whole, 0..42));
    }

    #[test]
    fn a_selection_becomes_the_revisions_it_compares() {
        // 0 merges 1 and 2; 3 is the root
        let log = log(vec![c(0, &[1, 2]), c(1, &[3]), c(2, &[3]), c(3, &[])]);
        let empty = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
        let revs = |s: Selection| log.revs(s, empty);
        let one = |row, parent| revs(Selection::One { pick: Pick::Commit(row), parent });
        let (c0, c1, c2, c3) = (h(0), h(1), h(2), h(3));
        assert_eq!(one(1, 0), [format!("{c1}^1"), c1.clone()]);
        assert_eq!(one(3, 0), [empty.to_string(), c3.clone()], "a root commit against the empty tree");
        assert_eq!(one(0, 0), [format!("{c0}^1"), c0.clone()], "a merge against parent 1");
        assert_eq!(one(0, 1), [format!("{c0}^2"), c0.clone()], "… or parent 2");
        assert_eq!(revs(Selection::One { pick: Pick::Uncommitted, parent: 0 }), ["HEAD"]);
        // two commits: older on the left, whichever was picked first
        assert_eq!(revs(Selection::Two(Pick::Commit(1), Pick::Commit(3))), [c3.clone(), c1.clone()]);
        assert_eq!(revs(Selection::Two(Pick::Commit(3), Pick::Commit(1))), [c3.clone(), c1.clone()]);
        assert_eq!(revs(Selection::Two(Pick::Commit(2), Pick::Uncommitted)), std::slice::from_ref(&c2), "against the working tree");
        // a range: the oldest's first parent ⟷ the newest
        assert_eq!(revs(Selection::Range(Pick::Commit(2), Pick::Commit(0))), [format!("{c2}^1"), c0.clone()]);
        assert_eq!(revs(Selection::Range(Pick::Commit(0), Pick::Commit(3))), [empty.to_string(), c0.clone()]);
        assert_eq!(revs(Selection::Range(Pick::Uncommitted, Pick::Commit(1))), [format!("{c1}^1")]);
        assert_eq!(revs(Selection::Range(Pick::Uncommitted, Pick::Uncommitted)), ["HEAD"]);
    }

    #[test]
    fn filter_tokens_parse_into_fields_and_the_rest_is_text() {
        let f = Filter::parse("Fix  Crash author:Ann path:src/a.rs after:2024-01-01 before:2024-01-31").unwrap();
        assert_eq!(
            f,
            Filter {
                text: "fix crash".into(),
                author: Some("ann".into()),
                path: Some("src/a.rs".into()),
                after: Some(1_704_067_200),
                before: Some(1_706_745_600), // 2024-02-01
            }
        );
        assert_eq!(Filter::parse("author: re:open").unwrap(), Filter { text: "re:open".into(), ..Default::default() });
        assert_eq!(Filter::parse("").unwrap(), Filter::default());
        assert_eq!(Filter::parse("after:1970-01-01").unwrap().after, Some(0));
        assert_eq!(Filter::parse("before:2000-03-01").unwrap().before, Some(951_955_200));
        assert!(Filter::parse("after:2024-13-01").is_err());
        assert!(Filter::parse("before:yesterday").is_err());
    }

    #[test]
    fn the_filter_matches_subject_author_hash_and_dates_in_memory() {
        let mut log = Log::default();
        let commit = |id: &str, author: &str, time, subject: &str| Commit {
            id: id.into(),
            parents: vec![],
            author: author.into(),
            email: format!("{}@x", author.to_lowercase()),
            time,
            offset: 0,
            subject: subject.into(),
        };
        log.extend([
            commit(&format!("abc{:037}", 0), "Ann", 1_704_067_200, "Fix the crash"),
            commit(&format!("def{:037}", 0), "Bob", 1_706_745_600, "Add a feature"),
        ]);
        let rows = |q: &str| {
            let f = Filter::parse(q).unwrap();
            (0..log.len()).filter(|&r| f.matches(&log, r)).collect::<Vec<_>>()
        };
        assert_eq!(rows(""), [0, 1]);
        assert_eq!(rows("CRASH"), [0]);
        assert_eq!(rows("bob"), [1], "author name");
        assert_eq!(rows("def"), [1], "hash prefix");
        assert_eq!(rows("author:ann@"), [0], "author email");
        assert_eq!(rows("author:ann feature"), Vec::<u32>::new());
        assert_eq!(rows("after:2024-01-02"), [1]);
        assert_eq!(rows("before:2024-01-31"), [0]);
        assert_eq!(rows("after:2024-02-01 before:2024-02-01"), [1]);
    }

    fn collect(stream: Stream) -> Result<Log, String> {
        let mut log = Log::default();
        for batch in stream.batches.iter() {
            log.extend(batch?);
        }
        Ok(log)
    }

    /// main: one, three, merge of side (two), with `v1` an annotated tag on one.
    fn branchy_repo() -> tempfile::TempDir {
        let dir = repo();
        let d = dir.path();
        write(d, "a", "1\n");
        commit_all(d, "one");
        git(d, &["tag", "-a", "v1", "-m", "first"]);
        git(d, &["checkout", "-q", "-b", "side"]);
        write(d, "b", "2\n");
        commit_all(d, "two");
        git(d, &["checkout", "-q", "main"]);
        write(d, "c", "3\n");
        commit_all(d, "three");
        git(d, &["merge", "-q", "--no-edit", "side"]);
        dir
    }

    #[test]
    fn the_log_streams_every_branch_in_topo_order_with_its_graph() {
        let dir = branchy_repo();
        let log = collect(stream(dir.path(), &Branches::All).unwrap()).unwrap();
        let subjects: Vec<&str> = (0..log.len()).map(|r| log.subject(r)).collect();
        assert_eq!(subjects.len(), 4);
        assert_eq!(subjects[0], "Merge branch 'side'");
        assert_eq!(subjects[3], "one");
        assert_eq!(log.parents(0).len(), 2);
        assert_eq!(log.parents(3), []);
        assert_eq!(log.width(), 2);
        assert_eq!(log.author(0), ("t", "t@t"));
        assert!(log.time(0) > 0);
        let head = git(dir.path(), &["rev-parse", "HEAD"]);
        assert_eq!(log.row_of(head.trim()), Some(0));
        assert_eq!(log.id(0), head.trim());

        let side = collect(stream(dir.path(), &Branches::One("refs/heads/side".into())).unwrap()).unwrap();
        assert_eq!((0..side.len()).map(|r| side.subject(r)).collect::<Vec<_>>(), ["two", "one"]);
        assert!(collect(stream(dir.path(), &Branches::One("nope".into())).unwrap()).is_err());
    }

    /// Receive until `n` commits arrived, failing if more come or the stream ends first.
    fn take(stream: &Stream, n: usize) -> usize {
        let mut got = 0;
        while got < n {
            got += stream.batches.recv_timeout(std::time::Duration::from_secs(10)).expect("commits").unwrap().len();
        }
        assert_eq!(got, n, "the stream sent more than it was asked for");
        got
    }

    #[test]
    fn the_log_pauses_at_the_wanted_count_and_resumes_on_demand() {
        let dir = repo();
        for i in 0..20 {
            write(dir.path(), "f", format!("{i}\n"));
            commit_all(dir.path(), &format!("c{i}"));
        }
        let stream = stream_limited(dir.path(), &Branches::All, 5).unwrap();
        take(&stream, 5);
        let started = std::time::Instant::now();
        while !stream.paused() {
            assert!(started.elapsed().as_secs() < 10, "the reader must pause once the wanted commits arrived");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(stream.batches.recv_timeout(std::time::Duration::from_millis(100)).is_err(), "nothing beyond the limit");
        stream.want(12);
        take(&stream, 7);
        stream.want(u64::MAX);
        take(&stream, 8);
        assert!(stream.batches.recv_timeout(std::time::Duration::from_secs(10)).is_err(), "the stream ends after the last commit");
    }

    #[test]
    fn dropping_a_paused_log_releases_its_reader() {
        let dir = repo();
        for i in 0..5 {
            write(dir.path(), "f", format!("{i}\n"));
            commit_all(dir.path(), &format!("c{i}"));
        }
        let stream = stream_limited(dir.path(), &Branches::All, 1).unwrap();
        take(&stream, 1);
        let reader = stream.reader_alive();
        drop(stream);
        let started = std::time::Instant::now();
        while reader.upgrade().is_some() {
            assert!(started.elapsed().as_secs() < 10, "a dropped paused stream must not leave its reader blocked");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn refs_label_their_commits() {
        let dir = branchy_repo();
        let d = dir.path();
        let id = |rev: &str| git(d, &["rev-parse", rev]).trim().to_string();
        git(d, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(d, &["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"]);
        let refs = refs(d).unwrap();
        let labels = |rev: &str| refs.at(&id(rev)).iter().map(|r| (r.kind, r.name.clone(), r.current)).collect::<Vec<_>>();
        assert_eq!(
            labels("HEAD"),
            [
                (RefKind::Head, "HEAD".into(), false),
                (RefKind::Branch, "main".into(), true),
                (RefKind::Remote, "origin/main".into(), false)
            ]
        );
        assert_eq!(labels("side"), [(RefKind::Branch, "side".into(), false)]);
        assert_eq!(labels("v1^{commit}"), [(RefKind::Tag, "v1".into(), false)], "annotated tag peeled");
        assert!(refs.at("0000").is_empty());
    }

    #[test]
    fn revisions_from_a_selection_load_as_a_change_set() {
        use crate::changeset::{ChangeSet, Spec, Status};
        let dir = branchy_repo();
        let d = dir.path();
        let log = collect(stream(d, &Branches::All).unwrap()).unwrap();
        let empty = empty_tree(d).unwrap();
        assert_eq!(empty, "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        let files = |selection| {
            let spec = Spec::Git { cached: false, revs: log.revs(selection, &empty), paths: vec![], untracked: false };
            let set = ChangeSet::load(&spec, d).unwrap();
            set.comparisons.into_iter().map(|c| (c.path, c.status)).collect::<Vec<_>>()
        };
        assert_eq!(files(Selection::One { pick: Pick::Commit(3), parent: 0 }), [("a".into(), Status::Added)]);
        assert_eq!(files(Selection::One { pick: Pick::Commit(0), parent: 0 }), [("b".into(), Status::Added)]);
        assert_eq!(files(Selection::One { pick: Pick::Commit(0), parent: 1 }), [("c".into(), Status::Added)]);
        write(d, "a", "changed\n");
        assert_eq!(files(Selection::One { pick: Pick::Uncommitted, parent: 0 }), [("a".into(), Status::Modified)]);
        assert_eq!(files(Selection::Range(Pick::Commit(3), Pick::Uncommitted)).len(), 3);
    }

    #[test]
    fn path_and_message_searches_return_matching_commits() {
        let dir = repo();
        let d = dir.path();
        write(d, "src/a.rs", "1\n");
        commit_all(d, "add a\n\nBody mentions Zebra");
        write(d, "b", "1\n");
        commit_all(d, "add b");
        write(d, "src/a.rs", "2\n");
        commit_all(d, "edit a");
        let id = |rev: &str| git(d, &["rev-parse", rev]).trim().to_string();
        assert_eq!(path_commits(d, &Branches::All, "src/a.rs").unwrap(), [id("HEAD"), id("HEAD~2")]);
        assert_eq!(path_commits(d, &Branches::All, "src").unwrap().len(), 2);
        assert_eq!(grep_commits(d, &Branches::All, "zebra").unwrap(), [id("HEAD~2")]);
        assert_eq!(grep_commits(d, &Branches::All, "a.*").unwrap(), Vec::<String>::new(), "fixed strings");
    }

    #[test]
    fn file_history_follows_a_rename() {
        let dir = repo();
        let d = dir.path();
        write(d, "old.txt", "one\ntwo\nthree\nfour\nfive\n");
        commit_all(d, "add");
        write(d, "other", "x\n");
        commit_all(d, "unrelated");
        git(d, &["mv", "old.txt", "new.txt"]);
        commit_all(d, "rename");
        write(d, "new.txt", "one\ntwo\nthree\nfour\nfive\nsix\n");
        commit_all(d, "edit");
        let id = |rev: &str| git(d, &["rev-parse", rev]).trim().to_string();
        assert_eq!(
            file_history(d, &Branches::Current, "new.txt").unwrap(),
            [(id("HEAD"), "new.txt".into()), (id("HEAD~1"), "new.txt".into()), (id("HEAD~3"), "old.txt".into())]
        );
    }

    #[test]
    fn the_commit_graph_file_is_detected_and_written_on_request() {
        let dir = repo();
        write(dir.path(), "a", "1\n");
        commit_all(dir.path(), "one");
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        assert!(!has_commit_graph(&sub));
        write_commit_graph(dir.path()).unwrap();
        assert!(has_commit_graph(&sub), "found from a subdirectory too");
    }

    #[test]
    #[ignore = "timing: cargo test -p lenz --release layout_a_million -- --ignored --nocapture"]
    fn layout_a_million_synthetic_commits() {
        // Blocks of five rows: a merge, its first parent, a two-commit side branch, the fork point; every 100th
        // merge also has a parent 10 blocks down (a long edge).
        let before = crate::perf::rss_mib();
        let mut log = Log::default();
        let mut laid_out = std::time::Duration::ZERO;
        let mut batch = Vec::new();
        // ids spread like real ones, not sharing the index's prefix
        let spread = |n: u32| format!("{:016x}{n:024x}", u64::from(n).wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let c = |n: u32, parents: &[u32]| Commit { id: spread(n), parents: parents.iter().map(|&p| spread(p)).collect(), ..c(n, &[]) };
        for b in 0..200_000u32 {
            let n = b * 5;
            let mut merge = vec![n + 1, n + 2];
            if b % 100 == 0 && b + 10 < 200_000 {
                merge.push(n + 54);
            }
            batch.extend([c(n, &merge), c(n + 1, &[n + 4]), c(n + 2, &[n + 3]), c(n + 3, &[n + 4])]);
            let next: &[u32] = if b + 1 < 200_000 { &[n + 5] } else { &[] };
            batch.push(c(n + 4, next));
            if batch.len() >= 4096 || b + 1 == 200_000 {
                let started = std::time::Instant::now();
                log.extend(std::mem::take(&mut batch));
                laid_out += started.elapsed();
            }
        }
        let started = std::time::Instant::now();
        let mut count = 0;
        for top in (0..log.len()).step_by(10_007) {
            count += log.edges(top..top + 60).len();
        }
        let queries = (log.len() as usize).div_ceil(10_007);
        let query_ms = started.elapsed().as_secs_f64() * 1000.0;
        let growth = before.zip(crate::perf::rss_mib()).map(|(a, b)| (b - a).max(0.0));
        crate::perf::record("history/layout-million", "ms", &[laid_out.as_secs_f64() * 1000.0]);
        crate::perf::record("history/visible-queries", "ms", &[query_ms]);
        if let Some(mib) = growth { crate::perf::record("history/resident-growth", "MiB", &[mib]); }
        eprintln!(
            "{} commits laid out in {:.0} ms, width {}, ~{} MiB more resident; {} edge queries of 60 rows in {:.2} ms \
             ({} edges)",
            log.len(),
            laid_out.as_secs_f64() * 1000.0,
            log.width(),
            growth.map(|v| format!("{v:.1}")).unwrap_or_else(|| "unavailable".into()),
            queries,
            query_ms,
            count
        );
        assert_eq!(log.len(), 1_000_000);
        const LAYOUT_MS: f64 = 570.0;
        let ms = laid_out.as_secs_f64() * 1000.0;
        crate::perf::check(ms <= 2.0 * LAYOUT_MS, "million-commit layout exceeds budget");
    }
}

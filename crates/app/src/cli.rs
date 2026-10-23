//! Command-line parsing: `git diff`'s grammar, plus two plain files or directories.

use std::path::{Path, PathBuf};

use lenz_engine::{Options, WhitespaceMode};

use crate::{
    changeset::{self, Spec},
    git,
};

/// What the invocation opens.
#[derive(Debug, Clone, PartialEq)]
pub enum Launch {
    /// Standalone information window, without a repository or files.
    About,
    /// A Change Set given as `git diff` arguments, files or directories: opens in Focus Mode.
    Diff,
    /// `lenz log [<revs>] [-- <path>…]`, or one file's history with `--follow <file>`: opens the picker.
    Log { follow: Option<String> },
    /// `lenz pr [<n>|<url>]`: a pull request, or the current branch's.
    Pr(Option<String>),
    /// `lenz merge …`: resolve conflicts.
    Merge(Merge),
}

/// What `lenz merge` resolves.
#[derive(Debug, Clone, PartialEq)]
pub enum Merge {
    /// `lenz merge BASE LOCAL REMOTE MERGED` (`git mergetool`): one file; the Result is saved to `merged`.
    Files { base: PathBuf, ours: PathBuf, theirs: PathBuf, merged: PathBuf },
    /// `lenz merge [[--] <path>…]`: the Conflict List of the operation in progress, limited to `paths`.
    List(Vec<String>),
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub launch: Launch,
    /// The Change Set to load.
    pub spec: Spec,
    pub options: Options,
    /// `--name`: the label for a single Comparison (from `git difftool`'s `$MERGED`; may be empty).
    pub name: Option<String>,
    /// `--scroll`: the starting Scroll Mode.
    pub scroll: crate::view::ScrollMode,
    /// `--blame`: start with Blame Annotations on.
    pub blame: bool,
    /// `--line N`: open the first file at line N of the right Side.
    pub line: Option<u32>,
    /// The starting presentation of a Markdown comparison (`[view] markdown` in the settings file).
    pub markdown: crate::markdown::PresentationMode,
}

impl Request {
    /// Plain `lenz` or `lenz log` in a repository, naming nothing: the launches that reopen where the user left.
    pub fn restores_place(&self) -> bool {
        let plain = matches!(&self.spec, Spec::Git { cached: false, revs, paths, .. } if revs.is_empty() && paths.is_empty());
        plain && matches!(self.launch, Launch::Diff | Launch::Log { follow: None }) && self.line.is_none()
    }
}

/// A usage error; the process exits with code 2.
#[derive(Debug, PartialEq)]
pub struct UsageError(pub String);

/// `--help` or `-h` on its own.
pub fn wants_help(args: &[String]) -> bool { matches!(args, [a] if a == "--help" || a == "-h") }

pub const USAGE: &str = "usage: lenz diff [options] [--staged] [<rev> [<rev>]] [-- <path>...]
       lenz diff [options] [--no-index] <left> <right>
       lenz diff log [<rev>...] [-- <path>...] | lenz diff log --follow <file>
       lenz diff --about | --version
       lenz diff report --last [--print]
       lenz diff pr [<number>|<url>]
       lenz diff merge [[--] <path>...] | lenz diff merge <base> <local> <remote> <merged>
lenz-diff takes the same arguments without the diff prefix.
options: --no-untracked, --name <label>, -w | --whitespace=default|trim|ignore|ignore-chunks, -i,
         --highlight=word|char|line, --scroll=shared|aligned|padded|independent, --blame, --line <n>";

/// Parses `args` (without the program name). `cwd` resolves plain paths and revisions. Starting options come from
/// the settings file; flags override them.
pub fn parse(args: &[String], cwd: &Path) -> Result<Request, UsageError> {
    parse_with(args, cwd, &crate::settings::get().view)
}

/// [`parse`] starting from `view` rather than the settings file.
pub fn parse_with(args: &[String], cwd: &Path, view: &crate::settings::View) -> Result<Request, UsageError> {
    let usage = || UsageError(USAGE.into());
    let defaults = Options { whitespace: view.whitespace, ignore_case: view.ignore_case, highlight: view.highlight };
    // subcommands: only as the first argument (a branch named `log` is `heads/log`)
    match args.first().map(String::as_str) {
        Some("--about") if args.len()==1=>return Ok(Request{launch:Launch::About,spec:Spec::Empty,options:defaults,name:None,scroll:view.scroll,blame:view.blame,line:None,markdown:view.markdown}),
        Some("pr") => {
            let target = match &args[1..] {
                [] => None,
                [one] if !one.starts_with('-') => Some(one.clone()),
                _ => return Err(usage()),
            };
            let spec = Spec::Git { cached: false, revs: Vec::new(), paths: Vec::new(), untracked: false };
            return Ok(Request { launch: Launch::Pr(target), spec, options: defaults, name: None, scroll: view.scroll, blame: view.blame, line: None, markdown: view.markdown });
        }
        Some("merge") => {
            let rest = &args[1..];
            let merge = match rest {
                [base, ours, theirs, merged] if rest.iter().all(|a| !a.starts_with('-')) => {
                    let at = |p: &String| cwd.join(p);
                    Merge::Files { base: at(base), ours: at(ours), theirs: at(theirs), merged: at(merged) }
                }
                [dashdash, paths @ ..] if dashdash == "--" => Merge::List(paths.to_vec()),
                paths if paths.iter().all(|a| !a.starts_with('-')) => Merge::List(paths.to_vec()),
                _ => return Err(usage()),
            };
            // the files form needs no repository (jj, hg, or plain files)
            let spec = match &merge {
                Merge::Files { ours, theirs, .. } => Spec::Plain { left: ours.clone(), right: theirs.clone() },
                Merge::List(_) => Spec::Git { cached: false, revs: Vec::new(), paths: Vec::new(), untracked: false },
            };
            return Ok(Request { launch: Launch::Merge(merge), spec, options: defaults, name: None, scroll: view.scroll, blame: view.blame, line: None, markdown: view.markdown });
        }
        Some("log") => {
            let rest = &args[1..];
            if let Some(i) = rest.iter().position(|a| a == "--follow") {
                let file = rest.get(i + 1).ok_or_else(usage)?.clone();
                let mut others: Vec<String> = rest.to_vec();
                others.drain(i..=i + 1);
                let mut r = parse_with(&others, cwd, view)?;
                if let Spec::Git { untracked, .. } = &mut r.spec {
                    *untracked = false;
                }
                r.launch = Launch::Log { follow: Some(file) };
                return Ok(r);
            }
            let mut r = parse_with(rest, cwd, view)?;
            if let Spec::Git { untracked, .. } = &mut r.spec {
                *untracked = false;
            }
            r.launch = Launch::Log { follow: None };
            return Ok(r);
        }
        _ => {}
    }
    let mut options = defaults;
    let mut scroll = view.scroll;
    let (mut name, mut no_index, mut cached, mut no_untracked, mut dashdash) = (None, false, false, false, false);
    let (mut blame, mut line) = (view.blame, None);
    let number = |s: &str| s.parse::<u32>().ok().filter(|&n| n > 0).ok_or_else(usage);
    let (mut revs, mut paths) = (Vec::new(), Vec::new());
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if dashdash {
            paths.push(arg.clone());
            continue;
        }
        match arg.as_str() {
            "--" => dashdash = true,
            "--no-index" => no_index = true,
            "--staged" | "--cached" => cached = true,
            "--no-untracked" => no_untracked = true,
            "--blame" => blame = true,
            "--line" => line = Some(number(it.next().ok_or_else(usage)?)?),
            a if a.starts_with("--line=") => line = Some(number(&a["--line=".len()..])?),
            "--name" => name = Some(it.next().ok_or_else(usage)?.clone()),
            "-w" | "--ignore-all-space" => options.whitespace = WhitespaceMode::Ignore,
            "-i" | "--ignore-case" => options.ignore_case = true,
            a if a.starts_with("--highlight=") => {
                use lenz_engine::HighlightLevel;
                options.highlight = match &a["--highlight=".len()..] {
                    "word" => HighlightLevel::Word,
                    "char" => HighlightLevel::Char,
                    "line" => HighlightLevel::Line,
                    _ => return Err(usage()),
                }
            }
            a if a.starts_with("--scroll=") => {
                use crate::view::ScrollMode;
                scroll = match &a["--scroll=".len()..] {
                    "shared" => ScrollMode::Shared,
                    "aligned" => ScrollMode::Aligned,
                    "independent" => ScrollMode::Independent,
                    "padded" => ScrollMode::Padded,
                    _ => return Err(usage()),
                }
            }
            a if a.starts_with("--name=") => name = Some(a["--name=".len()..].to_string()),
            a if a.starts_with("--whitespace=") => {
                options.whitespace = match &a["--whitespace=".len()..] {
                    "default" => WhitespaceMode::Default,
                    "trim" => WhitespaceMode::Trim,
                    "ignore" => WhitespaceMode::Ignore,
                    "ignore-chunks" => WhitespaceMode::IgnoreTrimChunks,
                    _ => return Err(usage()),
                }
            }
            a if a.starts_with('-') && a.len() > 1 => return Err(usage()),
            a => revs.push(a.to_string()),
        }
    }

    let plain = |left: &str, right: &str| Spec::Plain { left: left.into(), right: right.into() };
    let spec = if no_index {
        match [revs, paths].concat()[..] {
            [ref left, ref right] if !cached => plain(left, right),
            _ => return Err(usage()),
        }
    } else if let [left, right] = &revs[..]
        && !cached
        && !dashdash
        && is_plain_pair(cwd, left, right)
    {
        plain(left, right)
    } else if revs.len() > 2 {
        return Err(usage());
    } else {
        let untracked = !no_untracked && changeset::right_is_worktree(cached, &revs);
        Spec::Git { cached, revs, paths, untracked }
    };
    Ok(Request { launch: Launch::Diff, spec, options, name, scroll, blame, line, markdown: view.markdown })
}

/// Two arguments are plain paths when both exist on disk and neither names a commit.
fn is_plain_pair(cwd: &Path, left: &str, right: &str) -> bool {
    let exists = |p: &str| cwd.join(p).exists();
    exists(left) && exists(right) && !git::any_commit(cwd, &[left, right])
}

#[cfg(test)]
mod tests {
    #[test]
    fn help_is_asked_for_alone() {
        assert!(wants_help(&["--help".into()]) && wants_help(&["-h".into()]));
        assert!(!wants_help(&[]) && !wants_help(&["--help".into(), "a".into()]) && !wants_help(&["log".into()]));
        assert!(USAGE.starts_with("usage: lenz"));
    }

    use super::*;
    use crate::git::testing::{commit_all, repo, write};

    #[test]
    fn settings_view_defaults_start_the_options_and_flags_win() {
        let mut view = crate::settings::View::default();
        view.whitespace = WhitespaceMode::Ignore;
        view.highlight = lenz_engine::HighlightLevel::Char;
        view.scroll = crate::view::ScrollMode::Padded;
        view.blame = true;
        view.markdown = crate::markdown::PresentationMode::Rendered;
        let r = parse_with(&args(&[]), &tempfile::tempdir().unwrap().path().join("missing"), &view).unwrap();
        assert_eq!((r.options.whitespace, r.options.highlight, r.scroll, r.blame), (WhitespaceMode::Ignore, lenz_engine::HighlightLevel::Char, crate::view::ScrollMode::Padded, true));
        assert_eq!(r.markdown, crate::markdown::PresentationMode::Rendered);
        let r = parse_with(&args(&["--whitespace=default", "--highlight=word", "--scroll=shared"]), &tempfile::tempdir().unwrap().path().join("missing"), &view).unwrap();
        assert_eq!((r.options.whitespace, r.options.highlight, r.scroll), (WhitespaceMode::Default, lenz_engine::HighlightLevel::Word, crate::view::ScrollMode::Shared));
    }

    #[test]
    fn about_launch_requires_no_repository_or_files() {
        let dir=tempfile::tempdir().unwrap();
        let request=parse(&args(&["--about"]),dir.path()).expect("About must launch outside Git");
        let set=changeset::ChangeSet::load(&request.spec,dir.path()).unwrap();
        assert!(set.comparisons.is_empty());assert!(set.git_root().is_none());
        assert!(parse(&args(&["--about","extra"]),dir.path()).is_err());
    }

    #[test]
    fn merge_takes_the_four_mergetool_files_or_paths_to_list() {
        let cwd = Path::new("/repo");
        let r = parse(&args(&["merge", "b", "l", "r", "m"]), cwd).unwrap();
        let p = |s: &str| cwd.join(s);
        assert_eq!(r.launch, Launch::Merge(Merge::Files { base: p("b"), ours: p("l"), theirs: p("r"), merged: p("m") }));
        assert_eq!(parse(&args(&["merge"]), cwd).unwrap().launch, Launch::Merge(Merge::List(vec![])));
        let list = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse(&args(&["merge", "src", "x"]), cwd).unwrap().launch, Launch::Merge(Merge::List(list(&["src", "x"]))));
        // four paths to list, not four files
        let four = parse(&args(&["merge", "--", "a", "b", "c", "d"]), cwd).unwrap();
        assert_eq!(four.launch, Launch::Merge(Merge::List(list(&["a", "b", "c", "d"]))));
        assert!(parse(&args(&["merge", "-w"]), cwd).is_err());
        assert_eq!(parse_bare(&["heads/merge"]).unwrap().launch, Launch::Diff);
    }

    #[test]
    fn log_and_pr_subcommands_open_the_repository_window() {
        let r = parse_bare(&["log"]).unwrap();
        assert_eq!(r.launch, Launch::Log { follow: None });
        assert_eq!(r.spec, git_spec(false, &[], &[], false));
        let r = parse_bare(&["log", "main", "--", "src"]).unwrap();
        assert_eq!(r.launch, Launch::Log { follow: None });
        assert_eq!(r.spec, git_spec(false, &["main"], &["src"], false));
        let r = parse_bare(&["log", "--follow", "src/a.rs"]).unwrap();
        assert_eq!(r.launch, Launch::Log { follow: Some("src/a.rs".into()) });
        assert!(parse_bare(&["log", "--follow"]).is_err());
        assert_eq!(parse_bare(&["pr"]).unwrap().launch, Launch::Pr(None));
        assert_eq!(parse_bare(&["pr", "123"]).unwrap().launch, Launch::Pr(Some("123".into())));
        assert_eq!(parse_bare(&["pr", "https://github.com/o/r/pull/9"]).unwrap().launch, Launch::Pr(Some("https://github.com/o/r/pull/9".into())));
        assert!(parse_bare(&["pr", "1", "2"]).is_err());
        // a branch named like a subcommand stays reachable
        assert_eq!(parse_bare(&["heads/log"]).unwrap().launch, Launch::Diff);
        assert_eq!(parse_bare(&[]).unwrap().launch, Launch::Diff);
    }

    #[test]
    fn blame_and_line_flags() {
        let r = parse_bare(&["--blame", "--line", "120", "a", "b", "--", "src/x.rs"]).unwrap();
        assert!(r.blame);
        assert_eq!(r.line, Some(120));
        assert_eq!(r.spec, git_spec(false, &["a", "b"], &["src/x.rs"], false));
        assert_eq!(parse_bare(&["--line=7"]).unwrap().line, Some(7));
        let r = parse_bare(&[]).unwrap();
        assert!(!r.blame && r.line.is_none());
        assert!(parse_bare(&["--line", "x"]).is_err());
        assert!(parse_bare(&["--line"]).is_err());
    }

    #[test]
    fn highlight_flag_sets_the_starting_highlight_level() {
        use lenz_engine::HighlightLevel;
        assert_eq!(parse_bare(&["--highlight=char"]).unwrap().options.highlight, HighlightLevel::Char);
        assert_eq!(parse_bare(&["--highlight=line"]).unwrap().options.highlight, HighlightLevel::Line);
        assert_eq!(parse_bare(&[]).unwrap().options.highlight, HighlightLevel::Word);
        assert!(parse_bare(&["--highlight=paragraph"]).is_err());
    }

    #[test]
    fn scroll_flag_sets_the_starting_scroll_mode() {
        use crate::view::ScrollMode;
        assert_eq!(parse_bare(&["--scroll=padded"]).unwrap().scroll, ScrollMode::Padded);
        assert_eq!(parse_bare(&["--scroll=independent"]).unwrap().scroll, ScrollMode::Independent);
        assert_eq!(parse_bare(&["--scroll=aligned"]).unwrap().scroll, ScrollMode::Aligned);
        assert_eq!(parse_bare(&[]).unwrap().scroll, ScrollMode::Shared);
        assert!(parse_bare(&["--scroll=sideways"]).is_err());
    }

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    /// Parses with a cwd holding nothing, so every positional is a revision.
    fn parse_bare(a: &[&str]) -> Result<Request, UsageError> {
        parse(&args(a), &tempfile::tempdir().unwrap().path().join("missing"))
    }

    fn git_spec(cached: bool, revs: &[&str], paths: &[&str], untracked: bool) -> Spec {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
        Spec::Git { cached, revs: v(revs), paths: v(paths), untracked }
    }

    fn spec(a: &[&str]) -> Spec {
        parse_bare(a).unwrap().spec
    }

    #[test]
    fn no_arguments_compare_the_working_tree_with_the_index_including_untracked_files() {
        assert_eq!(spec(&[]), git_spec(false, &[], &[], true));
    }

    #[test]
    fn staged_and_cached_compare_the_index_with_head() {
        assert_eq!(spec(&["--staged"]), git_spec(true, &[], &[], false));
        assert_eq!(spec(&["--cached"]), git_spec(true, &[], &[], false));
    }

    #[test]
    fn one_revision_compares_the_working_tree_with_it() {
        assert_eq!(spec(&["HEAD~1"]), git_spec(false, &["HEAD~1"], &[], true));
    }

    #[test]
    fn two_revisions_and_ranges_compare_revisions_without_untracked_files() {
        assert_eq!(spec(&["a", "b"]), git_spec(false, &["a", "b"], &[], false));
        assert_eq!(spec(&["a..b"]), git_spec(false, &["a..b"], &[], false));
        assert_eq!(spec(&["a...b"]), git_spec(false, &["a...b"], &[], false));
    }

    #[test]
    fn paths_after_double_dash_filter_the_change_set() {
        assert_eq!(spec(&["--", "src", "a b.txt"]), git_spec(false, &[], &["src", "a b.txt"], true));
        assert_eq!(spec(&["HEAD", "--", "-w"]), git_spec(false, &["HEAD"], &["-w"], true));
        assert_eq!(spec(&["--cached", "--", "x"]), git_spec(true, &[], &["x"], false));
    }

    #[test]
    fn no_untracked_hides_untracked_files() {
        assert_eq!(spec(&["--no-untracked"]), git_spec(false, &[], &[], false));
        assert_eq!(spec(&["HEAD", "--no-untracked"]), git_spec(false, &["HEAD"], &[], false));
    }

    #[test]
    fn name_sets_the_label_and_may_be_empty() {
        assert_eq!(parse_bare(&["--name", "src/x.rs", "--no-index", "a", "b"]).unwrap().name, Some("src/x.rs".into()));
        assert_eq!(parse_bare(&["--name", "", "--no-index", "a", "b"]).unwrap().name, Some(String::new()));
        assert_eq!(parse_bare(&["--name=", "--no-index", "a", "b"]).unwrap().name, Some(String::new()));
        assert_eq!(parse_bare(&[]).unwrap().name, None);
        assert!(parse_bare(&["--name"]).is_err());
    }

    #[test]
    fn no_index_forces_plain_mode_with_exactly_two_paths() {
        let plain = Spec::Plain { left: "HEAD".into(), right: "b".into() };
        assert_eq!(spec(&["--no-index", "HEAD", "b"]), plain);
        assert_eq!(spec(&["--no-index", "--", "HEAD", "b"]), plain);
        assert!(parse_bare(&["--no-index", "a"]).is_err());
        assert!(parse_bare(&["--no-index", "a", "b", "c"]).is_err());
        assert!(parse_bare(&["--no-index", "--staged", "a", "b"]).is_err());
    }

    #[test]
    fn two_existing_paths_outside_a_repo_are_compared_as_plain_files() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "old.txt", "a");
        write(dir.path(), "new.txt", "b");
        let r = parse(&args(&["old.txt", "new.txt"]), dir.path()).unwrap();
        assert_eq!(r.spec, Spec::Plain { left: "old.txt".into(), right: "new.txt".into() });
        // difftool hands an empty Side over as /dev/null.
        let r = parse(&args(&["/dev/null", "new.txt"]), dir.path()).unwrap();
        assert_eq!(r.spec, Spec::Plain { left: "/dev/null".into(), right: "new.txt".into() });
    }

    #[test]
    fn existing_paths_that_name_commits_are_revisions() {
        let dir = repo();
        write(dir.path(), "main", "a file named like the branch");
        write(dir.path(), "other.txt", "x");
        commit_all(dir.path(), "first");
        let r = parse(&args(&["main", "other.txt"]), dir.path()).unwrap();
        assert_eq!(r.spec, git_spec(false, &["main", "other.txt"], &[], false));
        let r = parse(&args(&["other.txt", "main"]), dir.path()).unwrap();
        assert!(matches!(r.spec, Spec::Git { .. }));
        // Not revisions, inside a repo: still plain.
        write(dir.path(), "third.txt", "y");
        let r = parse(&args(&["other.txt", "third.txt"]), dir.path()).unwrap();
        assert_eq!(r.spec, Spec::Plain { left: "other.txt".into(), right: "third.txt".into() });
    }

    #[test]
    fn whitespace_and_case_flags_set_the_starting_options() {
        let r = parse_bare(&["-w"]).unwrap();
        assert_eq!(r.options.whitespace, WhitespaceMode::Ignore);
        let r = parse_bare(&["--ignore-all-space", "a", "b"]).unwrap();
        assert_eq!(r.options.whitespace, WhitespaceMode::Ignore);
        let r = parse_bare(&["--whitespace=trim", "-i", "a", "b"]).unwrap();
        assert_eq!(r.options.whitespace, WhitespaceMode::Trim);
        assert!(r.options.ignore_case);
        let r = parse_bare(&["a", "--whitespace=ignore-chunks", "--ignore-case", "b"]).unwrap();
        assert_eq!(r.options.whitespace, WhitespaceMode::IgnoreTrimChunks);
        assert!(r.options.ignore_case);
        assert_eq!(r.spec, git_spec(false, &["a", "b"], &[], false));
        assert!(parse_bare(&["--whitespace=sometimes"]).is_err());
    }

    #[test]
    fn unknown_flags_and_too_many_revisions_are_usage_errors() {
        assert!(parse_bare(&["--frobnicate"]).is_err());
        assert!(parse_bare(&["-b"]).is_err());
        assert!(parse_bare(&["--blame-everything"]).is_err());
        assert!(parse_bare(&["a", "b", "c"]).is_err());
    }
}

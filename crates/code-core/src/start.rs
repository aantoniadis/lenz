//! The welcome screen and the ways to open a project, as both adapters present them: what each start item is called
//! and where it lives (`~` for the home folder), the welcome copy, the keyboard selection, and the folders an Open
//! Folder prompt suggests for what has been typed. Adapters own only layout and painting.
use std::path::{Path, PathBuf};

use crate::session::{CommandId, StartItem};

/// The welcome screen's line under the application's name.
pub const TAGLINE: &str = "Open a project folder to start, or pick up where you left off.";
/// The welcome screen's closing tip, with the project-tab keys as this keymap and platform spell them.
pub fn tip(next: Option<&str>, previous: Option<&str>) -> String {
    match (next, previous) {
        (Some(next), Some(previous)) => format!("Tip: open more folders as project tabs and switch with {next} / {previous}; F10 opens the menus."),
        _ => "Tip: open more folders as project tabs (File → Project Tabs); F10 opens the menus.".into(),
    }
}
/// What the recent-projects dialog says it can do; `modifier` is the platform's primary modifier (`Ctrl`, `Cmd`).
pub fn recent_hint(modifier: &str) -> String {
    format!("↑↓ choose · Enter open · {modifier}+Enter open in a new project tab · type to filter · Esc close")
}
/// The most folders an Open Folder prompt suggests, and the most directory entries it reads to find them.
pub const SUGGESTIONS: usize = 40;
const READ_LIMIT: usize = 4096;

/// The user's home folder, for showing paths under it as `~/…`.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute() && h != Path::new("/"))
}

/// `path` for people: its last component, and the folder holding it with the home folder as `~`.
pub fn place(path: &Path, home: Option<&Path>) -> (String, String) {
    let shown = shown(path);
    let path = shown.as_path();
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    let location = path.parent().map(|parent| tilde(parent, home)).unwrap_or_default();
    (name, location)
}

/// The path a person reads for `path`: an entry inside an archive as `archive.zip!/entry`, any other path as it is.
pub fn shown(path: &Path) -> PathBuf {
    crate::archive::Location::from_uri(path).map_or_else(|| path.to_path_buf(), |location| PathBuf::from(location.display_path()))
}

/// `path` with the home folder shown as `~` (an archive entry as `archive.zip!/entry`).
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    let shown = shown(path);
    let path = shown.as_path();
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl StartItem {
    /// The row's main text: a command's label, a recent folder's or file's name.
    pub fn label(&self) -> String {
        match self {
            StartItem::Command(id) => id.label().to_string(),
            StartItem::RecentFolder(path) | StartItem::RecentFile(path) => place(path, None).0,
        }
    }

    /// The row's quieter text: where a recent folder or file lives (`~/…`); commands show their shortcut instead.
    pub fn location(&self, home: Option<&Path>) -> Option<String> {
        match self {
            StartItem::Command(_) => None,
            StartItem::RecentFolder(path) | StartItem::RecentFile(path) => Some(place(path, home).1),
        }
    }

    /// The command a row runs, for its shortcut.
    pub fn command(&self) -> Option<CommandId> {
        match self { StartItem::Command(id) => Some(*id), _ => None }
    }
}

/// A recent project as the Open Recent Project dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentProject {
    pub path: PathBuf,
    pub name: String,
    /// Where it lives, `~` for the home folder.
    pub location: String,
    /// The git branch checked out there (`detached a1b2c3d` without one), when it is a repository and was read.
    pub branch: Option<String>,
}

/// The recent projects whose name, path or branch contains `query` (case-insensitive), most recent first.
pub fn recent_projects(recent: &[PathBuf], query: &str, home: Option<&Path>, branch: impl Fn(&Path) -> Option<String>) -> Vec<RecentProject> {
    let query = query.trim().to_lowercase();
    recent.iter()
        .map(|path| { let (name, location) = place(path, home); RecentProject { path: path.clone(), name, location, branch: branch(path) } })
        .filter(|r| query.is_empty() || tilde(&r.path, home).to_lowercase().contains(&query) || r.branch.as_ref().is_some_and(|b| b.to_lowercase().contains(&query)))
        .collect()
}

/// The git branch checked out in the repository holding `root` (or `detached` and a short commit), read from its
/// `HEAD` file without running git: the nearest `.git` folder, or a worktree's `.git` file naming its folder.
pub fn git_branch(root: &Path) -> Option<String> {
    // An archive entry has no repository: and its URI, not being an absolute path, must not reach the current folder's.
    if !root.is_absolute() || crate::archive::Location::from_uri(root).is_some() { return None; }
    let dot = root.ancestors().take(64).map(|dir| dir.join(".git")).find(|dot| dot.exists())?;
    let git_dir = if dot.is_file() {
        let text = std::fs::read_to_string(&dot).ok()?;
        let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
        if target.is_absolute() { target } else { dot.parent()?.join(target) }
    } else { dot };
    let head = lenz_content::branches::head_of(&git_dir)?;
    match (head.branch, head.sha) {
        (Some(branch), _) => Some(branch),
        (None, Some(sha)) => Some(format!("detached {}", sha.chars().take(7).collect::<String>())),
        (None, None) => None,
    }
}

/// A badge's letters for a name: the first letters of its first two words (`lenz-api` → `LA`), else its first two.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let letters: String = match words.as_slice() {
        [] => "·".into(),
        [one] => one.chars().take(2).collect(),
        [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

/// The colours a badge's [`tint`] picks from: the theme's livelier syntax colours, so badges suit every theme.
pub fn badge_colors(p: &lenz_content::theme::Palette) -> [u32; 6] {
    [p.accent, p.syntax.keyword, p.syntax.string, p.syntax.function, p.syntax.type_, p.syntax.number]
}

/// A stable colour choice for a name's badge (an index the adapter maps into its palette).
pub fn tint(name: &str) -> usize {
    name.bytes().fold(0usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b)))
}

/// The folders an Open Folder prompt offers for `input`, with `resolve` turning typed text into a path as the prompt
/// does (`~`, relative paths): the typed folder itself when it exists, then the folders in the typed folder part
/// whose names start with the last component typed (case-insensitive; hidden ones only once a `.` is typed). Empty
/// input offers nothing (the adapter shows the recent projects). Reads at most one directory, bounded.
pub fn folder_suggestions(input: &str, resolve: impl Fn(&str) -> PathBuf) -> Vec<PathBuf> {
    if input.trim().is_empty() { return Vec::new(); }
    let split = input.rfind('/').map_or(0, |i| i + 1);
    let (head, prefix) = (input.get(..split).unwrap_or_default(), input.get(split..).unwrap_or_default().to_lowercase());
    let dir = resolve(if head.is_empty() { "." } else { head });
    let exact = resolve(input);
    let mut out = Vec::new();
    if exact.is_dir() { out.push(exact.clone()); }
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    let mut found: Vec<(String, PathBuf)> = entries
        .take(READ_LIMIT)
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            let wanted = lower.starts_with(&prefix) && (!name.starts_with('.') || prefix.starts_with('.'));
            // A symlink to a folder counts as a folder, as the explorer shows it.
            (wanted && entry.path().is_dir()).then(|| (lower, entry.path()))
        })
        .filter(|(_, path)| *path != exact)
        .collect();
    found.sort();
    out.extend(found.into_iter().map(|(_, p)| p).take(SUGGESTIONS));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_show_the_name_and_where_it_lives_under_home_as_a_tilde() {
        let home = Path::new("/home/ada");
        assert_eq!(place(Path::new("/home/ada/code/lenz"), Some(home)), ("lenz".into(), "~/code".into()));
        assert_eq!(place(Path::new("/home/ada/notes"), Some(home)), ("notes".into(), "~".into()));
        assert_eq!(place(Path::new("/srv/app"), Some(home)), ("app".into(), "/srv".into()));
        assert_eq!(place(Path::new("/home/adam/x"), Some(home)), ("x".into(), "/home/adam".into()), "a sibling folder is not home");
        assert_eq!(StartItem::RecentFolder("/home/ada/code/lenz".into()).label(), "lenz");
        assert_eq!(StartItem::Command(CommandId::OpenFolder).location(Some(home)), None);
        assert_eq!((initials("lenz-api"), initials("notes"), initials("web_dashboard_v2"), initials("--")), ("LA".into(), "NO".into(), "WD".into(), "·".into()));
        assert_eq!(tint("lenz-api"), tint("lenz-api"), "a name keeps its colour");
        let recent = [PathBuf::from("/home/ada/code/lenz-api"), PathBuf::from("/home/ada/work/portal")];
        let branch = |p: &Path| p.ends_with("portal").then(|| "feature/login".to_string());
        let names = |q: &str| recent_projects(&recent, q, Some(home), branch).into_iter().map(|r| (r.name, r.location)).collect::<Vec<_>>();
        assert_eq!(names("LOGIN"), [("portal".to_string(), "~/work".to_string())], "the branch matches too");
        assert_eq!(names(""), [("lenz-api".into(), "~/code".into()), ("portal".into(), "~/work".into())]);
        assert_eq!(names("WORK"), [("portal".to_string(), "~/work".to_string())], "the location matches too");
        assert!(names("nothing").is_empty());
    }

    #[test]
    fn git_branches_come_from_head_files_of_repositories_worktrees_and_detached_heads() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("sub/deeper")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feature/login\n").unwrap();
        assert_eq!(git_branch(&repo).as_deref(), Some("feature/login"));
        assert_eq!(git_branch(&repo.join("sub/deeper")).as_deref(), Some("feature/login"), "a folder inside a repository");
        // A linked worktree: a `.git` file naming its folder, relative to the worktree.
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(repo.join(".git/worktrees/wt")).unwrap();
        std::fs::write(repo.join(".git/worktrees/wt/HEAD"), "0123456789abcdef0123456789abcdef01234567\n").unwrap();
        std::fs::create_dir_all(&worktree).unwrap();
        std::fs::write(worktree.join(".git"), "gitdir: ../repo/.git/worktrees/wt\n").unwrap();
        assert_eq!(git_branch(&worktree).as_deref(), Some("detached 0123456"));
        let plain = dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(dir.path().ancestors().all(|a| !a.join(".git").exists()), "the fixture sits outside any repository");
        assert_eq!(git_branch(&plain), None);
    }

    #[test]
    fn folder_suggestions_offer_the_typed_folder_then_matching_folders_only() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["alpha", "Alpine", "beta", ".hidden", "alpha/inner"] { std::fs::create_dir_all(dir.path().join(name)).unwrap(); }
        std::fs::write(dir.path().join("alps.txt"), "a file is not a folder").unwrap();
        let base = dir.path().display().to_string();
        let names = |found: Vec<PathBuf>| found.iter().map(|p| p.strip_prefix(dir.path()).unwrap().display().to_string()).collect::<Vec<_>>();
        let suggest = |input: &str| names(folder_suggestions(input, |t: &str| PathBuf::from(t)));
        assert_eq!(suggest(&format!("{base}/al")), ["alpha", "Alpine"]);
        assert_eq!(suggest(&format!("{base}/alpha")), ["alpha"], "the typed folder first, not twice");
        assert_eq!(suggest(&format!("{base}/alpha/")), ["alpha", "alpha/inner"]);
        assert_eq!(suggest(&format!("{base}/")), ["", "alpha", "Alpine", "beta"], "hidden folders wait for a dot");
        assert_eq!(suggest(&format!("{base}/.")), ["", ".hidden"]);
        assert!(folder_suggestions("", |t: &str| PathBuf::from(t)).is_empty());
        assert!(suggest(&format!("{base}/nothing/")).is_empty());
    }
}

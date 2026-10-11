//! Choosing a repository when Lenz starts outside one: recent repositories (from saved places), a typed path with
//! Tab completion, and validation. gpui-free; both frontends present it and rerun the launch inside the choice.
use std::path::{Path, PathBuf};

use crate::{changeset, cli};

const MAX_RECENT: usize = 20;

/// Whether `request` needs a repository that `cwd` is not inside.
pub fn needed(request: &cli::Request, cwd: &Path) -> bool {
    let wants = matches!(request.spec, changeset::Spec::Git { .. })
        || matches!(request.launch, cli::Launch::Merge(cli::Merge::List(_)) | cli::Launch::Pr(_));
    // A `.git/HEAD` or a worktree's `.git` file above `cwd` (or GIT_DIR) settles it without starting git on every
    // launch; anything less certain asks git.
    let marked = |d: &Path| {
        let git = d.join(".git");
        git.join("HEAD").is_file() || (git.is_file() && std::fs::read(&git).is_ok_and(|b| b.starts_with(b"gitdir:")))
    };
    let likely = std::env::var_os("GIT_DIR").is_some() || cwd.ancestors().any(marked);
    wants && !likely && changeset::toplevel(cwd).is_err()
}

/// Repository roots with a saved place under `state_home`, most recently left first, that are still directories.
pub fn recent_in(state_home: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(state_home.join("lenz/places")) else { return Vec::new() };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = dir.flatten()
        .filter_map(|e| {
            let time = e.metadata().ok()?.modified().ok()?;
            let root = crate::place::root_of(&e.path())?;
            root.is_dir().then_some((time, root))
        })
        .collect();
    found.sort_by_key(|e| std::cmp::Reverse(e.0));
    found.into_iter().map(|(_, root)| root).take(MAX_RECENT).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum PickerCommand {
    Insert(char),
    Backspace,
    Up,
    Down,
    /// Tab: complete the typed path's last component among its folder's subfolders.
    Complete,
    /// Enter: the typed path when it names a folder, else the selected recent repository.
    Submit,
    /// A recent row clicked, or a folder from the native dialog.
    Choose(PathBuf),
}

#[derive(Debug, Clone)]
pub struct RepoPicker {
    pub input: String,
    pub selected: usize,
    pub error: Option<String>,
    cwd: PathBuf,
    recent: Vec<PathBuf>,
}

impl RepoPicker {
    pub fn new(cwd: PathBuf) -> Self {
        let recent = crate::place::state_home().map(|s| recent_in(&s)).unwrap_or_default();
        Self::with_recent(cwd, recent)
    }

    pub fn with_recent(cwd: PathBuf, recent: Vec<PathBuf>) -> Self {
        Self { input: String::new(), selected: 0, error: None, cwd, recent }
    }

    pub fn cwd(&self) -> &Path { &self.cwd }

    /// Recent repositories whose path contains the typed text (case-insensitive).
    pub fn matches(&self) -> Vec<&Path> {
        let needle = self.input.to_lowercase();
        self.recent.iter().map(PathBuf::as_path).filter(|p| p.to_string_lossy().to_lowercase().contains(&needle)).collect()
    }

    /// Applies `command`; returns the repository root once one is chosen.
    pub fn execute(&mut self, command: PickerCommand) -> Option<PathBuf> {
        match command {
            PickerCommand::Insert(c) => { self.input.push(c); self.selected = 0; self.error = None; }
            PickerCommand::Backspace => { self.input.pop(); self.selected = 0; self.error = None; }
            PickerCommand::Up => self.selected = self.selected.saturating_sub(1),
            PickerCommand::Down => self.selected = (self.selected + 1).min(self.matches().len().saturating_sub(1)),
            PickerCommand::Complete => self.complete(),
            PickerCommand::Submit => {
                let typed = (!self.input.trim().is_empty()).then(|| self.resolve(self.input.trim())).filter(|p| p.is_dir());
                let target = typed.or_else(|| self.matches().get(self.selected).map(|p| p.to_path_buf()));
                return match target {
                    Some(path) => self.open(path),
                    None => { self.error = Some(if self.input.trim().is_empty() { "Type a folder path".into() } else { format!("No such folder: {}", self.input.trim()) }); None }
                };
            }
            PickerCommand::Choose(path) => return self.open(path),
        }
        None
    }

    fn open(&mut self, path: PathBuf) -> Option<PathBuf> {
        match changeset::toplevel(&path) {
            Ok(root) => Some(root),
            Err(_) => { self.error = Some(format!("Not a Git repository: {}", path.display())); None }
        }
    }

    /// `~` is the home folder; relative paths start at the launch folder.
    fn resolve(&self, text: &str) -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        match (text.strip_prefix('~'), home) {
            (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
            _ => self.cwd.join(text),
        }
    }

    fn complete(&mut self) {
        let (folder, prefix) = match self.input.rsplit_once('/') {
            Some((folder, prefix)) => (format!("{folder}/"), prefix.to_owned()),
            None => (String::new(), self.input.clone()),
        };
        let Ok(entries) = std::fs::read_dir(self.resolve(if folder.is_empty() { "." } else { &folder })) else { return };
        let mut names: Vec<String> = entries.flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with(&prefix) && (prefix.starts_with('.') || !n.starts_with('.')))
            .collect();
        names.sort();
        let Some(first) = names.first() else { return };
        let common = names.iter().fold(first.clone(), |acc, n| acc.chars().zip(n.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect());
        let slash = if names.len() == 1 { "/" } else { "" };
        self.input = format!("{folder}{common}{slash}");
        self.selected = 0;
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::repo;

    #[test]
    fn typed_path_completes_and_opens_the_repository_root() {
        let base = tempfile::tempdir().unwrap();
        let dir = repo();
        std::os::unix::fs::symlink(dir.path(), base.path().join("project")).unwrap();
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let mut picker = RepoPicker::with_recent(base.path().into(), Vec::new());
        for c in "pr".chars() { picker.execute(PickerCommand::Insert(c)); }
        picker.execute(PickerCommand::Complete);
        assert_eq!(picker.input, "project/");
        let root = picker.execute(PickerCommand::Submit).expect("repository chosen");
        assert_eq!(root, changeset::toplevel(dir.path()).unwrap());
    }

    #[test]
    fn a_folder_outside_git_or_a_missing_one_explains_and_stays_open() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let mut picker = RepoPicker::with_recent(base.path().into(), Vec::new());
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert_eq!(picker.error.as_deref(), Some("Type a folder path"));
        for c in "plain".chars() { picker.execute(PickerCommand::Insert(c)); }
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert!(picker.error.as_deref().unwrap().starts_with("Not a Git repository"));
        picker.execute(PickerCommand::Insert('x'));
        assert_eq!(picker.error, None, "typing clears the error");
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert_eq!(picker.error.as_deref(), Some("No such folder: plainx"));
    }

    #[test]
    fn recent_repositories_filter_and_select() {
        let a = repo();
        let b = repo();
        let mut picker = RepoPicker::with_recent("/".into(), vec![a.path().into(), b.path().into()]);
        picker.execute(PickerCommand::Down);
        picker.execute(PickerCommand::Down);
        assert_eq!(picker.selected, 1, "selection stops at the last row");
        assert_eq!(picker.execute(PickerCommand::Submit), Some(changeset::toplevel(b.path()).unwrap()));
        let name = a.path().file_name().unwrap().to_string_lossy().into_owned();
        picker.input = name;
        assert_eq!(picker.matches(), vec![a.path()]);
    }

    #[test]
    fn recent_comes_from_saved_places_newest_first() {
        let state = tempfile::tempdir().unwrap();
        let a = repo();
        let b = repo();
        let place = |root: &Path| crate::place::Place { version: crate::place::VERSION, root: root.into(), source: crate::place::Source::Local, file: None, line: None };
        crate::place::save_to(&crate::place::path_in(state.path(), a.path()), &place(a.path())).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::place::save_to(&crate::place::path_in(state.path(), b.path()), &place(b.path())).unwrap();
        crate::place::save_to(&crate::place::path_in(state.path(), Path::new("/gone")), &place(Path::new("/gone"))).unwrap();
        assert_eq!(recent_in(state.path()), vec![b.path().to_path_buf(), a.path().to_path_buf()]);
    }

    #[test]
    fn only_repository_launches_outside_git_need_a_choice() {
        let dir = tempfile::tempdir().unwrap();
        let parse = |args: &[&str]| cli::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), dir.path()).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(needed(&parse(&[]), dir.path()), "an empty .git folder is not a repository");
        assert!(needed(&parse(&["log"]), dir.path()));
        assert!(!needed(&parse(&["--about"]), dir.path()));
        std::fs::write(dir.path().join("a"), "a").unwrap();
        std::fs::write(dir.path().join("b"), "b").unwrap();
        assert!(!needed(&parse(&["a", "b"]), dir.path()));
        let repo = repo();
        assert!(!needed(&cli::parse(&[], repo.path()).unwrap(), repo.path()));
    }
}

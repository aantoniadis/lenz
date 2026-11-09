//! Opening a repository or a plain comparison. Validation and editable fields are shared by both frontends.
use crate::{changeset, cli};
use lenz_content::line_edit::{Edit, Input};
use std::path::{Path, PathBuf};

const MAX_RECENT: usize = 20;

pub fn needed(request: &cli::Request, cwd: &Path) -> bool {
    if request.launch == cli::Launch::Choose {
        return true;
    }
    let wants = matches!(request.spec, changeset::Spec::Git { .. })
        || matches!(
            request.launch,
            cli::Launch::Merge(cli::Merge::List(_)) | cli::Launch::Pr(_)
        );
    let marked = |d: &Path| {
        let git = d.join(".git");
        git.join("HEAD").is_file()
            || (git.is_file() && std::fs::read(&git).is_ok_and(|b| b.starts_with(b"gitdir:")))
    };
    let likely = std::env::var_os("GIT_DIR").is_some() || cwd.ancestors().any(marked);
    wants && !likely && changeset::toplevel(cwd).is_err()
}

pub fn recent_in(state_home: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(state_home.join("lenz/places")) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = dir
        .flatten()
        .filter_map(|e| {
            let time = e.metadata().ok()?.modified().ok()?;
            let root = crate::place::root_of(&e.path())?;
            root.is_dir().then_some((time, root))
        })
        .collect();
    found.sort_by_key(|e| std::cmp::Reverse(e.0));
    found
        .into_iter()
        .map(|(_, root)| root)
        .take(MAX_RECENT)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Repository,
    Files,
    Folders,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Files, Self::Folders, Self::Repository];
    pub fn label(self) -> &'static str {
        match self {
            Self::Repository => "Open Repository",
            Self::Files => "Compare Files",
            Self::Folders => "Compare Folders",
        }
    }
    pub fn field(self, index: usize) -> &'static str {
        match (self, index) {
            (Self::Repository, _) => "Path",
            (_, 0) => "Left",
            _ => "Right",
        }
    }
    pub fn submit(self) -> &'static str {
        if self == Self::Repository {
            "Open"
        } else {
            "Compare"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Choice {
    Repository(PathBuf),
    Comparison { left: PathBuf, right: PathBuf },
}
impl Choice {
    /// Existing repository subcommands retain their arguments; a plain comparison never restores a saved Git place.
    pub fn invocation(&self, original: &[String], cwd: &Path) -> (Vec<String>, PathBuf) {
        match self {
            Self::Repository(root) => (
                (if original == ["--new-comparison"] {
                    &[]
                } else {
                    original
                })
                .to_vec(),
                root.clone(),
            ),
            Self::Comparison { left, right } => (
                vec![
                    "--no-index".into(),
                    left.to_string_lossy().into_owned(),
                    right.to_string_lossy().into_owned(),
                ],
                cwd.into(),
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub enum PickerCommand {
    Insert(char),
    InsertText(String),
    Backspace,
    Edit(Edit),
    SelectAll,
    Up,
    Down,
    Complete,
    Submit,
    Choose(PathBuf),
    Mode(Mode),
    Field(usize),
    NextField,
}

#[derive(Clone, Debug)]
pub struct RepoPicker {
    pub input: Input,
    pub right: Input,
    pub selected: usize,
    pub error: Option<String>,
    pub mode: Mode,
    pub field: usize,
    pub comparisons: bool,
    cwd: PathBuf,
    recent: Vec<PathBuf>,
}

impl RepoPicker {
    pub fn new(cwd: PathBuf) -> Self {
        let recent = crate::place::state_home()
            .map(|s| recent_in(&s))
            .unwrap_or_default();
        Self::with_recent(cwd, recent)
    }
    pub fn with_recent(cwd: PathBuf, recent: Vec<PathBuf>) -> Self {
        Self {
            input: Input::default(),
            right: Input::default(),
            selected: 0,
            error: None,
            mode: Mode::Repository,
            field: 0,
            comparisons: false,
            cwd,
            recent,
        }
    }
    pub fn with_comparisons(mut self, new: bool) -> Self {
        self.comparisons = true;
        if new {
            self.mode = Mode::Files;
        }
        self
    }
    pub fn with_optional_comparisons(self, allowed: bool, new: bool) -> Self {
        if allowed {
            self.with_comparisons(new)
        } else {
            self
        }
    }
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }
    pub fn fields(&self) -> usize {
        if self.mode == Mode::Repository { 1 } else { 2 }
    }
    pub fn active(&self) -> &Input {
        if self.field == 0 {
            &self.input
        } else {
            &self.right
        }
    }
    pub fn edit_field(&mut self, command: crate::field_edit::Command) -> crate::field_edit::Outcome {
        let outcome=self.active_mut().command(command);if outcome.changed {self.edited();}outcome
    }
    fn active_mut(&mut self) -> &mut Input {
        if self.field == 0 {
            &mut self.input
        } else {
            &mut self.right
        }
    }
    pub fn matches(&self) -> Vec<&Path> {
        if self.mode != Mode::Repository {
            return Vec::new();
        }
        let needle = self.input.text().to_lowercase();
        self.recent
            .iter()
            .map(PathBuf::as_path)
            .filter(|p| p.to_string_lossy().to_lowercase().contains(&needle))
            .collect()
    }
    pub fn execute(&mut self, command: PickerCommand) -> Option<Choice> {
        match command {
            PickerCommand::Insert(c) => {
                self.active_mut().insert(&c.to_string());
                self.edited();
            }
            PickerCommand::InsertText(text) => {
                self.active_mut()
                    .insert(text.lines().next().unwrap_or_default());
                self.edited();
            }
            PickerCommand::Backspace => {
                self.active_mut().edit(Edit::Backspace);
                self.edited();
            }
            PickerCommand::Edit(edit) => {
                self.active_mut().edit(edit);
                self.edited();
            }
            PickerCommand::SelectAll => self.active_mut().select_all(),
            PickerCommand::Field(field) => self.field = field.min(self.fields() - 1),
            PickerCommand::NextField => self.field = (self.field + 1) % self.fields(),
            PickerCommand::Mode(mode) if self.comparisons => {
                self.mode = mode;
                self.field = 0;
                self.error = None;
            }
            PickerCommand::Mode(_) => {}
            PickerCommand::Up => self.selected = self.selected.saturating_sub(1),
            PickerCommand::Down => {
                self.selected = (self.selected + 1).min(self.matches().len().saturating_sub(1))
            }
            PickerCommand::Complete => self.complete(),
            PickerCommand::Submit if self.mode == Mode::Repository => {
                let text = self.input.text().trim();
                let typed = (!text.is_empty())
                    .then(|| self.resolve(text))
                    .filter(|p| p.is_dir());
                let target =
                    typed.or_else(|| self.matches().get(self.selected).map(|p| p.to_path_buf()));
                return match target {
                    Some(path) => self.open_repository(path),
                    None => {
                        self.error = Some(if text.is_empty() {
                            "Type a folder path".into()
                        } else {
                            format!("No such folder: {text}")
                        });
                        None
                    }
                };
            }
            PickerCommand::Submit => match self.pair() {
                Ok(choice) => return Some(choice),
                Err(error) => self.error = Some(error),
            },
            PickerCommand::Choose(path) if self.mode == Mode::Repository => {
                return self.open_repository(path);
            }
            PickerCommand::Choose(path) => {
                *self.active_mut() = Input::new(path.to_string_lossy().into_owned());
                self.error = None;
                if self.field == 0 {
                    self.field = 1;
                }
            }
        }
        None
    }
    fn edited(&mut self) {
        self.selected = 0;
        self.error = None;
    }
    fn open_repository(&mut self, path: PathBuf) -> Option<Choice> {
        match changeset::toplevel(&path) {
            Ok(root) => Some(Choice::Repository(root)),
            Err(_) => {
                self.error = Some(format!("Not a Git repository: {}", path.display()));
                None
            }
        }
    }
    fn pair(&self) -> Result<Choice, String> {
        let path = |input: &Input, side: &str| {
            let text = input.text().trim();
            if text.is_empty() {
                return Err(format!(
                    "Enter the {side} {} path",
                    if self.mode == Mode::Files {
                        "file"
                    } else {
                        "folder"
                    }
                ));
            }
            let path = self.resolve(text);
            if if self.mode == Mode::Files {
                path.is_file()
            } else {
                path.is_dir()
            } {
                Ok(path)
            } else {
                Err(format!(
                    "Choose an existing {} for {side}: {}",
                    if self.mode == Mode::Files {
                        "file"
                    } else {
                        "folder"
                    },
                    path.display()
                ))
            }
        };
        Ok(Choice::Comparison {
            left: path(&self.input, "left")?,
            right: path(&self.right, "right")?,
        })
    }
    fn resolve(&self, text: &str) -> PathBuf {
        match (text.strip_prefix('~'), std::env::var_os("HOME")) {
            (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
                PathBuf::from(home).join(rest.trim_start_matches('/'))
            }
            _ => self.cwd.join(text),
        }
    }
    fn complete(&mut self) {
        let input = self.active().text();
        let (folder, prefix) = match input.rsplit_once('/') {
            Some((folder, prefix)) => (format!("{folder}/"), prefix.to_owned()),
            None => (String::new(), input.to_owned()),
        };
        let Ok(entries) =
            std::fs::read_dir(self.resolve(if folder.is_empty() { "." } else { &folder }))
        else {
            return;
        };
        let mut names: Vec<(String, bool)> = entries
            .flatten()
            .filter(|e| self.mode == Mode::Files || e.path().is_dir())
            .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path().is_dir())))
            .filter(|(n, _)| {
                n.starts_with(&prefix) && (prefix.starts_with('.') || !n.starts_with('.'))
            })
            .collect();
        names.sort();
        let Some((first, _)) = names.first() else {
            return;
        };
        let common = names.iter().fold(first.clone(), |acc, (n, _)| {
            acc.chars()
                .zip(n.chars())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a)
                .collect()
        });
        let slash = if names.len() == 1 && names[0].1 {
            "/"
        } else {
            ""
        };
        *self.active_mut() = Input::new(format!("{folder}{common}{slash}"));
        self.edited();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testing::repo;

    #[test]
    fn correction_selection_and_comparison_validation_preserve_inputs_and_bytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("left é.txt"), "left\n").unwrap();
        std::fs::write(dir.path().join("right.txt"), "right\n").unwrap();
        let mut picker = RepoPicker::with_recent(dir.path().into(), vec![]).with_comparisons(true);
        picker.execute(PickerCommand::InsertText("wrong path".into()));
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        picker.execute(PickerCommand::SelectAll);
        picker.execute(PickerCommand::InsertText("left é.txt".into()));
        picker.execute(PickerCommand::Edit(Edit::Start));
        picker.execute(PickerCommand::InsertText("x".into()));
        assert!(picker.active().visible(12).text.starts_with("xleft"));
        picker.execute(PickerCommand::Edit(Edit::Backspace));
        picker.execute(PickerCommand::NextField);
        picker.execute(PickerCommand::InsertText("right.txt\nignored".into()));
        assert_eq!(
            picker.execute(PickerCommand::Submit),
            Some(Choice::Comparison {
                left: dir.path().join("left é.txt"),
                right: dir.path().join("right.txt")
            })
        );
        picker.execute(PickerCommand::Mode(Mode::Folders));
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert!(picker.error.as_deref().unwrap().contains("folder"));
        assert_eq!(picker.input.text(), "left é.txt");
        assert_eq!(
            std::fs::read(dir.path().join("left é.txt")).unwrap(),
            b"left\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("right.txt")).unwrap(),
            b"right\n"
        );
    }

    #[test]
    fn typed_path_completes_and_opens_the_repository_root() {
        let base = tempfile::tempdir().unwrap();
        let dir = repo();
        std::os::unix::fs::symlink(dir.path(), base.path().join("project")).unwrap();
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let mut picker = RepoPicker::with_recent(base.path().into(), Vec::new());
        for c in "pr".chars() {
            picker.execute(PickerCommand::Insert(c));
        }
        picker.execute(PickerCommand::Complete);
        assert_eq!(picker.input.text(), "project/");
        let root = picker
            .execute(PickerCommand::Submit)
            .expect("repository chosen");
        assert_eq!(
            root,
            Choice::Repository(changeset::toplevel(dir.path()).unwrap())
        );
    }

    #[test]
    fn a_folder_outside_git_or_a_missing_one_explains_and_stays_open() {
        let base = tempfile::tempdir().unwrap();
        std::fs::create_dir(base.path().join("plain")).unwrap();
        let mut picker = RepoPicker::with_recent(base.path().into(), Vec::new());
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert_eq!(picker.error.as_deref(), Some("Type a folder path"));
        for c in "plain".chars() {
            picker.execute(PickerCommand::Insert(c));
        }
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert!(
            picker
                .error
                .as_deref()
                .unwrap()
                .starts_with("Not a Git repository")
        );
        picker.execute(PickerCommand::Insert('x'));
        assert_eq!(picker.error, None, "typing clears the error");
        assert_eq!(picker.execute(PickerCommand::Submit), None);
        assert_eq!(picker.error.as_deref(), Some("No such folder: plainx"));
    }

    #[test]
    fn recent_repositories_filter_and_select() {
        let a = repo();
        let b = repo();
        let mut picker =
            RepoPicker::with_recent("/".into(), vec![a.path().into(), b.path().into()]);
        picker.execute(PickerCommand::Down);
        picker.execute(PickerCommand::Down);
        assert_eq!(picker.selected, 1, "selection stops at the last row");
        assert_eq!(
            picker.execute(PickerCommand::Submit),
            Some(Choice::Repository(changeset::toplevel(b.path()).unwrap()))
        );
        let name = a.path().file_name().unwrap().to_string_lossy().into_owned();
        picker.input = Input::new(name);
        assert_eq!(picker.matches(), vec![a.path()]);
    }

    #[test]
    fn recent_comes_from_saved_places_newest_first() {
        let state = tempfile::tempdir().unwrap();
        let a = repo();
        let b = repo();
        let place = |root: &Path| crate::place::Place {
            version: crate::place::VERSION,
            root: root.into(),
            source: crate::place::Source::Local,
            file: None,
            line: None,
        };
        crate::place::save_to(
            &crate::place::path_in(state.path(), a.path()),
            &place(a.path()),
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::place::save_to(
            &crate::place::path_in(state.path(), b.path()),
            &place(b.path()),
        )
        .unwrap();
        crate::place::save_to(
            &crate::place::path_in(state.path(), Path::new("/gone")),
            &place(Path::new("/gone")),
        )
        .unwrap();
        assert_eq!(
            recent_in(state.path()),
            vec![b.path().to_path_buf(), a.path().to_path_buf()]
        );
    }

    #[test]
    fn only_repository_launches_outside_git_need_a_choice() {
        let dir = tempfile::tempdir().unwrap();
        let parse = |args: &[&str]| {
            cli::parse(
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                dir.path(),
            )
            .unwrap()
        };
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(
            needed(&parse(&[]), dir.path()),
            "an empty .git folder is not a repository"
        );
        assert!(needed(&parse(&["log"]), dir.path()));
        assert!(!needed(&parse(&["--about"]), dir.path()));
        std::fs::write(dir.path().join("a"), "a").unwrap();
        std::fs::write(dir.path().join("b"), "b").unwrap();
        assert!(!needed(&parse(&["a", "b"]), dir.path()));
        let repo = repo();
        assert!(!needed(&cli::parse(&[], repo.path()).unwrap(), repo.path()));
    }
}

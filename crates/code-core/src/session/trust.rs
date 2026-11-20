//! Folder trust (Restricted mode): Lenz runs no code from an opened folder until the user trusts it. The first time a
//! language server or formatter that would run the project's code is held back in a folder never asked about, one
//! dialog asks (Trust, Trust Parent Folder or Stay Restricted), naming what would run. The answer is kept privately in
//! providers.json (trusted or declined), so a folder is asked about once; the Restricted status item and Folder Trust…
//! open the dialog again. Trust decisions are made on paths with their links resolved. Only what the open folder's own
//! answer decides asks: a tool held back elsewhere (outside the open folder, or in a folder inside it that was answered
//! on its own) stays restricted without asking. Enter takes the focused choice, Stay Restricted unless Tab moved the
//! focus there just before: any other input to the dialog (a key, typed or pasted text) puts the focus back on Stay
//! Restricted, so that typing into it never trusts.
use super::*;
use semantics::provider::{Trust, RESTRICTED};

/// The dialog's title, the same in both frontends.
pub const TITLE: &str = "Do you trust the authors of the files in this folder?";

/// An answer to the folder-trust dialog. Escape chooses [`TrustChoice::StayRestricted`], and so does Enter until the
/// focus is moved ([`Session::trust_focus`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustChoice {
    StayRestricted,
    Trust,
    TrustParent,
}

/// How the folder-trust dialog's focus moves ([`super::Command::TrustFocus`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusMove {
    /// Tab: the next choice, back to Stay Restricted after the last.
    Next,
    /// Shift+Tab: the previous choice, never past Stay Restricted.
    Previous,
    /// Any other input the dialog receives: back to Stay Restricted.
    Reset,
}

/// The folder-trust dialog: the opened folder, with its links resolved, the folder holding it (see
/// [`offered_parent`]), what would run there, in the order it was held back, and the focused choice (an index into
/// [`Session::trust_choices`]).
#[derive(Clone, Debug)]
pub(crate) struct TrustPrompt {
    folder: PathBuf,
    real: PathBuf,
    parent: Option<PathBuf>,
    runs: Vec<&'static str>,
    focus: usize,
}

/// What Restricted mode keeps off, for the dialog the user opens rather than a held-back tool.
const KEPT_OFF: [&str; 2] = [
    "language servers that run the project's code: rust-analyzer (build scripts and proc macros), typescript-language-server (the project's TypeScript and its plugins), Ruby LSP (Bundler and add-ons), Eclipse JDT LS and Kotlin LSP (Maven and Gradle builds) and gopls (the go command)",
    "the project's own programs: typescript-go's node_modules/.bin/tsc, a configured Python environment's interpreter, ruff and black, and the erb_lint and haml-lint formatters",
];

/// The folder Trust Parent Folder would trust for folder `real`: its parent, but never the file system root, the home
/// folder `home` or a folder holding it (`/home`, `/Users`), which hold nearly everything, nor a folder that is not
/// the user's own (`own`: owned by the user, and not writable by every account, as `/tmp` is).
fn offered_parent(real: &Path, home: Option<&Path>, own: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    real.parent().filter(|d| d.parent().is_some() && !home.is_some_and(|h| h.starts_with(d)) && own(d)).map(Path::to_owned)
}

/// Whether folder `dir` is the user's own: no other account can write to it, and it belongs to the owner of the home
/// folder `home` (when known). Reads the folders' metadata.
fn own_folder(dir: &Path, home: Option<&Path>) -> bool {
    use semantics::provider::{closed_folder, owner_of};
    closed_folder(dir) && home.is_none_or(|h| owner_of(dir).is_some_and(|o| Some(o) == owner_of(h)))
}

impl Session {
    /// The open folder's trust; `None` with no folder open.
    pub fn folder_trust(&self) -> Option<Trust> {
        Some(self.state.semantics.settings().trust(self.project_real.as_deref()?))
    }

    /// The status bar's Restricted item, shown while the open folder is not trusted; it opens the dialog.
    pub fn restricted_label(&self) -> Option<&'static str> {
        self.folder_trust().filter(|t| *t != Trust::Trusted).map(|_| "Restricted")
    }

    /// The folder-trust dialog's text, the same in both frontends; `None` while it is closed.
    pub fn trust_text(&self) -> Option<String> {
        let p = self.state.trust_prompt.as_ref()?;
        let runs: Vec<&str> = if p.runs.is_empty() { KEPT_OFF.to_vec() } else { p.runs.clone() };
        let list: String = runs.iter().map(|r| format!("\n  • {r}")).collect();
        let now = match self.state.semantics.settings().trust(&p.real) {
            Trust::Declined => "You chose to keep it restricted.",
            _ => "It is restricted until you choose.",
        };
        let parent = p.parent.as_ref().map(|d| format!(" Trust Parent Folder trusts {} and every folder in it.", d.display())).unwrap_or_default();
        Some(format!(
            "{}\n\nLenz Code runs no code from this folder in Restricted mode. {now} Trusting it lets Lenz start:{list}\n\nThese run the project's own files as programs. Trust the folder only if you trust its authors.{parent} Stay Restricted keeps them off (\"{RESTRICTED}\"); syntax highlighting, search, Markdown, Git views and the other language servers keep working. Restricted in the status bar asks again.",
            p.folder.display()
        ))
    }

    /// The folder-trust dialog's buttons, in order: Stay Restricted (the default) first.
    pub fn trust_choices(&self) -> Vec<(&'static str, TrustChoice)> {
        let Some(p) = self.state.trust_prompt.as_ref() else { return Vec::new() };
        let mut choices = vec![("Stay Restricted", TrustChoice::StayRestricted), ("Trust", TrustChoice::Trust)];
        if p.parent.is_some() { choices.push(("Trust Parent Folder", TrustChoice::TrustParent)); }
        choices
    }

    /// The folder-trust dialog's focused choice, which Enter takes: Stay Restricted until the focus is moved.
    pub fn trust_focus(&self) -> Option<TrustChoice> {
        let focus = self.state.trust_prompt.as_ref()?.focus;
        let choices = self.trust_choices();
        choices.get(focus).or(choices.first()).map(|c| c.1)
    }

    /// Move the dialog's focus (see [`FocusMove`]).
    pub(super) fn move_trust_focus(&mut self, to: FocusMove) -> Effect {
        let count = self.trust_choices().len();
        let Some(p) = self.state.trust_prompt.as_mut().filter(|_| count > 0) else { return Effect::default() };
        let before = p.focus;
        p.focus = match to {
            FocusMove::Next => (p.focus.min(count - 1) + 1) % count,
            FocusMove::Previous => p.focus.min(count - 1).saturating_sub(1),
            FocusMove::Reset => 0,
        };
        Effect { repaint: p.focus != before, ..Effect::default() }
    }

    /// A tool that would run the project's code (`runs` says what) was held back in folder `real` (its links
    /// resolved): when that folder is in the open folder and decided by the open folder's own answer, never given, the
    /// dialog asks the first time; while it is open, it says this too. Never asked with no folder open, for a folder
    /// already answered, or about a folder outside the open one (it stays restricted: opening it asks).
    pub(super) fn want_trust(&mut self, real: &Path, runs: &'static str) {
        let Some(project) = self.project_real.as_deref() else { return };
        // Unasked here means no folder from `real` up has an answer: the open folder's answer decides it.
        if !real.starts_with(project) || self.state.semantics.settings().trust(real) != Trust::Unasked { return; }
        match self.state.trust_prompt.as_mut() {
            Some(p) => if !p.runs.contains(&runs) { p.runs.push(runs); },
            None => self.open_trust_prompt(Some(runs)),
        }
    }

    /// Open the dialog for the open folder (Folder Trust…, the Restricted status item), or for a held-back tool.
    pub(super) fn open_trust_prompt(&mut self, runs: Option<&'static str>) {
        let (Some(folder), Some(real)) = (self.state.project.as_ref().map(|p| p.root().to_owned()), self.project_real.clone()) else { return };
        let home = self.home.as_deref().map(|h| std::fs::canonicalize(h).unwrap_or_else(|_| h.to_owned()));
        let parent = offered_parent(&real, home.as_deref(), |d| own_folder(d, home.as_deref()));
        self.state.trust_prompt = Some(TrustPrompt { folder, real, parent, runs: runs.into_iter().collect(), focus: 0 });
    }

    /// The dialog's answer: trust the folder (or the folder holding it), or keep it restricted; remembered either way.
    pub(super) fn choose_trust(&mut self, choice: TrustChoice) -> Effect {
        let Some(p) = self.state.trust_prompt.take() else { return Effect::default() };
        let (folder, trusted) = match choice {
            TrustChoice::StayRestricted => (p.real.clone(), false),
            TrustChoice::Trust => (p.real.clone(), true),
            TrustChoice::TrustParent => match p.parent.clone() {
                Some(parent) => (parent, true),
                None => { self.state.trust_prompt = Some(p); return Effect::default(); }
            },
        };
        if self.state.runtime_saving.is_some() || self.state.provider_saves > 0 {
            // Another settings save is running: the answer is not lost, and not applied over it.
            self.state.trust_prompt = Some(p);
            self.state.messages.push("Folder trust: language server settings are being saved; choose again in a moment".into());
            return Effect { repaint: true, ..Effect::default() };
        }
        self.set_folder_trust(folder, trusted)
    }

    /// Trust `folder` (a path with its links resolved), or keep it restricted, saved privately: servers held back
    /// start, or those that ran because of trust stop, without reopening anything.
    pub(super) fn set_folder_trust(&mut self, folder: PathBuf, trusted: bool) -> Effect {
        let browse = self.state.project.as_ref().map(|p| p.root().to_owned());
        self.state.asking = None;
        self.cancel_semantic_analysis();
        let settings = self.state.semantics.set_trust(folder.clone(), trusted, browse.as_deref());
        self.save_provider_settings(settings);
        self.synced = 0;
        self.state.messages.push(if trusted { format!("Trusted {}: its project tools may run", folder.display()) }
            else { format!("{} stays restricted: Lenz runs no code from it", folder.display()) });
        Effect { repaint: true, ..Effect::default() }
    }

    /// The home folder Trust Parent Folder is never offered for, in tests instead of the user's own.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn set_home_for_test(&mut self, home: PathBuf) { self.home = Some(home); }
}

#[cfg(test)]
mod tests {
    use super::{offered_parent, own_folder};
    use std::path::Path;

    #[test]
    fn trust_parent_folder_is_never_offered_for_the_file_system_root_home_or_a_shared_folder() {
        let mine = |_: &Path| true;
        assert_eq!(offered_parent(Path::new("/srv"), None, mine), None, "the parent is /");
        assert_eq!(offered_parent(Path::new("/"), None, mine), None);
        assert_eq!(offered_parent(Path::new("/home/u/proj"), Some(Path::new("/home/u")), mine), None, "the parent is home");
        assert_eq!(offered_parent(Path::new("/home/u/work/proj"), Some(Path::new("/home/u")), mine).as_deref(), Some(Path::new("/home/u/work")));
        assert_eq!(offered_parent(Path::new("/home/u"), Some(Path::new("/home/u")), mine), None, "home opened: /home holds every account's home");
        assert_eq!(offered_parent(Path::new("/Users/u"), Some(Path::new("/Users/u")), mine), None);
        // A folder every account can write to (as /tmp), or another account's, is not the user's own.
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let (shared, private) = (d.path().join("shared"), d.path().join("private"));
        for f in [&shared, &private] { std::fs::create_dir_all(f.join("proj")).unwrap(); }
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let home = Some(d.path());
        assert_eq!(offered_parent(&shared.join("proj"), home, |f| own_folder(f, home)), None, "world-writable");
        assert_eq!(offered_parent(&private.join("proj"), home, |f| own_folder(f, home)).as_deref(), Some(private.as_path()));
        let root_owned = semantics_owner(Path::new("/")) != semantics_owner(d.path());
        if root_owned { assert!(!own_folder(Path::new("/usr"), home), "another account's folder"); }
    }

    fn semantics_owner(path: &Path) -> Option<u32> { crate::semantics::provider::owner_of(path) }
}

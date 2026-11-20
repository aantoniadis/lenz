//! Folder trust (Restricted mode): Lenz runs no code from an opened folder until the user trusts it. The first time a
//! language server or formatter that would run the project's code is held back in a folder never asked about, one
//! dialog asks (Trust, Trust Parent Folder or Stay Restricted), naming what would run. The answer is kept privately in
//! providers.json (trusted or declined), so a folder is asked about once; the Restricted status item and Folder Trust…
//! open the dialog again. Trust decisions are made on paths with their links resolved.
use super::*;
use semantics::provider::{Trust, RESTRICTED};

/// The dialog's title, the same in both frontends.
pub const TITLE: &str = "Do you trust the authors of the files in this folder?";

/// An answer to the folder-trust dialog. Enter and Escape choose [`TrustChoice::StayRestricted`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustChoice {
    StayRestricted,
    Trust,
    TrustParent,
}

/// The folder-trust dialog: the opened folder, with its links resolved, the folder holding it (unless that is the file
/// system root or the home folder), and what would run there, in the order it was held back.
#[derive(Clone, Debug)]
pub(crate) struct TrustPrompt {
    folder: PathBuf,
    real: PathBuf,
    parent: Option<PathBuf>,
    runs: Vec<&'static str>,
}

/// What Restricted mode keeps off, for the dialog the user opens rather than a held-back tool.
const KEPT_OFF: [&str; 2] = [
    "language servers that run the project's code: rust-analyzer (build scripts and proc macros), typescript-language-server (the project's TypeScript and its plugins), Ruby LSP (Bundler and add-ons), Eclipse JDT LS and Kotlin LSP (Maven and Gradle builds) and gopls (the go command)",
    "the project's own programs: typescript-go's node_modules/.bin/tsc, a configured Python environment's interpreter, ruff and black, and the erb_lint and haml-lint formatters",
];

/// The folder Trust Parent Folder would trust for folder `real`: its parent, but never the file system root nor the
/// home folder `home`, which hold nearly everything.
fn offered_parent(real: &Path, home: Option<&Path>) -> Option<PathBuf> {
    real.parent().filter(|d| d.parent().is_some() && Some(*d) != home).map(Path::to_owned)
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

    /// A tool that would run the project's code (`runs` says what) was held back: the first time in an opened folder
    /// never asked about, the dialog asks; while it is open, it says this too. Never asked with no folder open, or
    /// for a folder already answered.
    pub(super) fn want_trust(&mut self, runs: &'static str) {
        if self.folder_trust() != Some(Trust::Unasked) { return; }
        match self.state.trust_prompt.as_mut() {
            Some(p) => if !p.runs.contains(&runs) { p.runs.push(runs); },
            None => self.open_trust_prompt(Some(runs)),
        }
    }

    /// Open the dialog for the open folder (Folder Trust…, the Restricted status item), or for a held-back tool.
    pub(super) fn open_trust_prompt(&mut self, runs: Option<&'static str>) {
        let (Some(folder), Some(real)) = (self.state.project.as_ref().map(|p| p.root().to_owned()), self.project_real.clone()) else { return };
        let home = self.home.as_deref().map(|h| std::fs::canonicalize(h).unwrap_or_else(|_| h.to_owned()));
        let parent = offered_parent(&real, home.as_deref());
        self.state.trust_prompt = Some(TrustPrompt { folder, real, parent, runs: runs.into_iter().collect() });
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
    use super::offered_parent;
    use std::path::Path;

    #[test]
    fn trust_parent_folder_is_never_offered_for_the_file_system_root_or_home() {
        assert_eq!(offered_parent(Path::new("/srv"), None), None, "the parent is /");
        assert_eq!(offered_parent(Path::new("/"), None), None);
        assert_eq!(offered_parent(Path::new("/home/u/proj"), Some(Path::new("/home/u"))), None, "the parent is home");
        assert_eq!(offered_parent(Path::new("/home/u/work/proj"), Some(Path::new("/home/u"))).as_deref(), Some(Path::new("/home/u/work")));
        assert_eq!(offered_parent(Path::new("/home/u"), Some(Path::new("/home/u"))).as_deref(), Some(Path::new("/home")), "home itself is opened: its parent is offered");
    }
}

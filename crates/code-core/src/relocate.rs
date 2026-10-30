//! Renaming and moving project entries from the explorer: where an entry goes for a typed name or folder. The
//! session checks the disk and the documents before anything moves, and both adapters' prompts ask the same
//! functions, so neither can drift. A move changes paths only; references in other files are not updated.

use crate::create;

/// What the explorer does with its highlighted entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A new name in the same folder.
    Rename,
    /// The same name in another folder of the project.
    Move,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rename => "Rename",
            Self::Move => "Move",
        }
    }
    /// What the prompt asks for.
    pub fn asks(self) -> &'static str {
        match self {
            Self::Rename => "the new name",
            Self::Move => "the destination folder (project-relative; empty for the root)",
        }
    }
}

/// The entry's folder (`""` at the root) and its name.
pub fn split(rel: &str) -> (&str, &str) { rel.rsplit_once('/').unwrap_or(("", rel)) }

/// What the prompt starts with: the current name, or the current folder.
pub fn start(kind: Kind, from: &str) -> String {
    match kind {
        Kind::Rename => split(from).1.to_string(),
        Kind::Move => split(from).0.to_string(),
    }
}

/// Where `from` goes for the typed `input` (project-relative, `/`-separated), or why it cannot go anywhere for it.
/// Nothing on disk is consulted here.
pub fn destination(kind: Kind, from: &str, input: &str) -> Result<String, String> {
    if from.is_empty() || !create::is_relative(from) { return Err("Choose a file or folder of the open project".into()); }
    let (dir, name) = split(from);
    let to = match kind {
        Kind::Rename => {
            let input = input.trim();
            if let Some(why) = create::name_refusal(input) { return Err(why); }
            create::relative(dir, input)
        }
        Kind::Move => {
            let folder = input.trim().trim_matches('/');
            if let Some(why) = create::dir_refusal(folder) { return Err(why); }
            if folder == from || folder.starts_with(&format!("{from}/")) { return Err("A folder cannot move into itself".into()); }
            create::relative(folder, name)
        }
    };
    if to == from { return Err("That is where it already is".into()); }
    Ok(to)
}

/// `path` (project-relative) after `from` became `to`: itself, or an entry inside a moved folder. `None` when the
/// move does not touch it.
pub fn moved(path: &str, from: &str, to: &str) -> Option<String> {
    if path == from { return Some(to.to_string()); }
    path.strip_prefix(from).and_then(|rest| rest.strip_prefix('/')).map(|rest| format!("{to}/{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_keeps_the_folder_and_a_move_keeps_the_name() {
        assert_eq!(destination(Kind::Rename, "src/a.rs", " b.rs "), Ok("src/b.rs".into()));
        assert_eq!(destination(Kind::Rename, "a.rs", "b.rs"), Ok("b.rs".into()));
        assert_eq!(destination(Kind::Move, "src/a.rs", "lib/"), Ok("lib/a.rs".into()));
        assert_eq!(destination(Kind::Move, "src/a.rs", ""), Ok("a.rs".into()));
        assert_eq!(destination(Kind::Move, "src", "other/deep"), Ok("other/deep/src".into()));
        assert_eq!((start(Kind::Rename, "src/a.rs"), start(Kind::Move, "src/a.rs")), ("a.rs".into(), "src".into()));
    }

    #[test]
    fn impossible_destinations_are_refused_before_the_disk() {
        assert!(destination(Kind::Rename, "src/a.rs", "x/y").unwrap_err().contains("path separator"));
        assert_eq!(destination(Kind::Rename, "src/a.rs", "a.rs").unwrap_err(), "That is where it already is");
        assert_eq!(destination(Kind::Move, "src/a.rs", "src").unwrap_err(), "That is where it already is");
        assert_eq!(destination(Kind::Move, "src", "src/inner").unwrap_err(), "A folder cannot move into itself");
        assert_eq!(destination(Kind::Move, "src", "src").unwrap_err(), "A folder cannot move into itself");
        assert!(destination(Kind::Move, "src/a.rs", "../out").is_err());
        assert_eq!(destination(Kind::Move, "src/a.rs", "/lib/"), Ok("lib/a.rs".into()), "slashes around a folder are the project's");
        assert!(destination(Kind::Rename, "", "b").is_err());
        // A sibling whose name only starts like the folder is not inside it.
        assert_eq!(destination(Kind::Move, "src", "src2"), Ok("src2/src".into()));
    }

    #[test]
    fn entries_inside_a_moved_folder_follow_it() {
        assert_eq!(moved("src/a.rs", "src", "lib"), Some("lib/a.rs".into()));
        assert_eq!(moved("src", "src", "lib"), Some("lib".into()));
        assert_eq!(moved("src2/a.rs", "src", "lib"), None);
        assert_eq!(moved("other.rs", "src", "lib"), None);
    }
}

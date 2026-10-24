//! New files and folders in the project explorer: the rules a name and its folder must meet. The session checks
//! them before anything is created, and both adapters' prompts ask the same function, so neither can drift.

use std::path::{Component, Path};

/// What New File or New Folder creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

impl Kind {
    /// The word messages and prompts use: "New file", "New folder".
    pub fn label(self) -> &'static str {
        match self {
            Self::File => "New file",
            Self::Folder => "New folder",
        }
    }
}

/// The most bytes a name may take on the file systems Lenz supports.
pub const MAX_NAME: usize = 255;

/// Why `name` is not a name Lenz creates (`None`: it is one), whatever folder it is for.
pub fn name_refusal(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("Enter a name".into());
    }
    if name == "." || name == ".." {
        return Some(format!("\"{name}\" cannot be a name"));
    }
    if name.contains('/') || name.contains('\\') {
        return Some("A name cannot hold a path separator; create the folder first".into());
    }
    if name.chars().any(char::is_control) {
        return Some("A name cannot hold control characters".into());
    }
    if name.len() > MAX_NAME {
        return Some(format!("A name can be at most {MAX_NAME} bytes"));
    }
    None
}

/// Whether `dir` is a folder of the project: relative, `/`-separated, without `.` or `..` (`""` is the root).
pub fn is_relative(dir: &str) -> bool {
    dir.is_empty() || (!Path::new(dir).is_absolute() && Path::new(dir).components().all(|c| matches!(c, Component::Normal(_))))
}

/// Why `dir` is not a folder Lenz creates in (`None`: it is one).
pub fn dir_refusal(dir: &str) -> Option<String> {
    (!is_relative(dir)).then(|| "Choose a folder of the open project".to_string())
}

/// `dir`'s path to `name`, `/`-separated (`""` for the root).
pub fn relative(dir: &str, name: &str) -> String {
    if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_one_path_segment_with_no_control_characters() {
        for good in ["main.rs", ".env", "my file.txt", "ünicode", "..."] {
            assert_eq!(name_refusal(good), None, "{good}");
        }
        assert_eq!(name_refusal("").as_deref(), Some("Enter a name"));
        assert_eq!(name_refusal("   ").as_deref(), Some("Enter a name"));
        assert_eq!(name_refusal("..").as_deref(), Some("\"..\" cannot be a name"));
        assert!(name_refusal("a/b").unwrap().contains("path separator"));
        assert!(name_refusal("a\\b").unwrap().contains("path separator"));
        assert!(name_refusal("a\nb").unwrap().contains("control"));
        assert!(name_refusal(&"a".repeat(MAX_NAME + 1)).unwrap().contains("at most 255"));
    }

    #[test]
    fn folders_are_project_relative_paths() {
        for good in ["", "src", "src/ui"] {
            assert_eq!(dir_refusal(good), None, "{good}");
        }
        for bad in ["/etc", "../out", "src/../etc", "./src"] {
            assert!(dir_refusal(bad).is_some(), "{bad}");
        }
        assert_eq!(relative("", "a.rs"), "a.rs");
        assert_eq!(relative("src/ui", "a.rs"), "src/ui/a.rs");
    }
}

//! Validation shared by the reader's typed Open File / Open Folder prompts. No document or file is changed here.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Self::File => "Open File",
            Self::Folder => "Open Folder",
        }
    }
    pub fn validate(self, path: PathBuf) -> Result<PathBuf, String> {
        let exists = match self {
            Self::File => path.is_file(),
            Self::Folder => path.is_dir(),
        };
        if exists {
            Ok(path)
        } else {
            Err(format!(
                "Choose an existing {}: {}",
                match self {
                    Self::File => "file",
                    Self::Folder => "folder",
                },
                path.display()
            ))
        }
    }
    pub fn resolve(self, text: &str, cwd: &Path) -> Result<PathBuf, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("Enter a path to open".into());
        }
        let path = match text.strip_prefix("~/").zip(std::env::var_os("HOME")) {
            Some((rest, home)) => PathBuf::from(home).join(rest),
            None => cwd.join(text),
        };
        self.validate(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_paths_keep_the_dialog_open_on_wrong_kind_or_missing_input() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("with spaces.txt");
        std::fs::write(&file, "unchanged\n").unwrap();
        assert_eq!(
            Kind::File.resolve("with spaces.txt", dir.path()),
            Ok(file.clone())
        );
        assert_eq!(
            Kind::Folder.resolve(".", dir.path()),
            Ok(dir.path().join("."))
        );
        assert!(Kind::Folder.resolve("with spaces.txt", dir.path()).is_err());
        assert!(Kind::File.resolve(".", dir.path()).is_err());
        assert!(Kind::File.resolve("missing", dir.path()).is_err());
        assert!(Kind::Folder.resolve("", dir.path()).is_err());
        assert_eq!(std::fs::read(file).unwrap(), b"unchanged\n");
    }
}

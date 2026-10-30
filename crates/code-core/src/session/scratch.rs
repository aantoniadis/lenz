//! Durable personal files use the ordinary retained document, Save and recovery paths.
use super::*;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

impl Session {
    pub fn scratch_directory(&self) -> &Path { &self.scratch_dir }

    fn prepare_scratch_directory(&self) -> Result<(), String> {
        match std::fs::symlink_metadata(&self.scratch_dir) {
            Ok(m) if !m.file_type().is_dir() => return Err("The personal scratch folder must be a real directory, not a symbolic link".into()),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
            Err(e) => return Err(e.to_string()),
        }
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.scratch_dir).map_err(|e| e.to_string())?;
        if !std::fs::symlink_metadata(&self.scratch_dir).map_err(|e| e.to_string())?.file_type().is_dir() {
            return Err("The personal scratch folder must be a real directory, not a symbolic link".into());
        }
        Ok(())
    }

    /// Create lazily, when the user opens a scratch picker; never during ordinary startup.
    pub fn scratch_files(&self) -> Result<Vec<PathBuf>, String> {
        self.prepare_scratch_directory()?;
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.scratch_dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_type().map_err(|e| e.to_string())?.is_file() { files.push(entry.path()); }
        }
        files.sort();
        Ok(files)
    }

    pub(super) fn new_scratch(&mut self, name: String) -> Effect {
        let result = (|| {
            if let Some(why) = crate::create::name_refusal(&name) { return Err(why); }
            self.prepare_scratch_directory()?;
            let path = self.scratch_dir.join(name);
            std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).map_err(|e| e.to_string())?;
            Ok(path)
        })();
        match result {
            Ok(path) => self.command(Command::OpenRetained(path)),
            Err(why) => self.command(Command::Notice(format!("New scratch file: {why}. Existing files were not changed."))),
        }
    }

    pub(super) fn open_scratch(&mut self, path: PathBuf) -> Effect {
        if let Err(why) = self.prepare_scratch_directory() { return self.command(Command::Notice(format!("Scratch files: {why}"))); }
        if path.parent() != Some(self.scratch_dir.as_path()) || !std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_file()) {
            return self.command(Command::Notice("Choose an existing file in the personal scratch folder".into()));
        }
        self.command(Command::OpenRetained(path))
    }
}

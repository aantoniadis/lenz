//! Saved-byte explorer copies, staged privately and published without replacing an entry.
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

/// Copies a regular file or folder's saved bytes and permissions. Any failure before publication
/// leaves the destination absent; an existing destination, including a dangling link, is never replaced.
/// Cancellation is checked between directory entries and each 64 KiB chunk.
// shortcut: ancestor checks cannot defeat hostile concurrent path replacement; use descriptor-relative
// operations if explorer operations must support an actively hostile filesystem namespace.
pub fn copy(source: &Path, target: &Path, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    let source_meta = checked_path(source)?;
    let parent = target.parent().filter(|_| target.file_name().is_some())
        .ok_or_else(|| "The copy destination must name a new entry".to_string())?;
    let parent_meta = checked_path(parent)?;
    if !parent_meta.is_dir() { return Err("The copy destination's parent is not a folder".into()); }
    if target.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("The copy destination must not contain parent-folder steps".into());
    }
    if source_meta.is_dir() && target.starts_with(source) {
        return Err("A folder cannot be copied into itself".into());
    }
    match fs::symlink_metadata(target) {
        Ok(_) => return Err(format!("{} already exists; nothing was copied", target.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
        Err(e) => return Err(format!("{} cannot be checked: {e}", target.display())),
    }
    let stage = Stage::new(parent)?;
    let payload = stage.0.join("entry");
    let mut buffer = [0; 64 * 1024];
    copy_entry(source, &payload, cancel, &mut buffer, 0)?;
    check_cancel(cancel)?;
    if !unchanged(&source_meta, &checked_path(source)?) || !same_entry(&parent_meta, &checked_path(parent)?) {
        return Err("The source or destination folder changed while copying; nothing was copied".into());
    }
    lenz_content::write::rename_noclobber(&payload, target)
        .map_err(|e| format!("{} could not be created; nothing was copied: {e}", target.display()))
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) { Err("Copy cancelled; nothing was copied".into()) } else { Ok(()) }
}

/// Reject links in every existing component, including an ancestor of the selected entry.
fn checked_path(path: &Path) -> Result<fs::Metadata, String> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("Copy paths must be absolute and must not contain parent-folder steps".into());
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        let meta = fs::symlink_metadata(&current).map_err(|e| format!("{} cannot be read: {e}", current.display()))?;
        if meta.is_symlink() { return Err(format!("Copying through symbolic links is not supported: {}", current.display())); }
    }
    fs::symlink_metadata(path).map_err(|e| format!("{} cannot be read: {e}", path.display()))
}

fn same_entry(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.dev() == after.dev() && before.ino() == after.ino() && before.file_type() == after.file_type()
}

fn unchanged(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    same_entry(before, after) && before.len() == after.len() && before.modified().ok() == after.modified().ok()
        && before.ctime() == after.ctime() && before.ctime_nsec() == after.ctime_nsec()
}

fn copy_entry(source: &Path, target: &Path, cancel: &AtomicBool, buffer: &mut [u8], depth: usize) -> Result<(), String> {
    check_cancel(cancel)?;
    if depth > 256 { return Err("The copied folder is too deeply nested (more than 256 folders)".into()); }
    let mut run = || -> Result<(), String> {
        let meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if meta.is_symlink() { return Err("Copying symbolic links is not supported".into()); }
        if meta.is_file() {
            let mut input = File::open(source).map_err(|e| e.to_string())?;
            if !unchanged(&meta, &input.metadata().map_err(|e| e.to_string())?) {
                return Err("The source changed while opening it".into());
            }
            let mut output = OpenOptions::new().write(true).create_new(true).mode(0o600).open(target).map_err(|e| e.to_string())?;
            loop {
                check_cancel(cancel)?;
                let count = input.read(buffer).map_err(|e| e.to_string())?;
                if count == 0 { break; }
                output.write_all(&buffer[..count]).map_err(|e| e.to_string())?;
            }
            if !unchanged(&meta, &input.metadata().map_err(|e| e.to_string())?) {
                return Err("The source changed while copying it".into());
            }
            output.set_permissions(meta.permissions()).map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
        } else if meta.is_dir() {
            fs::DirBuilder::new().mode(0o700).create(target).map_err(|e| e.to_string())?;
            for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
                check_cancel(cancel)?;
                let entry = entry.map_err(|e| e.to_string())?;
                copy_entry(&entry.path(), &target.join(entry.file_name()), cancel, buffer, depth + 1)?;
            }
            let directory = File::open(target).map_err(|e| e.to_string())?;
            directory.set_permissions(meta.permissions()).map_err(|e| e.to_string())?;
            directory.sync_all().map_err(|e| e.to_string())?;
        } else {
            return Err("Only a regular file or folder can be copied".into());
        }
        if !unchanged(&meta, &fs::symlink_metadata(source).map_err(|e| e.to_string())?) {
            return Err("The source changed while copying it".into());
        }
        Ok(())
    };
    run().map_err(|e| format!("{} could not be copied: {e}", source.display()))
}

/// Owns only this operation's exclusively created private staging folder.
struct Stage(PathBuf);
impl Stage {
    fn new(parent: &Path) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        for _ in 0..100 {
            let path = parent.join(format!(".lenz-copy-{}-{now}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {},
                Err(e) => return Err(format!("The copy staging folder cannot be created: {e}")),
            }
        }
        Err("A private copy staging folder could not be reserved".into())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        // Copied folders may already have read-only permissions when a later entry fails.
        fn writable(path: &Path) {
            if !fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) { return; }
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() { writable(&entry.path()); }
            }
        }
        writable(&self.0);
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::{PermissionsExt, symlink}};

    #[test]
    fn explorer_copy_preserves_binary_bytes_permissions_and_source() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        let target = d.path().join("target");
        fs::write(&source, b"\0\xffsaved\r\n").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o751)).unwrap();
        copy(&source, &target, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"\0\xffsaved\r\n");
        assert_eq!(fs::read(&target).unwrap(), b"\0\xffsaved\r\n");
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o751);
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
    }

    #[test]
    fn explorer_copy_folders_keep_empty_children_and_permissions() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        let target = d.path().join("target");
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        fs::write(source.join("nested/file"), b"binary\0\xff").unwrap();
        fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o555)).unwrap();
        copy(&source, &target, &AtomicBool::new(false)).unwrap();
        assert!(target.join("nested/empty").is_dir());
        assert_eq!(fs::read(target.join("nested/file")).unwrap(), b"binary\0\xff");
        assert_eq!(fs::metadata(target.join("nested")).unwrap().permissions().mode() & 0o777, 0o555);
        fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(target.join("nested"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn explorer_copy_never_replaces_existing_entries_including_dangling_links() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        let target = d.path().join("target");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"keep").unwrap();
        assert!(copy(&source, &target, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        fs::remove_file(&target).unwrap();
        symlink("missing", &target).unwrap();
        assert!(copy(&source, &target, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read_link(&target).unwrap(), Path::new("missing"));
        fs::remove_file(&target).unwrap();
        fs::create_dir(&target).unwrap();
        assert!(copy(&source, &target, &AtomicBool::new(false)).is_err());
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 2);
    }

    #[test]
    fn explorer_copy_failed_folder_is_cleaned_without_changing_any_source() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        let target = d.path().join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file"), b"keep").unwrap();
        symlink("file", source.join("nested/link")).unwrap();
        assert!(copy(&source, &target, &AtomicBool::new(false)).unwrap_err().contains("symbolic"));
        assert!(!target.exists());
        assert_eq!(fs::read(source.join("nested/file")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn explorer_copy_refuses_symlinked_sources_and_destination_ancestors() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("source"), b"keep").unwrap();
        let link = d.path().join("link");
        symlink(&real, &link).unwrap();
        assert!(copy(&link.join("source"), &d.path().join("target"), &AtomicBool::new(false)).is_err());
        assert!(copy(&real.join("source"), &link.join("target"), &AtomicBool::new(false)).is_err());
        assert!(copy(&link, &d.path().join("target"), &AtomicBool::new(false)).is_err());
        assert!(!real.join("target").exists());
        assert_eq!(fs::read(real.join("source")).unwrap(), b"keep");
    }

    #[test]
    fn explorer_copy_refuses_self_descendants_and_cancellation() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"keep").unwrap();
        assert!(copy(&source, &source.join("child"), &AtomicBool::new(false)).is_err());
        assert!(copy(&source, &source, &AtomicBool::new(false)).is_err());
        let target = d.path().join("target");
        assert!(copy(&source, &target, &AtomicBool::new(true)).unwrap_err().contains("cancel"));
        assert!(!target.exists());
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&source).unwrap().count(), 1);
    }

    #[test]
    fn explorer_copy_cleans_owned_read_only_partial_folders_on_cancellation() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file"), b"keep").unwrap();
        fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o500)).unwrap();
        let cancel = AtomicBool::new(false);
        let stage = Stage::new(d.path()).unwrap();
        let staged = stage.0.clone();
        copy_entry(&source, &staged.join("partial"), &cancel, &mut [0; 1024], 0).unwrap();
        cancel.store(true, Ordering::Relaxed);
        assert!(copy_entry(&source, &staged.join("next"), &cancel, &mut [0; 1024], 0).unwrap_err().contains("cancel"));
        drop(stage);
        assert!(!staged.exists());
        assert_eq!(fs::read(source.join("nested/file")).unwrap(), b"keep");
        assert_eq!(fs::metadata(source.join("nested")).unwrap().permissions().mode() & 0o777, 0o500);
        fs::set_permissions(source.join("nested"), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn explorer_copy_refuses_special_files_without_opening_them() {
        let d = tempfile::tempdir().unwrap();
        let source = d.path().join("socket");
        let _socket = std::os::unix::net::UnixListener::bind(&source).unwrap();
        assert!(copy(&source, &d.path().join("target"), &AtomicBool::new(false)).unwrap_err().contains("regular file"));
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }
}

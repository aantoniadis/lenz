//! Recoverable deletion through the freedesktop.org Trash specification: an entry moves into the trash of its own
//! file system with a `.trashinfo` record, so the desktop's trash (and Lenz) can put it back. Where no such trash can
//! be used, nothing moves and the caller is told recoverable deletion is unavailable there.
use std::{
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// How the explorer deletes an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Removal {
    /// Into the trash, from where it can be restored.
    Trash,
    /// Gone at once; asked for explicitly and confirmed.
    Permanent,
}

impl Removal {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trash => "Delete",
            Self::Permanent => "Delete Permanently",
        }
    }
}

/// An entry this session moved to a trash: where it was, and where it and its record are now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trashed {
    pub original: PathBuf,
    pub item: PathBuf,
    pub info: PathBuf,
}

/// The user's home trash: `$XDG_DATA_HOME/Trash`, else `~/.local/share/Trash`.
pub fn home_trash() -> Option<PathBuf> {
    let absolute = |v: std::ffi::OsString| Some(PathBuf::from(v)).filter(|p| p.is_absolute());
    std::env::var_os("XDG_DATA_HOME").and_then(absolute).map(|d| d.join("Trash"))
        .or_else(|| std::env::var_os("HOME").and_then(absolute).map(|h| h.join(".local/share/Trash")))
}

/// Moves `path` (a file or a folder, not followed if it is a link) into the trash of its file system, preferring
/// `home` when it is on the same device.
pub fn trash(path: &Path, home: Option<&Path>) -> Result<Trashed, String> {
    if !cfg!(target_os = "linux") { return Err("Recoverable deletion uses the freedesktop trash, which this platform does not have".into()); }
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
    let name = path.file_name().ok_or_else(|| format!("{} has no name to keep in the trash", path.display()))?.to_os_string();
    let (dir, topdir) = trash_dir(path, meta.dev(), home)?;
    let (files, info) = (dir.join("files"), dir.join("info"));
    for sub in [&files, &info] { private_dir(sub)?; }
    let recorded = match &topdir {
        Some(top) => path.strip_prefix(top).map_err(|_| "The trash's top folder does not hold the entry".to_string())?.to_path_buf(),
        None => path.to_path_buf(),
    };
    let record = format!("[Trash Info]\nPath={}\nDeletionDate={}\n", encode(&recorded), deletion_date());
    for n in 1..=1000 {
        let mut candidate = name.clone();
        if n > 1 { candidate.push(format!(".{n}")); }
        let item = files.join(&candidate);
        let mut info_name = candidate.clone();
        info_name.push(".trashinfo");
        let record_file = info.join(info_name);
        // The record is created first and exclusively: it reserves the name, as the specification requires.
        let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&record_file) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("The trash record cannot be written: {e}")),
        };
        if let Err(e) = file.write_all(record.as_bytes()).and_then(|()| file.sync_all()) {
            let _ = std::fs::remove_file(&record_file);
            return Err(format!("The trash record cannot be written: {e}"));
        }
        match lenz_content::write::rename_noclobber(path, &item) {
            Ok(()) => return Ok(Trashed { original: path.to_path_buf(), item, info: record_file }),
            Err(e) => {
                let _ = std::fs::remove_file(&record_file);
                if e.kind() == std::io::ErrorKind::AlreadyExists { continue; }
                return Err(format!("{} could not be moved to the trash: {e}", path.display()));
            }
        }
    }
    Err("The trash holds too many entries of this name".into())
}

/// Puts a trashed entry back where it was. Nothing is replaced: an entry now at that path refuses the restore.
pub fn restore(trashed: &Trashed) -> Result<(), String> {
    if std::fs::symlink_metadata(&trashed.item).is_err() { return Err("It is no longer in the trash".into()); }
    match lenz_content::write::rename_noclobber(&trashed.item, &trashed.original) {
        Ok(()) => {
            let _ = std::fs::remove_file(&trashed.info);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(format!("{} exists again; nothing was restored", trashed.original.display())),
        Err(e) => Err(format!("{} could not be restored: {e}", trashed.original.display())),
    }
}

/// The trash for an entry on device `dev`, and the top folder its record is relative to (`None`: the home trash).
fn trash_dir(path: &Path, dev: u64, home: Option<&Path>) -> Result<(PathBuf, Option<PathBuf>), String> {
    if let Some(home) = home {
        // The home trash is made when first used, as the specification allows.
        if private_dir(home).is_ok() && std::fs::metadata(home).is_ok_and(|m| m.dev() == dev) { return Ok((home.to_path_buf(), None)); }
    }
    let uid = std::fs::metadata("/proc/self").map_err(|e| format!("Recoverable deletion is unavailable: {e}"))?.uid();
    let top = topdir(path, dev);
    let shared = top.join(".Trash");
    // An administrator's shared trash counts only when it is a real sticky folder, never a link.
    if std::fs::symlink_metadata(&shared).is_ok_and(|m| m.is_dir() && m.mode() & 0o1000 != 0) {
        let own = shared.join(uid.to_string());
        if private_dir(&own).is_ok() { return Ok((own, Some(top))); }
    }
    let own = top.join(format!(".Trash-{uid}"));
    match private_dir(&own) {
        Ok(()) if std::fs::symlink_metadata(&own).is_ok_and(|m| m.uid() == uid) => Ok((own, Some(top))),
        _ => Err(format!("Recoverable deletion is unavailable on the file system of {}: no usable trash there", path.display())),
    }
}

/// The top folder of the mount holding `path`: its highest ancestor on the same device.
fn topdir(path: &Path, dev: u64) -> PathBuf {
    let mut top = path.parent().unwrap_or(path).to_path_buf();
    while let Some(up) = top.parent() {
        if std::fs::metadata(up).map_or(true, |m| m.dev() != dev) { break; }
        top = up.to_path_buf();
    }
    top
}

/// A folder that exists, is not a link, and only its owner may enter.
fn private_dir(dir: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(format!("{} is not a folder", dir.display())),
        Err(_) => {
            std::fs::create_dir_all(dir).map_err(|e| format!("{} cannot be made: {e}", dir.display()))?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
        }
    }
}

/// A path as the specification stores it: URI-escaped bytes, `/` kept.
fn encode(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().iter().map(|&b| match b {
        b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}

/// `YYYY-MM-DDThh:mm:ss` for now, in UTC: the specification asks for local time, which needs a time-zone database
/// this crate does not hold. Restoring does not read it.
fn deletion_date() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trashed_file_has_a_record_and_comes_back_without_replacing_anything() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("data/Trash");
        let file = d.path().join("proj/a b%.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"keep\n").unwrap();
        let trashed = trash(&file, Some(&home)).unwrap();
        assert!(!file.exists());
        assert_eq!(trashed.item, home.join("files/a b%.txt"));
        assert_eq!(std::fs::read(&trashed.item).unwrap(), b"keep\n");
        let record = std::fs::read_to_string(&trashed.info).unwrap();
        assert!(record.starts_with("[Trash Info]\nPath=") && record.contains("/proj/a%20b%25.txt\nDeletionDate="), "{record}");
        assert_eq!(std::fs::metadata(home.join("files")).unwrap().permissions().mode() & 0o777, 0o700);
        // A second entry of the same name gets its own slot.
        std::fs::write(&file, b"second\n").unwrap();
        let again = trash(&file, Some(&home)).unwrap();
        assert_eq!(again.item, home.join("files/a b%.txt.2"));
        // Something new at the old path is never replaced by a restore.
        std::fs::write(&file, b"new\n").unwrap();
        assert!(restore(&trashed).unwrap_err().contains("exists again"));
        assert_eq!(std::fs::read(&file).unwrap(), b"new\n");
        std::fs::remove_file(&file).unwrap();
        restore(&trashed).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"keep\n");
        assert!(!trashed.info.exists() && !trashed.item.exists());
        assert!(restore(&trashed).unwrap_err().contains("no longer in the trash"));
    }

    #[test]
    fn a_folder_moves_whole_and_dates_are_well_formed() {
        let d = tempfile::tempdir().unwrap();
        let home = d.path().join("Trash");
        let dir = d.path().join("proj/src");
        std::fs::create_dir_all(dir.join("deep")).unwrap();
        std::fs::write(dir.join("deep/x.rs"), b"x").unwrap();
        let trashed = trash(&dir, Some(&home)).unwrap();
        assert_eq!(std::fs::read(trashed.item.join("deep/x.rs")).unwrap(), b"x");
        restore(&trashed).unwrap();
        assert_eq!(std::fs::read(dir.join("deep/x.rs")).unwrap(), b"x");
        let date = deletion_date();
        assert_eq!(date.len(), 19);
        assert!(date.starts_with("20") && &date[10..11] == "T", "{date}");
    }
}

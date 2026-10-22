//! Open in another app: `o` hands a file to the user's editor at the line, `shift-o` to the system default.
//! Resolution is shared with Lenz Code ([`lenz_content::open`]); this adds the settings file's editor and read-only
//! copies of revision and index Sides.

pub use lenz_content::open::*;

use std::{
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};

/// Copies of revision and index Sides older than this are swept at startup.
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Resolves `o` (`default_app` false) or `shift-o` (true) for `target`, with the settings file's editor.
pub fn plan(host: &Host, target: &Target, default_app: bool) -> Plan {
    plan_with(host, target, crate::settings::get().tools.editor.as_deref(), default_app)
}

/// `LENZ_EDITOR`, else `$VISUAL`, else `$EDITOR`, else the settings file's `editor`, split into program and arguments.
pub fn editor(host: &Host) -> Option<Vec<String>> {
    editor_with(host, crate::settings::get().tools.editor.as_deref())
}

/// Resolve a comparison Side once for either frontend, including read-only revision copies.
pub fn comparison_plan(set:&crate::changeset::ChangeSet,index:usize,side:crate::view::Side,binary:bool,line:Option<usize>,column:Option<usize>,default_app:bool)->Result<Plan,String>{
    use crate::changeset::Content;
    let c=set.comparisons.get(index).ok_or("Comparison no longer available")?;
    let(content,path)=if side==crate::view::Side::Left{(&c.left,&c.old_path)}else{(&c.right,&c.path)};
    let host=Host::current();
    let file=match content {
        Content::Empty=>return Err("This Side doesn't exist".into()),Content::File(p)=>std::path::absolute(p).unwrap_or_else(|_|p.clone()),
        other=>{let bytes=set.read(other)?;let key=match other{Content::Blob(name)if name.len()>=40&&name.bytes().all(|b|b.is_ascii_hexdigit())=>name[..10].to_string(),_=>format!("{:016x}",bytes.iter().fold(0xcbf29ce484222325u64,|h,&b|(h^b as u64).wrapping_mul(0x100000001b3)))};
            let root=cache_dir(&host).ok_or("No cache directory")?;copy(&root,&key,path,&bytes).map_err(|e|format!("Cannot open: {e}"))?}
    };
    Ok(plan(&host,&Target{file:&file,binary,line,column,cwd:set.git_root()},default_app))
}

/// `<cache>/lenz/open`: `$XDG_CACHE_HOME` or `~/.cache`; macOS `~/Library/Caches`.
pub fn cache_dir(host: &Host) -> Option<PathBuf> {
    let home = || host.get("HOME").map(PathBuf::from);
    let cache = if host.macos {
        home()?.join("Library/Caches")
    } else {
        host.get("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| Some(home()?.join(".cache")))?
    };
    Some(cache.join("lenz/open"))
}

/// A read-only copy of a revision or index Side at `<root>/<short_sha>/<path>`, keeping its name so apps recognise it.
/// An identical copy is reused. `short_sha` names the version: a commit's, or the blob's for the index.
pub fn copy(root: &Path, short_sha: &str, path: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    let rel = Path::new(short_sha).join(path);
    // git paths are relative and `..`-free; refuse anything that could escape the cache
    if !rel.components().all(|c| matches!(c, Component::Normal(_))) || short_sha.contains('/') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("lenz: bad path {short_sha}/{path}")));
    }
    let dest = root.join(rel);
    if std::fs::read(&dest).is_ok_and(|old| old == bytes) {
        return Ok(dest);
    }
    std::fs::create_dir_all(dest.parent().expect("has a parent"))?;
    match std::fs::remove_file(&dest) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::write(&dest, bytes)?;
    let mut perms = std::fs::metadata(&dest)?.permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&dest, perms)?;
    Ok(dest)
}

/// Deletes copies under `root` last written more than [`MAX_AGE`] before `now`, then the emptied folders.
/// Best effort: run at startup, never on exit.
pub fn sweep(root: &Path, now: SystemTime) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            sweep(&path, now);
            let _ = std::fs::remove_dir(&path); // only succeeds once empty
        } else if meta.modified().is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > MAX_AGE)) {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn host(vars: &[(&str, &str)], path: &[&str], macos: bool) -> Host {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let path: Vec<String> = path.iter().map(|p| p.to_string()).collect();
        Host { var: Box::new(move |k| vars.get(k).cloned()), on_path: Box::new(move |p| path.iter().any(|q| q == p)), macos }
    }

    #[test]
    fn cache_dir_per_platform() {
        let linux = |vars: &[(&str, &str)]| cache_dir(&host(vars, &[], false));
        assert_eq!(linux(&[("HOME", "/h"), ("XDG_CACHE_HOME", "/x")]), Some(PathBuf::from("/x/lenz/open")));
        assert_eq!(linux(&[("HOME", "/h"), ("XDG_CACHE_HOME", "rel")]), Some(PathBuf::from("/h/.cache/lenz/open")));
        assert_eq!(linux(&[("HOME", "/h")]), Some(PathBuf::from("/h/.cache/lenz/open")));
        assert_eq!(linux(&[]), None);
        let mac = cache_dir(&host(&[("HOME", "/Users/a"), ("XDG_CACHE_HOME", "/x")], &[], true));
        assert_eq!(mac, Some(PathBuf::from("/Users/a/Library/Caches/lenz/open")));
    }

    #[test]
    fn copies_are_read_only_keep_their_name_and_are_reused() {
        let root = tempfile::tempdir().unwrap();
        let path = copy(root.path(), "abc1234", "src/a b.rs", b"one").unwrap();
        assert_eq!(path, root.path().join("abc1234/src/a b.rs"));
        assert_eq!(std::fs::read(&path).unwrap(), b"one");
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
        assert_eq!(copy(root.path(), "abc1234", "src/a b.rs", b"one").unwrap(), path);
        // the index's copy is keyed by blob, but a changed one is replaced anyway
        copy(root.path(), "abc1234", "src/a b.rs", b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert!(std::fs::metadata(&path).unwrap().permissions().readonly());
        for (sha, bad) in [("abc", "../escape"), ("abc", "/etc/passwd"), ("a/b", "x"), ("..", "x")] {
            assert!(copy(root.path(), sha, bad, b"").is_err(), "{sha} {bad}");
        }
    }

    #[test]
    fn sweep_deletes_copies_older_than_a_week() {
        let root = tempfile::tempdir().unwrap();
        let old = copy(root.path(), "old1234", "deep/x.rs", b"x").unwrap();
        let fresh = copy(root.path(), "new1234", "y.rs", b"y").unwrap();
        let written = std::fs::metadata(&fresh).unwrap().modified().unwrap();
        let file = std::fs::File::open(&old).unwrap();
        file.set_modified(written - MAX_AGE - Duration::from_secs(60)).unwrap();
        sweep(root.path(), written + Duration::from_secs(60));
        assert!(!old.exists());
        assert!(!root.path().join("old1234").exists(), "emptied folders go too");
        assert!(fresh.exists());
        sweep(root.path(), written + MAX_AGE + Duration::from_secs(1));
        assert!(!fresh.exists());
        assert!(root.path().exists());
    }
}

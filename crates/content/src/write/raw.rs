//! Guarded raw-byte replacement. Binary containers require a complete atomic replacement:
//! existing hard links, foreign ownership, or an unavailable sibling temp never fall back to an in-place write.
use super::*;

/// Atomically replace the exact file that supplied `snapshot`, without decoding or changing any bytes.
/// A symlink keeps its own entry and replaces its confirmed target. Like [`write_exact`], this is not a
/// filesystem compare-and-swap: a different process can still race the final guard and rename.
pub fn write_bytes_exact(
    snapshot: &Snapshot,
    expected: &TargetIdentity,
    bytes: &[u8],
) -> WriteOutcome {
    let mut source = bytes;
    write_bytes_exact_from(snapshot, expected, &mut source)
}

/// Streaming form of [`write_bytes_exact`]. `source` is read from its current position to EOF into an owned
/// sibling temp. Reads, hashing, and validation use fixed-size buffers; the original file is never rewritten
/// in place. A failure after replacement can leave the new file installed and is reported as Uncertain.
pub fn write_bytes_exact_from(
    snapshot: &Snapshot,
    expected: &TargetIdentity,
    source: &mut impl Read,
) -> WriteOutcome {
    #[cfg(any(test, feature = "write-testing"))]
    let _fault = readiness_fault::enter_path(&snapshot.path);
    use WriteOutcome::{Rejected, Saved, Uncertain};
    let path = &expected.canonical;
    let mut prepare = || -> Result<_, WriteError> {
        snapshot.check_repository()?;
        expected.verify(&snapshot.path)?;
        let mut original = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(path)
            .map_err(io_err("opening", &snapshot.path))?;
        let metadata = validate(snapshot, expected, &mut original, None)?;
        let mut temp = tempfile::Builder::new()
            .prefix(".lenz-")
            .suffix(".lenz~")
            .tempfile_in(parent(path))
            .map_err(io_err("creating atomic replacement for", path))?;
        let mode = metadata.mode() & 0o7777;
        if !prepare_atomic(temp.as_file(), &metadata, mode, source)
            .map_err(io_err("writing", path))?
        {
            return Err(WriteError::Io(
                "Atomic replacement cannot preserve this file's ownership and group".into(),
            ));
        }
        // Copying can clear set-id permission bits. Restore exactly the confirmed mode before the final fsync.
        temp.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))
            .map_err(io_err("setting permissions for", path))?;
        temp.as_file().sync_all().map_err(io_err("syncing", path))?;
        let replacement_len = temp.as_file().metadata().map_err(io_err("reading replacement for", path))?.len();
        let (hash, size) =
            digest(temp.as_file_mut(), replacement_len).map_err(io_err("reading replacement for", path))?;
        validate(snapshot, expected, &mut original, Some(&metadata))?;
        #[cfg(any(test, feature = "write-testing"))]
        readiness_fault::point(readiness_fault::Fault::BeforeReplace)
            .map_err(io_err("writing", path))?;
        Ok((temp, hash, size))
    };
    let (temp, hash, size) = match prepare() {
        Ok(prepared) => prepared,
        Err(error) => return Rejected(error),
    };
    if let Err(error) = fs::rename(temp.path(), path) {
        return Uncertain(io_err("writing", path)(error));
    }
    let installed = temp.into_file();
    if let Err(error) = File::open(parent(path)).and_then(|directory| directory.sync_all()) {
        return Uncertain(io_err("syncing directory for", path)(error));
    }
    #[cfg(any(test, feature = "write-testing"))]
    if let Err(error) = readiness_fault::point(readiness_fault::Fault::AfterReplace) {
        return Uncertain(io_err("reading", path)(error));
    }
    let metadata = match installed.metadata() {
        Ok(metadata) => metadata,
        Err(error) => return Uncertain(io_err("reading", path)(error)),
    };
    let target = TargetIdentity::from_metadata(path.clone(), &metadata);
    if let Err(error) = snapshot
        .check_repository()
        .and_then(|()| target.verify(&snapshot.path))
    {
        return Uncertain(error);
    }
    Saved {
        snapshot: Snapshot {
            hash: Some(hash),
            size,
            mtime: metadata.modified().ok(),
            // No text decoding has established that these new bytes can round-trip as text.
            lossy: true,
            ..snapshot.clone()
        },
        target,
    }
}

fn validate(
    snapshot: &Snapshot,
    expected: &TargetIdentity,
    original: &mut File,
    prepared: Option<&fs::Metadata>,
) -> Result<fs::Metadata, WriteError> {
    snapshot.check_repository()?;
    expected.verify(&snapshot.path)?;
    let metadata = original
        .metadata()
        .map_err(io_err("reading", &snapshot.path))?;
    if !metadata.is_file() || !expected.matches(&metadata) {
        return Err(WriteError::ChangedOnDisk);
    }
    if metadata.nlink() != 1 {
        return Err(WriteError::Io(
            "Atomic replacement cannot preserve a hard-linked file; no bytes were written".into(),
        ));
    }
    if prepared.is_some_and(|old| {
        old.mode() != metadata.mode() || old.uid() != metadata.uid() || old.gid() != metadata.gid()
    }) {
        return Err(WriteError::ChangedOnDisk);
    }
    if metadata.len() != snapshot.size { return Err(WriteError::ChangedOnDisk); }
    let (actual, size) = digest(original, snapshot.size).map_err(io_err("reading", &snapshot.path))?;
    if snapshot.hash != Some(actual) || snapshot.size != size {
        return Err(WriteError::ChangedOnDisk);
    }
    let after = original
        .metadata()
        .map_err(io_err("reading", &snapshot.path))?;
    if metadata.mode() != after.mode()
        || metadata.uid() != after.uid()
        || metadata.gid() != after.gid()
        || metadata.nlink() != after.nlink()
        || metadata.len() != after.len()
        || metadata.modified().ok() != after.modified().ok()
    {
        return Err(WriteError::ChangedOnDisk);
    }
    expected.verify(&snapshot.path)?;
    Ok(after)
}

fn digest(file: &mut File, expected_size: u64) -> io::Result<(u64, u64)> {
    file.rewind()?;
    // One extra byte proves growth without following a concurrent append indefinitely.
    let mut file = file.take(expected_size.saturating_add(1));
    let mut hash = DefaultHasher::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 << 10];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok((hash.finish(), size));
        }
        hash.write(&buffer[..count]);
        size = size
            .checked_add(count as u64)
            .ok_or_else(|| io::Error::other("file length overflow"))?;
    }
}

#[cfg(test)]
mod tests;

//! Descriptor-level bounds for archive reads, including files that grow after their initial stat.
use super::*;

/// Read one regular file with a hard byte cap and cancellation between chunks. Nonblocking open
/// prevents a symlink to a FIFO from waiting for a writer before its type can be rejected.
pub fn read_exact_bytes_bounded(path: &Path, max: u64, cancel: &dyn Fn() -> bool) -> Result<ExactRead, WriteError> {
    if cancel() { return Err(WriteError::Io("archive operation cancelled".into())); }
    let canonical = fs::canonicalize(path).map_err(io_err("reading", path))?;
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW).open(&canonical).map_err(io_err("reading", path))?;
    let metadata = file.metadata().map_err(io_err("reading", path))?;
    if !metadata.is_file() { return Err(WriteError::Io("archive source is not a regular file".into())); }
    if metadata.len() > max { return Err(limit()); }
    let bytes = read_bounded(&mut file, max, cancel)?;
    let after = file.metadata().map_err(io_err("reading", path))?;
    if after.len() != metadata.len() || after.modified().ok() != metadata.modified().ok() || bytes.len() as u64 != metadata.len() {
        return Err(WriteError::ChangedOnDisk);
    }
    let target = TargetIdentity::from_metadata(canonical, &metadata);
    target.verify(path)?;
    Ok(ExactRead { bytes, metadata, path: path.to_owned(), target })
}
fn limit() -> WriteError { WriteError::Io("archive exceeds the stored bytes limit".into()) }
fn read_bounded(reader: &mut impl Read, max: u64, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, WriteError> {
    let mut bytes = Vec::new();
    let mut chunk = [0; 64 << 10];
    loop {
        if cancel() { return Err(WriteError::Io("archive operation cancelled".into())); }
        let remaining = max.saturating_sub(bytes.len() as u64);
        let request = remaining.min(chunk.len() as u64).max(1) as usize;
        let n = match reader.read(&mut chunk[..request]) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            other => other.map_err(|e| WriteError::Io(format!("reading archive: {e}")))?,
        };
        if n == 0 { return Ok(bytes); }
        if n as u64 > remaining { return Err(limit()); }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_reader_rejects_infinite_or_growing_input_and_cancels_between_chunks() {
        let mut infinite = io::repeat(0);
        assert_eq!(read_bounded(&mut infinite, 65_537, &|| false).unwrap_err(), limit());
        let calls = std::cell::Cell::new(0);
        let cancel = || { calls.set(calls.get() + 1); calls.get() > 2 };
        assert!(read_bounded(&mut infinite, 10 << 20, &cancel).unwrap_err().to_string().contains("cancelled"));
        assert_eq!(calls.get(), 3);
        assert_eq!(read_bounded(&mut &b"exact"[..], 5, &|| false).unwrap(), b"exact");
    }
    #[test]
    fn bounded_exact_read_rejects_devices_and_keeps_valid_symlink_identity() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.zip");
        let alias = dir.path().join("alias.zip");
        fs::write(&source, b"archive").unwrap();
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        let exact = read_exact_bytes_bounded(&alias, 7, &|| false).unwrap();
        assert_eq!(exact.bytes(), b"archive");
        assert_eq!(exact.target.canonical, source);
        assert_eq!(read_exact_bytes_bounded(&alias, 6, &|| false).unwrap_err(), limit());
        if Path::new("/dev/zero").exists() {
            fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink("/dev/zero", &alias).unwrap();
            assert!(read_exact_bytes_bounded(&alias, 7, &|| false).unwrap_err().to_string().contains("not a regular file"));
        }
    }
}

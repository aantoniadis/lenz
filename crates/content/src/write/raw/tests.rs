use super::*;
use std::{io::Cursor, os::unix::fs::symlink};

struct Fixture {
    dir: tempfile::TempDir,
    path: PathBuf,
    read: ExactRead,
}

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("archive.bin");
        fs::write(&path, bytes).unwrap();
        let read = read_exact_bytes(&path).unwrap();
        Self { dir, path, read }
    }

    fn write(&self, bytes: &[u8]) -> WriteOutcome {
        write_bytes_exact(&self.read.bytes_snapshot(), &self.read.target, bytes)
    }

    fn assert_bytes_and_no_temps(&self, bytes: &[u8]) {
        assert_eq!(fs::read(&self.path).unwrap(), bytes);
        assert!(fs::read_dir(self.dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".lenz~")
        }));
    }
}

fn saved(outcome: WriteOutcome) -> (Snapshot, TargetIdentity) {
    match outcome {
        WriteOutcome::Saved { snapshot, target } => (snapshot, target),
        other => panic!("expected confirmed save, got {other:?}"),
    }
}

#[test]
fn binary_replacement_preserves_all_byte_values_mode_and_symlink_target() {
    let fixture = Fixture::new(b"old\0\xff\r\n");
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o6750)).unwrap();
    let metadata = fs::metadata(&fixture.path).unwrap();
    let link = fixture.dir.path().join("linked.bin");
    symlink("archive.bin", &link).unwrap();
    let read = read_exact_bytes(&link).unwrap();
    let bytes: Vec<u8> = (0..=255).collect();

    let (snapshot, target) = saved(write_bytes_exact(
        &read.bytes_snapshot(),
        &read.target,
        &bytes,
    ));

    fixture.assert_bytes_and_no_temps(&bytes);
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("archive.bin"));
    assert_ne!(
        target.inode, read.target.inode,
        "must replace the inode atomically"
    );
    let installed = fs::metadata(&fixture.path).unwrap();
    assert_eq!(installed.mode(), metadata.mode());
    assert_eq!(installed.uid(), metadata.uid());
    assert_eq!(installed.gid(), metadata.gid());
    assert_eq!(snapshot.path, link);
    assert!(snapshot.verify().is_ok());
    saved(write_bytes_exact(&snapshot, &target, b"\xff\x00next\r\n"));
    fixture.assert_bytes_and_no_temps(b"\xff\x00next\r\n");
}

#[test]
fn raw_snapshot_refuses_accidental_use_by_text_writer() {
    let fixture = Fixture::new(b"\x00\xff\xfe\r\n");
    assert_eq!(fixture.read.bytes(), b"\x00\xff\xfe\r\n");
    assert_eq!(
        write_exact(
            &fixture.read.bytes_snapshot(),
            &fixture.read.target,
            "replacement"
        ),
        WriteOutcome::Rejected(WriteError::Lossy),
    );
    fixture.assert_bytes_and_no_temps(fixture.read.bytes());

    let fixture = Fixture::new(b"valid text");
    let (text_snapshot, target, _) = read_exact(&fixture.path, None).unwrap();
    let (raw_snapshot, target) = saved(write_bytes_exact(&text_snapshot, &target, b"\xff\x00"));
    assert_eq!(
        write_exact(&raw_snapshot, &target, "replacement"),
        WriteOutcome::Rejected(WriteError::Lossy)
    );
    fixture.assert_bytes_and_no_temps(b"\xff\x00");
}

struct Fragmented<'a> {
    bytes: &'a [u8],
}

impl Read for Fragmented<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer.len().min(7).min(self.bytes.len());
        buffer[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}

#[test]
fn streaming_replacement_handles_large_baselines_fragmented_reads_and_empty_output() {
    let original: Vec<u8> = (0..196_613).map(|i| (i % 251) as u8).collect();
    let fixture = Fixture::new(&original);
    let bytes: Vec<u8> = (0..200_003).map(|i| (i % 256) as u8).collect();
    let (snapshot, target) = saved(write_bytes_exact_from(
        &fixture.read.bytes_snapshot(),
        &fixture.read.target,
        &mut Fragmented { bytes: &bytes },
    ));
    fixture.assert_bytes_and_no_temps(&bytes);
    assert_eq!(snapshot.hash, Some(hash(&bytes)));
    assert_eq!(snapshot.size, bytes.len() as u64);
    saved(write_bytes_exact(&snapshot, &target, b""));
    fixture.assert_bytes_and_no_temps(b"");
}

#[test]
fn atomic_binary_save_refuses_existing_hardlinks_without_rewriting_either_name() {
    let fixture = Fixture::new(b"original archive");
    let link = fixture.dir.path().join("hardlink.bin");
    fs::hard_link(&fixture.path, &link).unwrap();
    assert!(matches!(
        fixture.write(b"replacement"),
        WriteOutcome::Rejected(WriteError::Io(_))
    ));
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_eq!(fs::read(&link).unwrap(), b"original archive");
    assert_eq!(
        fs::metadata(&fixture.path).unwrap().ino(),
        fixture.read.target.inode
    );
}

#[test]
fn stale_content_and_same_byte_inode_substitutions_are_rejected() {
    let fixture = Fixture::new(b"original archive");
    fs::write(&fixture.path, b"external archive").unwrap();
    assert_eq!(
        fixture.write(b"replacement"),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    fixture.assert_bytes_and_no_temps(b"external archive");

    let fixture = Fixture::new(b"original archive");
    let replacement = fixture.dir.path().join("external.bin");
    fs::write(&replacement, fixture.read.bytes()).unwrap();
    fs::rename(replacement, &fixture.path).unwrap();
    assert_eq!(
        fixture.write(b"replacement"),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    fixture.assert_bytes_and_no_temps(b"original archive");
}

#[test]
fn retargeted_symlink_does_not_overwrite_either_target() {
    let fixture = Fixture::new(b"original archive");
    let other = fixture.dir.path().join("other.bin");
    fs::write(&other, b"original archive").unwrap();
    let link = fixture.dir.path().join("linked.bin");
    symlink("archive.bin", &link).unwrap();
    let read = read_exact_bytes(&link).unwrap();
    fs::remove_file(&link).unwrap();
    symlink("other.bin", &link).unwrap();

    assert_eq!(
        write_bytes_exact(&read.bytes_snapshot(), &read.target, b"replacement"),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_eq!(fs::read(other).unwrap(), b"original archive");
    assert_eq!(fs::read_link(link).unwrap(), Path::new("other.bin"));
}

struct DuringCopy<F> {
    change: Option<F>,
    bytes: Cursor<&'static [u8]>,
}

impl<F: FnOnce()> Read for DuringCopy<F> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if let Some(change) = self.change.take() {
            change();
        }
        self.bytes.read(buffer)
    }
}

fn changing_source(change: impl FnOnce()) -> impl Read {
    DuringCopy {
        change: Some(change),
        bytes: Cursor::new(b"replacement archive"),
    }
}

#[test]
fn final_guard_rejects_content_and_inode_changes_while_streaming() {
    let fixture = Fixture::new(b"original archive");
    let mut source = changing_source(|| fs::write(&fixture.path, b"external archive").unwrap());
    assert_eq!(
        write_bytes_exact_from(
            &fixture.read.bytes_snapshot(),
            &fixture.read.target,
            &mut source
        ),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    fixture.assert_bytes_and_no_temps(b"external archive");

    let fixture = Fixture::new(b"original archive");
    let mut source = changing_source(|| {
        let external = fixture.dir.path().join("external.bin");
        fs::write(&external, fixture.read.bytes()).unwrap();
        fs::rename(external, &fixture.path).unwrap();
    });
    assert_eq!(
        write_bytes_exact_from(
            &fixture.read.bytes_snapshot(),
            &fixture.read.target,
            &mut source
        ),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_ne!(
        fs::metadata(&fixture.path).unwrap().ino(),
        fixture.read.target.inode
    );
}

#[test]
fn final_guard_rejects_new_hardlink_and_permission_changes_while_streaming() {
    let fixture = Fixture::new(b"original archive");
    let link = fixture.dir.path().join("new-hardlink.bin");
    let mut source = changing_source(|| fs::hard_link(&fixture.path, &link).unwrap());
    assert!(matches!(
        write_bytes_exact_from(
            &fixture.read.bytes_snapshot(),
            &fixture.read.target,
            &mut source
        ),
        WriteOutcome::Rejected(WriteError::Io(_))
    ));
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_eq!(fs::read(&link).unwrap(), b"original archive");

    let fixture = Fixture::new(b"original archive");
    fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o640)).unwrap();
    let mut source = changing_source(|| {
        fs::set_permissions(&fixture.path, fs::Permissions::from_mode(0o600)).unwrap()
    });
    assert_eq!(
        write_bytes_exact_from(
            &fixture.read.bytes_snapshot(),
            &fixture.read.target,
            &mut source
        ),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_eq!(fs::metadata(&fixture.path).unwrap().mode() & 0o7777, 0o600);
}

struct FailingSource(bool);

impl Read for FailingSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.0 {
            return Err(io::Error::other("injected source read failure"));
        }
        self.0 = true;
        let prefix = b"partial";
        buffer[..prefix.len()].copy_from_slice(prefix);
        Ok(prefix.len())
    }
}

#[test]
fn failed_stream_and_before_replace_fault_leave_original_confirmed_and_clean_up_temp() {
    let fixture = Fixture::new(b"original archive");
    assert!(matches!(
        write_bytes_exact_from(
            &fixture.read.bytes_snapshot(),
            &fixture.read.target,
            &mut FailingSource(false)
        ),
        WriteOutcome::Rejected(WriteError::Io(_))
    ));
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert!(fixture.read.bytes_snapshot().verify().is_ok());

    assert!(matches!(
        readiness_fault::during(readiness_fault::Fault::BeforeReplace, || fixture
            .write(b"replacement")),
        WriteOutcome::Rejected(WriteError::Io(_))
    ));
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_eq!(
        fs::metadata(&fixture.path).unwrap().ino(),
        fixture.read.target.inode
    );
}

#[test]
fn after_replace_fault_reports_uncertainty_and_requires_a_fresh_baseline() {
    let fixture = Fixture::new(b"original archive");
    assert!(matches!(
        readiness_fault::during(readiness_fault::Fault::AfterReplace, || fixture
            .write(b"replacement")),
        WriteOutcome::Uncertain(WriteError::Io(_))
    ));
    fixture.assert_bytes_and_no_temps(b"replacement");
    assert_eq!(
        fixture.read.bytes_snapshot().verify(),
        Err(WriteError::ChangedOnDisk)
    );
    assert_eq!(
        fixture.write(b"retry"),
        WriteOutcome::Rejected(WriteError::ChangedOnDisk)
    );
    let read = read_exact_bytes(&fixture.path).unwrap();
    saved(write_bytes_exact(
        &read.bytes_snapshot(),
        &read.target,
        b"rechecked",
    ));
    fixture.assert_bytes_and_no_temps(b"rechecked");
}

#[test]
fn foreign_owner_is_refused_when_the_process_can_create_such_a_fixture() {
    // Other users cannot change a fixture's owner. Root can exercise the refusal without a privileged writer mode.
    let fixture = Fixture::new(b"original archive");
    if fixture.read.metadata().uid() != 0 {
        return;
    }
    std::os::unix::fs::chown(&fixture.path, Some(1), None).unwrap();
    assert!(matches!(
        fixture.write(b"replacement"),
        WriteOutcome::Rejected(WriteError::Io(_))
    ));
    fixture.assert_bytes_and_no_temps(b"original archive");
    assert_eq!(fs::metadata(&fixture.path).unwrap().uid(), 1);
}

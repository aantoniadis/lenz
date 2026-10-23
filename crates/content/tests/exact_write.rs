use lenz_content::write::{read_exact, write_exact, WriteOutcome};

#[test]
fn editor_save_preserves_encoding_and_endings() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text");
    std::fs::write(&path, b"\xef\xbb\xbfa\r\nb\r\nc\n").unwrap();
    let (snapshot, target, decoded) = read_exact(&path, None).unwrap();
    assert_eq!(decoded.text, "a\r\nb\r\nc\n");
    assert!(matches!(write_exact(&snapshot, &target, "A\r\nb\r\nc\n"), WriteOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), b"\xef\xbb\xbfA\r\nb\r\nc\n");
}

#[test]
fn editor_save_preserves_encodings_mode_and_hardlinks() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let original = "café\r\nsecond\nlast\r\n";
    let edited = "déjà\r\nsecond\nlast\r\n";
    let utf16 = |s: &str, little: bool, bom: bool| {
        let mut bytes = Vec::new();
        if bom { bytes.extend_from_slice(if little { &[0xff, 0xfe] } else { &[0xfe, 0xff] }); }
        bytes.extend(s.encode_utf16().flat_map(|unit| if little { unit.to_le_bytes() } else { unit.to_be_bytes() }));
        bytes
    };
    let mut cases = vec![(original.as_bytes().to_vec(), edited.as_bytes().to_vec(), None)];
    for little in [true, false] {
        for bom in [true, false] {
            cases.push((utf16(original, little, bom), utf16(edited, little, bom), Some(if little { "UTF-16LE" } else { "UTF-16BE" })));
        }
    }
    cases.push((encoding_rs::WINDOWS_1252.encode(original).0.into_owned(), encoding_rs::WINDOWS_1252.encode(edited).0.into_owned(), Some("windows-1252")));
    for (old, new, declared) in cases {
        for linked in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("text");
            let alias = dir.path().join("alias");
            std::fs::write(&path, &old).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o751)).unwrap();
            if linked { std::fs::hard_link(&path, &alias).unwrap(); }
            let (snapshot, target, decoded) = read_exact(&path, declared).unwrap();
            assert_eq!(decoded.text, original);
            let WriteOutcome::Saved { target: next, .. } = write_exact(&snapshot, &target, edited) else { panic!("save rejected") };
            assert_eq!(std::fs::read(&path).unwrap(), new);
            assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o7777, 0o751);
            if linked {
                assert_eq!(std::fs::read(&alias).unwrap(), new);
                assert_eq!(next, target);
            }
        }
    }
}

#[test]
fn editor_save_rejects_same_path_hardlink_substitution() {
    use lenz_content::write::WriteError;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    std::fs::write(&a, "old\n").unwrap();
    std::fs::write(&b, "old\n").unwrap();
    let (snapshot, target, _) = read_exact(&a, None).unwrap();
    std::fs::remove_file(&a).unwrap();
    std::fs::hard_link(&b, &a).unwrap();
    assert_eq!(write_exact(&snapshot, &target, "new\n"), WriteOutcome::Rejected(WriteError::ChangedOnDisk));
    assert_eq!(std::fs::read(&a).unwrap(), b"old\n");
    assert_eq!(std::fs::read(&b).unwrap(), b"old\n");
}

#[test]
fn editor_save_rejects_retargeted_symlink_and_changed_bytes() {
    use lenz_content::write::WriteError;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a");
    let b = dir.path().join("b");
    let link = dir.path().join("link");
    std::fs::write(&a, "old\n").unwrap();
    std::fs::write(&b, "old\n").unwrap();
    std::os::unix::fs::symlink(&a, &link).unwrap();
    let (snapshot, target, _) = read_exact(&link, None).unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&b, &link).unwrap();
    assert_eq!(write_exact(&snapshot, &target, "new\n"), WriteOutcome::Rejected(WriteError::ChangedOnDisk));
    assert_eq!(std::fs::read(&a).unwrap(), b"old\n");
    assert_eq!(std::fs::read(&b).unwrap(), b"old\n");
    let (snapshot, target, _) = read_exact(&a, None).unwrap();
    std::fs::write(&a, "external\n").unwrap();
    assert_eq!(write_exact(&snapshot, &target, "new\n"), WriteOutcome::Rejected(WriteError::ChangedOnDisk));
    assert_eq!(std::fs::read(&a).unwrap(), b"external\n");
}

#[test]
fn editor_save_refreshes_identity_after_replace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text");
    std::fs::write(&path, "old\n").unwrap();
    let (snapshot, target, _) = read_exact(&path, None).unwrap();
    let WriteOutcome::Saved { snapshot: next, target: next_target } = write_exact(&snapshot, &target, "new\n") else { panic!("first save rejected") };
    assert_eq!(target.canonical, next_target.canonical);
    assert_ne!(target.inode, next_target.inode);
    assert_eq!(next_target, read_exact(&path, None).unwrap().1);
    assert!(matches!(write_exact(&next, &next_target, "again\n"), WriteOutcome::Saved { .. }));
    assert_eq!(std::fs::read(&path).unwrap(), b"again\n");
}

#[test]
fn editor_save_rejects_lossy_and_unrepresentable_text() {
    use lenz_content::write::WriteError;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text");
    std::fs::write(&path, b"\xff\xfe\x00\xd8").unwrap();
    let (snapshot, target, _) = read_exact(&path, None).unwrap();
    assert_eq!(write_exact(&snapshot, &target, "replacement"), WriteOutcome::Rejected(WriteError::Lossy));
    assert_eq!(std::fs::read(&path).unwrap(), b"\xff\xfe\x00\xd8");
    std::fs::write(&path, b"caf\xe9\n").unwrap();
    let (snapshot, target, _) = read_exact(&path, Some("windows-1252")).unwrap();
    assert_eq!(write_exact(&snapshot, &target, "line\n\u{1f600}"), WriteOutcome::Rejected(WriteError::Unrepresentable { line: 2, encoding: "windows-1252" }));
    assert_eq!(std::fs::read(&path).unwrap(), b"caf\xe9\n");
}

#[cfg(feature = "write-testing")]
#[test]
fn editor_save_distinguishes_rejected_and_uncertain_real_file_outcomes() {
    use lenz_content::write::readiness_fault::{during, Fault};
    for (fault, linked, expected, uncertain) in [
        (Fault::BeforeReplace, false, b"abcdef".as_slice(), false),
        (Fault::Write, true, b"abcdef".as_slice(), true),
        (Fault::WriteAndRollback, true, b"aVWdef".as_slice(), true),
        (Fault::AfterReplace, false, b"UVWXYZ".as_slice(), true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text");
        let alias = dir.path().join("alias");
        std::fs::write(&path, b"abcdef").unwrap();
        if linked { std::fs::hard_link(&path, &alias).unwrap(); }
        let (snapshot, target, _) = read_exact(&path, None).unwrap();
        let outcome = during(fault, || write_exact(&snapshot, &target, "UVWXYZ"));
        assert_eq!(matches!(outcome, WriteOutcome::Uncertain(_)), uncertain);
        assert!(matches!(outcome, WriteOutcome::Rejected(_) | WriteOutcome::Uncertain(_)));
        assert_eq!(std::fs::read(&path).unwrap(), expected);
        if linked { assert_eq!(std::fs::read(&alias).unwrap(), expected); }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), if linked { 2 } else { 1 });
    }
}

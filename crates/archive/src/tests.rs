use super::*;
use std::{cell::Cell, io::{Cursor, Read, Write}};

const PAYLOAD: &[u8] = b"\xef\xbb\xbfalpha\r\nbeta\0\xff\n";
const PY_TAR: &[u8] = include_bytes!("../tests/fixtures/python.tar");
const PY_ZIP: &[u8] = include_bytes!("../tests/fixtures/python.zip");

fn source(kind: Kind, bytes: &[u8]) -> Index { Index::open(kind, Arc::from(bytes), Limits::default(), &|| false).unwrap() }
fn named<'a>(index: &'a Index, name: &str) -> &'a Member { index.members().iter().find(|m| m.kind != MemberKind::Metadata && m.path.as_deref() == Some(name)).unwrap() }
fn contents(index: &Index, name: &str) -> Vec<u8> { index.read_member(named(index, name).id, &|| false).unwrap() }
fn formats() -> Vec<(Kind, &'static [u8])> {
    vec![(Kind::Zip, PY_ZIP), (Kind::Tar(Compression::None), PY_TAR),
        (Kind::Tar(Compression::Gzip), include_bytes!("../tests/fixtures/python.tgz")),
        (Kind::Tar(Compression::Bzip2), include_bytes!("../tests/fixtures/python.tbz2")),
        (Kind::Tar(Compression::Xz), include_bytes!("../tests/fixtures/python.txz")),
        (Kind::Tar(Compression::Zstd), include_bytes!("../tests/fixtures/python.tzst"))]
}

#[test]
fn suffixes_match_diff_and_bare_hidden_names_are_not_archives() {
    for (suffix, kind) in [("zip", Kind::Zip), ("tar", Kind::Tar(Compression::None)),
        ("tar.gz", Kind::Tar(Compression::Gzip)), ("tgz", Kind::Tar(Compression::Gzip)),
        ("tar.bz2", Kind::Tar(Compression::Bzip2)), ("tbz", Kind::Tar(Compression::Bzip2)), ("tbz2", Kind::Tar(Compression::Bzip2)),
        ("tar.xz", Kind::Tar(Compression::Xz)), ("txz", Kind::Tar(Compression::Xz)),
        ("tar.zst", Kind::Tar(Compression::Zstd)), ("tzst", Kind::Tar(Compression::Zstd))] {
        assert_eq!(Kind::of(&format!("folder/App.{}", suffix.to_uppercase())), Some(kind));
        assert_eq!(Kind::of(&format!(".{suffix}")), None);
    }
    for path in ["a.gz", "a.jar", "a.docx", "a.rar", "a.7z", "zip", ""] { assert_eq!(Kind::of(path), None); }
}

#[test]
fn selected_archive_layers_keep_their_format_when_canonical_alias_names_have_another_suffix() {
    for (kind, bytes) in formats() {
        assert_eq!(Kind::detect("content-addressed.blob", bytes), Some(kind));
        assert_eq!(Kind::detect("misleading.zip", bytes), Some(kind));
        assert_eq!(Kind::of("content-addressed.blob"), None);
    }
    assert_eq!(Kind::detect("empty.blob", &testing::zip(&[])), Some(Kind::Zip));
    assert_eq!(Kind::detect("empty.blob", &testing::tar(Compression::None, &[])), Some(Kind::Tar(Compression::None)));
    let mut old_tar = Vec::new();
    let mut header = ::tar::Header::new_old();
    header.set_path("old").unwrap(); header.set_size(0); header.set_cksum(); old_tar.extend_from_slice(header.as_bytes());
    assert_eq!(Kind::detect("old.blob", &old_tar), Some(Kind::Tar(Compression::None)));
    assert_eq!(Kind::detect("expected.txz", b"malformed"), Some(Kind::Tar(Compression::Xz)));
    assert_eq!(Kind::detect("unknown.blob", b"malformed"), None);
    let skip = b"\x50\x2a\x4d\x18\0\0\0\0";
    assert_eq!(Kind::detect("skippable.blob", skip), Some(Kind::Tar(Compression::Zstd)));
}

#[test]
fn independent_python_fixtures_read_exact_payload_links_and_metadata() {
    for (kind, bytes) in formats() {
        let index = source(kind, bytes);
        assert_eq!(contents(&index, "dir/plain.txt"), PAYLOAD, "{kind:?}");
        assert!(named(&index, "dir/plain.txt").editable(), "{kind:?}: {:?}", named(&index, "dir/plain.txt").read_only_reason);
        let link = if kind == Kind::Zip { "link" } else { "sym" };
        assert_eq!(contents(&index, link), b"dir/plain.txt");
        assert!(!named(&index, link).editable());
        assert_eq!(index.bytes().as_ref(), bytes);
        assert_eq!(index.replace_members(&[], &|| false).unwrap(), bytes);
    }
}

#[test]
fn all_codecs_rewrite_multiple_members_reparse_and_preserve_independent_metadata() {
    let replacement = b"\xffnew\r\n\0payload";
    for (kind, bytes) in formats() {
        let index = source(kind, bytes);
        let untouched = contents(&index, "untouched.bin");
        let old_link = contents(&index, if kind == Kind::Zip { "link" } else { "sym" });
        let mut updates = vec![Replacement { id: named(&index, "dir/plain.txt").id, bytes: replacement }];
        let long = index.members().iter().find(|m| m.kind == MemberKind::File && m.path.as_ref().is_some_and(|p| p.starts_with("long/")));
        if let Some(long) = long { updates.push(Replacement { id: long.id, bytes: b"new long contents\n" }); }
        let rewritten = index.replace_members(&updates, &|| false).unwrap();
        let reopened = source(kind, &rewritten);
        assert_eq!(reopened.members().len(), index.members().len(), "{kind:?}");
        assert_eq!(contents(&reopened, "dir/plain.txt"), replacement);
        assert_eq!(contents(&reopened, "untouched.bin"), untouched);
        assert_eq!(contents(&reopened, if kind == Kind::Zip { "link" } else { "sym" }), old_link);
        assert_eq!(index.bytes().as_ref(), bytes);
        match &reopened.data {
            Data::Zip(_) => {
                let mut zip = ::zip::ZipArchive::new(Cursor::new(&rewritten)).unwrap();
                assert_eq!(zip.comment(), b"archive comment\0\xff");
                let mut entry = zip.by_name("dir/plain.txt").unwrap();
                assert_eq!(entry.unix_mode(), Some(0o100755));
                assert_eq!(entry.comment(), "entry-note");
                assert_eq!(entry.extra_data().unwrap(), b"\xfe\xca\x04\0ABCD");
                let mut decoded = Vec::new(); entry.read_to_end(&mut decoded).unwrap();
                assert_eq!(decoded, replacement);
            }
            Data::Tar(data) => {
                assert!(data.raw.ends_with(b"TRAILER preserved\n"));
                let mut tar = ::tar::Archive::new(Cursor::new(data.raw.as_ref()));
                let mut entry = tar.entries().unwrap().map(Result::unwrap).find(|e| e.path_bytes().as_ref() == b"dir/plain.txt").unwrap();
                assert_eq!(entry.header().mode().unwrap(), 0o640);
                assert_eq!(entry.header().uid().unwrap(), 123);
                assert_eq!(entry.header().gid().unwrap(), 456);
                assert_eq!(entry.header().username().unwrap(), Some("alice"));
                let pax = entry.pax_extensions().unwrap().unwrap().map(Result::unwrap).map(|p| (p.key().unwrap().to_owned(), p.value().unwrap().to_owned())).collect::<BTreeMap<_, _>>();
                assert_eq!(pax.get("SCHILY.xattr.user.note").map(String::as_str), Some("preserve metadata"));
                assert_eq!(pax.get("mtime").map(String::as_str), Some("1700000000.25"));
            }
        }
    }
}

#[test]
fn zip_listing_and_untouched_copy_do_not_decompress_other_payloads() {
    let mut bytes = testing::zip(&[("good.txt", b"before"), ("corrupt.bin", &[42; 4096])]);
    let mut zip = ::zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
    let ordinal = (0..zip.len()).find(|i| zip.name_for_index(*i).unwrap().unwrap() == "corrupt.bin").unwrap();
    let file = zip.by_index_raw(ordinal).unwrap();
    let start = file.data_start().unwrap() as usize;
    drop(file); drop(zip);
    bytes[start] ^= 0xff;
    let index = source(Kind::Zip, &bytes); // listing reads only metadata.
    assert!(index.read_member(named(&index, "corrupt.bin").id, &|| false).is_err());
    let before = raw_zip(&bytes, "corrupt.bin");
    let output = index.replace_members(&[Replacement { id: named(&index, "good.txt").id, bytes: b"after" }], &|| false).unwrap();
    assert_eq!(raw_zip(&output, "corrupt.bin"), before);
    assert_eq!(contents(&source(Kind::Zip, &output), "good.txt"), b"after");
}
fn raw_zip(bytes: &[u8], name: &str) -> Vec<u8> {
    let mut zip = ::zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let ordinal = (0..zip.len()).find(|i| zip.name_for_index(*i).unwrap().unwrap() == name).unwrap();
    let mut file = zip.by_index_raw(ordinal).unwrap();
    let mut raw = Vec::new(); file.read_to_end(&mut raw).unwrap(); raw
}

#[test]
fn zip_duplicates_aliases_and_snapshot_ids_are_explicit() {
    let bytes = stored_zip(&[("same", b"first"), ("same", b"last")]);
    let index = source(Kind::Zip, &bytes);
    assert!(index.rewrite_blocked().unwrap().contains("duplicate"));
    assert_eq!(contents(&index, "same"), b"last");
    assert!(matches!(index.replace_members(&[Replacement { id: named(&index, "same").id, bytes: b"new" }], &|| false), Err(Error::Unsupported(_))));
    let bytes = stored_zip(&[("a/./b", b"one"), ("a/b", b"two"), ("../hidden", b"three")]);
    let index = source(Kind::Zip, &bytes);
    assert_eq!(index.members().len(), 3);
    assert!(!index.members()[0].editable()); assert!(!index.members()[1].editable());
    assert_eq!(index.members()[2].path, None);
    let clone = index.clone();
    assert_eq!(clone.read_member(index.members()[0].id, &|| false).unwrap(), b"one");
    let reopened = source(Kind::Zip, &bytes);
    assert!(matches!(reopened.read_member(index.members()[0].id, &|| false), Err(Error::StaleMember)));
}

#[test]
fn conflicting_zip_local_integrity_metadata_is_readable_but_not_silently_rewritten() {
    let mut bytes = stored_zip(&[("file", b"original")]);
    bytes[14] ^= 0xff; // Local CRC conflicts with the central CRC, without a data descriptor flag.
    let index = source(Kind::Zip, &bytes);
    assert!(index.rewrite_blocked().unwrap().contains("integrity metadata disagree"));
    assert_eq!(contents(&index, "file"), b"original");
    assert!(matches!(index.replace_members(&[Replacement { id: index.members()[0].id, bytes: b"new" }], &|| false), Err(Error::Unsupported(_))));
}

#[test]
fn opaque_zip_extra_padding_remains_browsable_with_an_explicit_preserving_write_limit() {
    let bytes = include_bytes!("../tests/fixtures/python-padding.zip");
    let index = source(Kind::Zip, bytes);
    assert_eq!(contents(&index, "file"), b"padding fixture");
    assert!(index.rewrite_blocked().unwrap().contains("opaque ZIP extra-field padding"));
    assert!(matches!(index.replace_members(&[Replacement { id: index.members()[0].id, bytes: b"new" }], &|| false), Err(Error::Unsupported(_))));
}

#[test]
fn every_codec_rejects_malformed_input_and_respects_stored_unpacked_member_and_count_limits() {
    for (kind, bytes) in formats() {
        let mut limits = Limits { max_archive: (bytes.len() - 1) as u64, ..Limits::default() };
        assert!(matches!(Index::open(kind, Arc::from(bytes), limits, &|| false), Err(Error::Limit("stored bytes"))));
        limits = Limits { max_unpacked: 10, ..Limits::default() };
        assert!(matches!(Index::open(kind, Arc::from(bytes), limits, &|| false), Err(Error::Limit("unpacked bytes"))));
        limits = Limits { max_members: 1, ..Limits::default() };
        assert!(matches!(Index::open(kind, Arc::from(bytes), limits, &|| false), Err(Error::Limit("member count"))));
        limits = Limits { max_member: 2, ..Limits::default() };
        let index = Index::open(kind, Arc::from(bytes), limits, &|| false).unwrap();
        assert!(matches!(index.read_member(named(&index, "dir/plain.txt").id, &|| false), Err(Error::Limit("member bytes"))));
        assert!(!named(&index, "dir/plain.txt").editable());
        assert!(Index::open(kind, Arc::from(&bytes[..bytes.len().min(8)]), Limits::default(), &|| false).is_err(), "{kind:?}");
        if kind != Kind::Tar(Compression::None) {
            assert!(Index::open(kind, Arc::from(&bytes[..bytes.len() - 8]), Limits::default(), &|| false).is_err(), "{kind:?}");
        }
    }
    let duplicate_bomb = stored_zip(&[("same", b"a"), ("same", b"b")]);
    assert!(matches!(Index::open(Kind::Zip, duplicate_bomb.into(), Limits { max_members: 1, ..Limits::default() }, &|| false), Err(Error::Limit("member count"))));
}

#[test]
fn gzip_preserves_its_exact_original_header_including_header_crc() {
    let bytes = include_bytes!("../tests/fixtures/python.tgz");
    let index = source(Kind::Tar(Compression::Gzip), bytes);
    let rewritten = index.replace_members(&[Replacement { id: named(&index, "dir/plain.txt").id, bytes: b"changed" }], &|| false).unwrap();
    let Data::Tar(data) = &index.data else { panic!() };
    let header = data.gzip_header.as_deref().unwrap();
    assert_eq!(&rewritten[..header.len()], header);
    assert_eq!(contents(&source(index.kind(), &rewritten), "dir/plain.txt"), b"changed");
}

#[test]
fn tar_duplicates_gnu_long_paths_links_and_raw_records_survive_editing_another_member() {
    let long = format!("deep/{}file.txt", "path/".repeat(30));
    let mut raw = Vec::new();
    append_record(&mut raw, "././@LongLink", b'L', format!("{long}\0").as_bytes(), None);
    append_record(&mut raw, "placeholder", b'0', b"long content", None);
    append_record(&mut raw, "same", b'0', b"first", None);
    append_record(&mut raw, "same", b'0', b"second", None);
    append_record(&mut raw, "././@LongLink", b'K', b"very-long-target\0", None);
    append_record(&mut raw, "link", b'2', b"", Some("placeholder"));
    raw.extend_from_slice(&[0; 1024]);
    let index = source(Kind::Tar(Compression::None), &raw);
    assert_eq!(contents(&index, &long), b"long content");
    assert_eq!(contents(&index, "link"), b"very-long-target");
    let duplicates = index.members().iter().filter(|m| m.path.as_deref() == Some("same")).collect::<Vec<_>>();
    assert_eq!(duplicates.len(), 2);
    assert_eq!(index.read_member(duplicates[0].id, &|| false).unwrap(), b"first");
    assert_eq!(index.read_member(duplicates[1].id, &|| false).unwrap(), b"second");
    assert!(duplicates.iter().all(|m| !m.editable()));
    let rewritten = index.replace_members(&[Replacement { id: named(&index, &long).id, bytes: b"edited long\n" }], &|| false).unwrap();
    let reopened = source(index.kind(), &rewritten);
    assert_eq!(contents(&reopened, &long), b"edited long\n");
    assert_eq!(&rewritten[..1024], &raw[..1024]); // untouched GNU LongName header + payload.
    let old = raw_records(&raw); let new = raw_records(&rewritten);
    assert_eq!(old.len(), new.len());
    for i in 0..old.len() { if i != 1 { assert_eq!(new[i], old[i], "raw record {i}"); } }
}

#[test]
fn pax_size_override_is_browsable_but_not_a_lossy_write_target() {
    let mut bytes = Vec::new();
    append_record(&mut bytes, "pax", b'x', b"10 size=3\n", None);
    let at = bytes.len();
    append_record(&mut bytes, "target", b'0', b"abc", None);
    let mut header = ::tar::Header::new_old(); header.as_mut_bytes().copy_from_slice(&bytes[at..at + 512]);
    header.set_size(0); header.set_cksum(); bytes[at..at + 512].copy_from_slice(header.as_bytes());
    append_record(&mut bytes, "other", b'0', b"unchanged", None);
    bytes.extend_from_slice(&[0; 1024]);
    let index = source(Kind::Tar(Compression::None), &bytes);
    assert_eq!(contents(&index, "target"), b"abc");
    assert!(named(&index, "target").read_only_reason.as_ref().unwrap().contains("PAX size"));
    assert!(matches!(index.replace_members(&[Replacement { id: named(&index, "target").id, bytes: b"unsafe" }], &|| false), Err(Error::Unsupported(_))));
    let output = index.replace_members(&[Replacement { id: named(&index, "other").id, bytes: b"edited" }], &|| false).unwrap();
    assert_eq!(contents(&source(index.kind(), &output), "target"), b"abc");
}

#[test]
fn compressed_wrapper_shapes_that_would_lose_metadata_are_explicitly_read_only() {
    let first = codecs::encode(Compression::Gzip, &PY_TAR[..5120], None, Limits::default(), &|| false).unwrap();
    let second = codecs::encode(Compression::Gzip, &PY_TAR[5120..], None, Limits::default(), &|| false).unwrap();
    let mut multi = first; multi.extend_from_slice(&second);
    let index = source(Kind::Tar(Compression::Gzip), &multi);
    assert!(index.rewrite_blocked().unwrap().contains("multi-stream"));
    assert_eq!(contents(&index, "dir/plain.txt"), PAYLOAD);
    let zstd = include_bytes!("../tests/fixtures/python.tzst");
    let mut skip = b"\x50\x2a\x4d\x18\x04\0\0\0note".to_vec(); skip.extend_from_slice(zstd);
    let index = source(Kind::Tar(Compression::Zstd), &skip);
    assert!(index.rewrite_blocked().unwrap().contains("skippable"));
    assert_eq!(contents(&index, "dir/plain.txt"), PAYLOAD);
}

#[test]
fn mixed_nested_rewrites_compose_and_keep_siblings_exact() {
    let inner = testing::tar(Compression::Xz, &[("leaf.txt", PAYLOAD), ("sibling", b"inside")]);
    let outer = testing::zip(&[("nested.txz", &inner), ("outside", b"outer sibling")]);
    let outer_index = source(Kind::Zip, &outer);
    let inner_index = source(Kind::Tar(Compression::Xz), &contents(&outer_index, "nested.txz"));
    let new_inner = inner_index.replace_members(&[Replacement { id: named(&inner_index, "leaf.txt").id, bytes: b"edited nested\r\n" }], &|| false).unwrap();
    let new_outer = outer_index.replace_members(&[Replacement { id: named(&outer_index, "nested.txz").id, bytes: &new_inner }], &|| false).unwrap();
    let reopened = source(Kind::Zip, &new_outer);
    assert_eq!(contents(&reopened, "outside"), b"outer sibling");
    let nested = source(Kind::Tar(Compression::Xz), &contents(&reopened, "nested.txz"));
    assert_eq!(contents(&nested, "leaf.txt"), b"edited nested\r\n");
    assert_eq!(contents(&nested, "sibling"), b"inside");
}

#[test]
fn cancelled_or_over_budget_operations_return_no_partial_candidate() {
    for (kind, bytes) in formats() {
        assert!(matches!(Index::open(kind, Arc::from(bytes), Limits::default(), &|| true), Err(Error::Cancelled)));
        let index = source(kind, bytes);
        let id = named(&index, "dir/plain.txt").id;
        assert!(matches!(index.read_member(id, &|| true), Err(Error::Cancelled)));
        assert!(matches!(index.replace_members(&[Replacement { id, bytes: b"change" }], &|| true), Err(Error::Cancelled)));
        let calls = Cell::new(0);
        let cancel = || { calls.set(calls.get() + 1); calls.get() > 5 };
        assert!(matches!(index.replace_members(&[Replacement { id, bytes: b"change" }], &cancel), Err(Error::Cancelled)));
        assert_eq!(index.bytes().as_ref(), bytes);
        assert!(matches!(index.replace_members(&[Replacement { id, bytes: b"one" }, Replacement { id, bytes: b"two" }], &|| false), Err(Error::Unsupported(_))));
    }
    for compression in [Compression::None, Compression::Gzip, Compression::Bzip2, Compression::Xz, Compression::Zstd] {
        let limits = Limits { max_archive: 1, ..Limits::default() };
        assert!(matches!(codecs::encode(compression, b"too big for one byte", None, limits, &|| false), Err(Error::Limit("stored bytes"))));
    }
}

#[test]
fn resident_estimate_includes_the_unpacked_tar_snapshot() {
    let raw = testing::tar(Compression::None, &[("big", &[b'x'; 100_000])]);
    for compression in [Compression::Gzip, Compression::Bzip2, Compression::Xz, Compression::Zstd] {
        let bytes = codecs::encode(compression, &raw, None, Limits::default(), &|| false).unwrap();
        let index = source(Kind::Tar(compression), &bytes);
        assert!(index.resident_bytes() >= bytes.len() + raw.len());
    }
}

#[test]
fn global_pax_metadata_cannot_amplify_tiny_headers_past_the_index_budget() {
    let value = "p".repeat(2048);
    let body = format!(" path={value}\n");
    let mut length = body.len() + 1;
    loop { let next = body.len() + length.to_string().len(); if next == length { break; } length = next; }
    let pax = format!("{length}{body}");
    let mut bytes = Vec::new();
    append_record(&mut bytes, "global", b'g', pax.as_bytes(), None);
    for name in ["a", "b", "c"] { append_record(&mut bytes, name, b'0', b"", None); }
    bytes.extend_from_slice(&[0; 1024]);
    let limits = Limits { max_unpacked: bytes.len() as u64, ..Limits::default() };
    assert!(matches!(Index::open(Kind::Tar(Compression::None), bytes.into(), limits, &|| false), Err(Error::Limit("index metadata bytes"))));
}

#[test]
fn every_zip_writer_method_and_forced_zip64_can_be_reopened_after_replacement() {
    for method in [::zip::CompressionMethod::Stored, ::zip::CompressionMethod::Deflated, ::zip::CompressionMethod::Bzip2, ::zip::CompressionMethod::Xz] {
        let mut writer = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = ::zip::write::SimpleFileOptions::default().compression_method(method).large_file(true).unix_permissions(0o751);
        for name in ["selected", "untouched"] { writer.start_file(name, options).unwrap(); writer.write_all(PAYLOAD).unwrap(); }
        let bytes = writer.finish().unwrap().into_inner();
        let index = source(Kind::Zip, &bytes);
        let output = index.replace_members(&[Replacement { id: named(&index, "selected").id, bytes: b"new payload" }], &|| false).unwrap();
        let candidate = source(Kind::Zip, &output);
        assert_eq!(contents(&candidate, "selected"), b"new payload");
        assert_eq!(contents(&candidate, "untouched"), PAYLOAD);
        assert_eq!(raw_zip(&output, "untouched"), raw_zip(&bytes, "untouched"));
    }
}

fn append_record(out: &mut Vec<u8>, path: &str, kind: u8, bytes: &[u8], link: Option<&str>) {
    let mut header = ::tar::Header::new_gnu();
    header.set_path(path).unwrap(); header.set_size(bytes.len() as u64); header.set_mode(0o640); header.set_uid(123); header.set_gid(456);
    header.set_entry_type(::tar::EntryType::new(kind)); if let Some(link) = link { header.set_link_name(link).unwrap(); }
    header.set_cksum(); out.extend_from_slice(header.as_bytes()); out.extend_from_slice(bytes);
    out.resize(out.len() + (512 - bytes.len() % 512) % 512, 0);
}
fn raw_records(raw: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new(); let mut at = 0;
    while raw.get(at..at + 512).is_some_and(|b| b.iter().any(|b| *b != 0)) {
        let header = ::tar::Header::from_byte_slice(&raw[at..at + 512]);
        let size = header.entry_size().unwrap() as usize;
        let end = at + 512 + size.div_ceil(512) * 512;
        out.push(raw[at..end].to_vec()); at = end;
    }
    out
}

// Deliberately independent classic stored-ZIP fixture writer: zip::ZipWriter refuses duplicate names.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new(); let mut central = Vec::new();
    for (name, content) in files {
        let offset = bytes.len() as u32; let crc = crc32fast::hash(content); let size = content.len() as u32;
        bytes.extend_from_slice(b"PK\x03\x04"); bytes.extend_from_slice(&[20,0,0,0,0,0,0,0,0x21,0]);
        for value in [crc, size, size] { bytes.extend_from_slice(&value.to_le_bytes()); }
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes()); bytes.extend_from_slice(&[0,0]); bytes.extend_from_slice(name.as_bytes()); bytes.extend_from_slice(content);
        central.extend_from_slice(b"PK\x01\x02"); central.extend_from_slice(&[20,3,20,0,0,0,0,0,0,0,0x21,0]);
        for value in [crc, size, size] { central.extend_from_slice(&value.to_le_bytes()); }
        central.extend_from_slice(&(name.len() as u16).to_le_bytes()); central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes()); central.extend_from_slice(&offset.to_le_bytes()); central.extend_from_slice(name.as_bytes());
    }
    let start = bytes.len() as u32; let size = central.len() as u32; bytes.extend_from_slice(&central);
    bytes.extend_from_slice(b"PK\x05\x06"); bytes.extend_from_slice(&[0;4]);
    bytes.extend_from_slice(&(files.len() as u16).to_le_bytes()); bytes.extend_from_slice(&(files.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&size.to_le_bytes()); bytes.extend_from_slice(&start.to_le_bytes()); bytes.extend_from_slice(&[0;2]); bytes
}

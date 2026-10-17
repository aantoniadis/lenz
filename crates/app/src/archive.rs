//! Archives as folders: a zip or tar (plain, gzip, bzip2, xz or zstd) Side lists its member files, so a Change Set can
//! show one Comparison per changed member. Everything happens in memory; nothing is extracted to disk.

use std::{
    collections::BTreeMap,
    io::{self, Cursor, Read},
};

/// An archive format Lenz opens as a folder, chosen by file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    Tar(Compression),
}

/// The compression around a tar stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

/// An archive larger than this as stored is not expanded: it is read whole into memory.
pub(crate) const MAX_ARCHIVE: u64 = 512 << 20;
/// A tar stream is decompressed whole to fingerprint its members; listing stops past this many bytes.
const MAX_UNPACKED: u64 = 2 << 30;
/// The largest member read for display.
const MAX_MEMBER: u64 = 1 << 30;
/// Listing stops past this many members.
const MAX_MEMBERS: usize = 100_000;

impl Kind {
    /// The format `path`'s file name names, if Lenz opens it as a folder.
    pub fn of(path: &str) -> Option<Kind> {
        let name = path.rsplit('/').next()?.to_ascii_lowercase();
        // a bare `.zip` is a hidden file, not an archive
        let ends = |suffixes: &[&str]| suffixes.iter().any(|s| name.len() > s.len() && name.ends_with(s));
        Some(if ends(&[".zip"]) {
            Kind::Zip
        } else if ends(&[".tar"]) {
            Kind::Tar(Compression::None)
        } else if ends(&[".tar.gz", ".tgz"]) {
            Kind::Tar(Compression::Gzip)
        } else if ends(&[".tar.bz2", ".tbz2", ".tbz"]) {
            Kind::Tar(Compression::Bzip2)
        } else if ends(&[".tar.xz", ".txz"]) {
            Kind::Tar(Compression::Xz)
        } else if ends(&[".tar.zst", ".tzst"]) {
            Kind::Tar(Compression::Zstd)
        } else {
            return None;
        })
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Zip => "zip archive",
            Kind::Tar(Compression::None) => "tar archive",
            Kind::Tar(Compression::Gzip) => "gzip tar archive",
            Kind::Tar(Compression::Bzip2) => "bzip2 tar archive",
            Kind::Tar(Compression::Xz) => "xz tar archive",
            Kind::Tar(Compression::Zstd) => "zstd tar archive",
        }
    }

    fn error(self) -> impl Fn(String) -> String {
        move |e| format!("invalid {}: {e}", self.label())
    }
}

/// What identifies a member's content: its size and CRC-32, as zip records them. Zip members are listed from the
/// central directory without decompressing them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Fingerprint {
    pub size: u64,
    crc: u32,
}

#[cfg(test)]
impl Fingerprint {
    fn of(bytes: &[u8]) -> Self {
        Fingerprint { size: bytes.len() as u64, crc: crc32fast::hash(bytes) }
    }
}

/// Every member file by normalized path. Directories are left out, and a later duplicate replaces an earlier one,
/// as extracting would. Members whose path climbs out of the archive (`..`) are left out too.
pub(crate) fn members(kind: Kind, bytes: &[u8]) -> Result<BTreeMap<String, Fingerprint>, String> {
    let mut out = BTreeMap::new();
    match kind {
        Kind::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| kind.error()(e.to_string()))?;
            for index in 0..zip.len() {
                // raw: the central directory's record, so neither decompression nor a password is needed
                let file = zip.by_index_raw(index).map_err(|e| kind.error()(e.to_string()))?;
                if file.is_dir() {
                    continue;
                }
                let Some(name) = normalize(&file.name().map_err(|e| kind.error()(e.to_string()))?) else { continue };
                out.insert(name, Fingerprint { size: file.size(), crc: file.crc32() });
                if out.len() > MAX_MEMBERS {
                    return Err(format!("more than {MAX_MEMBERS} files"));
                }
            }
        }
        Kind::Tar(compression) => {
            each_tar(compression, bytes, |name, content| {
                let mut hasher = crc32fast::Hasher::new();
                let mut size = 0;
                let mut buffer = [0; 64 << 10];
                loop {
                    let n = content.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                    size += n as u64;
                }
                out.insert(name, Fingerprint { size, crc: hasher.finalize() });
                if out.len() > MAX_MEMBERS {
                    return Err(io::Error::other(format!("more than {MAX_MEMBERS} files")));
                }
                Ok(())
            })
            .map_err(kind.error())?;
        }
    }
    Ok(out)
}

/// Member `name`'s bytes (as [`members`] names it); a symbolic or hard link's content is its target, as git stores one.
pub(crate) fn extract(kind: Kind, bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let missing = || format!("lenz: {name} is not in the {}", kind.label());
    let too_large = || format!("lenz: {name} is larger than {} MiB", MAX_MEMBER >> 20);
    match kind {
        Kind::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| kind.error()(e.to_string()))?;
            let mut found = None;
            for index in 0..zip.len() {
                let file = zip.by_index_raw(index).map_err(|e| kind.error()(e.to_string()))?;
                if !file.is_dir() && file.name().ok().and_then(|n| normalize(&n)).as_deref() == Some(name) {
                    found = Some(index);
                }
            }
            let file = zip.by_index(found.ok_or_else(missing)?).map_err(|e| format!("lenz: {name}: {e}"))?;
            if file.size() > MAX_MEMBER {
                return Err(too_large());
            }
            let mut out = Vec::with_capacity(file.size() as usize);
            // the reader checks the CRC-32 at its end
            file.take(MAX_MEMBER + 1).read_to_end(&mut out).map_err(|e| format!("lenz: {name}: {e}"))?;
            if out.len() as u64 > MAX_MEMBER {
                return Err(too_large());
            }
            Ok(out)
        }
        Kind::Tar(compression) => {
            let mut found = None;
            each_tar(compression, bytes, |member, content| {
                if member == name {
                    let mut out = Vec::new();
                    content.take(MAX_MEMBER + 1).read_to_end(&mut out)?;
                    if out.len() as u64 > MAX_MEMBER {
                        return Err(io::Error::other(too_large()));
                    }
                    found = Some(out);
                }
                Ok(())
            })
            .map_err(kind.error())?;
            found.ok_or_else(missing)
        }
    }
}

/// Member `name`'s size, read from the archive's headers.
pub(crate) fn size(kind: Kind, bytes: &[u8], name: &str) -> Result<u64, String> {
    match kind {
        Kind::Zip => members(kind, bytes)?.get(name).map(|f| f.size).ok_or_else(|| format!("lenz: {name} is not in the {}", kind.label())),
        // a tar stream has no index: reading the member costs about as much as finding its header
        Kind::Tar(_) => extract(kind, bytes, name).map(|b| b.len() as u64),
    }
}

/// A compressed tar stream decompressed whole, to read many members from without decompressing it again; `None` past
/// `limit` bytes.
pub(crate) fn unpack(compression: Compression, bytes: &[u8], limit: u64) -> Result<Option<Vec<u8>>, String> {
    let error = Kind::Tar(compression).error();
    let mut out = Vec::new();
    stream(compression, bytes).map_err(&error)?.take(limit + 1).read_to_end(&mut out).map_err(|e| error(e.to_string()))?;
    Ok((out.len() as u64 <= limit).then_some(out))
}

fn stream(compression: Compression, bytes: &[u8]) -> Result<Box<dyn Read + '_>, String> {
    Ok(match compression {
        Compression::None => Box::new(bytes),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(bytes)),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(bytes)),
        Compression::Xz => Box::new(lzma_rust2::XzReader::new(bytes, true)),
        Compression::Zstd => Box::new(ruzstd::decoding::StreamingDecoder::new(bytes).map_err(|e| e.to_string())?),
    })
}

/// Calls `visit` with each member file's normalized path and content, in archive order.
fn each_tar(
    compression: Compression,
    bytes: &[u8],
    mut visit: impl FnMut(String, &mut dyn Read) -> io::Result<()>,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(Limit { inner: stream(compression, bytes)?, left: MAX_UNPACKED });
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path_bytes();
        let path = std::str::from_utf8(&path).map_err(|_| "non-UTF-8 archive member paths are unsupported".to_string())?;
        let Some(name) = normalize(path) else { continue };
        let kind = entry.header().entry_type();
        if kind.is_file() || kind == tar::EntryType::Continuous {
            visit(name, &mut entry)
        } else if kind.is_symlink() || kind.is_hard_link() {
            let target = entry.link_name_bytes().map(|t| t.into_owned()).unwrap_or_default();
            visit(name, &mut target.as_slice())
        } else {
            continue;
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `name` relative and `/`-separated, without empty or `.` parts; `None` for one that climbs out with `..` or names
/// nothing.
fn normalize(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A reader that fails once more than `left` bytes came through, so a decompression bomb stops early.
struct Limit<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for Limit<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buffer)?;
        self.left = self
            .left
            .checked_sub(n as u64)
            .ok_or_else(|| io::Error::other(format!("larger than {} MiB unpacked", MAX_UNPACKED >> 20)))?;
        Ok(n)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::io::Write;

    use super::Compression;

    /// A zip archive of `files` (path, content), deflated.
    pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in files {
            out.start_file(*path, options).unwrap();
            out.write_all(content).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// A tar archive of `files` (path, content), compressed as `compression` asks where the test can write it.
    pub fn tar(files: &[(&str, &[u8])], compression: Compression) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, *content).unwrap();
        }
        let tar = builder.into_inner().unwrap();
        match compression {
            Compression::None => tar,
            Compression::Gzip => {
                let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                gz.write_all(&tar).unwrap();
                gz.finish().unwrap()
            }
            Compression::Bzip2 => {
                let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::default());
                bz.write_all(&tar).unwrap();
                bz.finish().unwrap()
            }
            Compression::Xz => {
                let mut xz = lzma_rust2::XzWriter::new(Vec::new(), lzma_rust2::XzOptions::default()).unwrap();
                xz.write_all(&tar).unwrap();
                xz.finish().unwrap()
            }
            Compression::Zstd => panic!("no pure-Rust zstd encoder in the tests"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{testing, *};

    #[test]
    fn formats_are_chosen_by_file_name() {
        assert_eq!(Kind::of("dist/App.ZIP"), Some(Kind::Zip));
        assert_eq!(Kind::of("a.tar"), Some(Kind::Tar(Compression::None)));
        assert_eq!(Kind::of("a.tar.gz"), Some(Kind::Tar(Compression::Gzip)));
        assert_eq!(Kind::of("a.tgz"), Some(Kind::Tar(Compression::Gzip)));
        assert_eq!(Kind::of("a.tbz2"), Some(Kind::Tar(Compression::Bzip2)));
        assert_eq!(Kind::of("a.tar.xz"), Some(Kind::Tar(Compression::Xz)));
        assert_eq!(Kind::of("a.tar.zst"), Some(Kind::Tar(Compression::Zstd)));
        for other in ["a.gz", "a.rar", ".zip", "zip", "a.zip/b.txt", "a.docx"] {
            assert_eq!(Kind::of(other), None, "{other}");
        }
    }

    #[test]
    fn zip_members_are_listed_without_directories_and_extracted() {
        let zip = testing::zip(&[("dir/", b""), ("dir/a.txt", b"alpha\n"), ("./b.txt", b"beta\n"), ("../evil", b"x")]);
        let list = members(Kind::Zip, &zip).unwrap();
        assert_eq!(list.keys().collect::<Vec<_>>(), ["b.txt", "dir/a.txt"]);
        assert_eq!(list["dir/a.txt"], Fingerprint::of(b"alpha\n"));
        assert_eq!(extract(Kind::Zip, &zip, "dir/a.txt").unwrap(), b"alpha\n");
        assert_eq!(extract(Kind::Zip, &zip, "b.txt").unwrap(), b"beta\n");
        assert_eq!(size(Kind::Zip, &zip, "b.txt").unwrap(), 5);
        assert!(extract(Kind::Zip, &zip, "missing").unwrap_err().contains("not in the zip archive"));
    }

    #[test]
    fn tar_members_are_listed_and_extracted_in_every_compression() {
        for compression in [Compression::None, Compression::Gzip, Compression::Bzip2, Compression::Xz] {
            let tar = testing::tar(&[("a.txt", b"one\n"), ("sub/b.bin", b"\0\x01"), ("a.txt", b"two\n")], compression);
            let kind = Kind::Tar(compression);
            let list = members(kind, &tar).unwrap();
            assert_eq!(list.keys().collect::<Vec<_>>(), ["a.txt", "sub/b.bin"], "{compression:?}");
            // a later duplicate wins, as extracting the archive would leave it
            assert_eq!(list["a.txt"], Fingerprint::of(b"two\n"));
            assert_eq!(extract(kind, &tar, "a.txt").unwrap(), b"two\n");
            assert_eq!(size(kind, &tar, "sub/b.bin").unwrap(), 2);
            // unpacked once, the plain stream reads the same members
            let plain = unpack(compression, &tar, 1 << 20).unwrap().unwrap();
            assert_eq!(extract(Kind::Tar(Compression::None), &plain, "a.txt").unwrap(), b"two\n");
            assert_eq!(unpack(compression, &tar, 16).unwrap(), None, "past the limit");
        }
    }

    #[test]
    fn a_zstd_tar_stream_is_read() {
        let tar = testing::tar(&[("a.txt", b"zstd\n")], Compression::None);
        let frame = zstd_stored_frame(&tar);
        assert_eq!(extract(Kind::Tar(Compression::Zstd), &frame, "a.txt").unwrap(), b"zstd\n");
    }

    /// A zstd frame of raw (uncompressed) blocks: valid zstd any decoder reads, without an encoder.
    fn zstd_stored_frame(data: &[u8]) -> Vec<u8> {
        let mut out = vec![0x28, 0xB5, 0x2F, 0xFD];
        // frame header: single segment, 8-byte content size
        out.push(0b1110_0000);
        out.extend((data.len() as u64).to_le_bytes());
        let chunks: Vec<&[u8]> = data.chunks(128 << 10).collect();
        for (i, chunk) in chunks.iter().enumerate() {
            let last = u32::from(i + 1 == chunks.len());
            // raw block: type 0, size in the upper 21 bits
            let header = last | ((chunk.len() as u32) << 3);
            out.extend(&header.to_le_bytes()[..3]);
            out.extend(*chunk);
        }
        out
    }

    #[test]
    fn corrupt_and_oversized_archives_fail_with_a_reason() {
        assert!(members(Kind::Zip, b"not a zip").unwrap_err().starts_with("invalid zip archive"));
        assert!(members(Kind::Tar(Compression::Gzip), b"not gzip").unwrap_err().starts_with("invalid gzip tar archive"));
        let mut limited = Limit { inner: &[0u8; 10][..], left: 4 };
        assert!(limited.read_to_end(&mut Vec::new()).unwrap_err().to_string().contains("unpacked"));
    }

    #[test]
    fn names_never_climb_out_of_the_archive() {
        assert_eq!(normalize("./a//b/./c"), Some("a/b/c".into()));
        assert_eq!(normalize("/abs/x"), Some("abs/x".into()));
        assert_eq!(normalize("a/../b"), None);
        assert_eq!(normalize("./"), None);
    }
    #[test]
    fn summary_tar_names_cannot_collide_after_utf8_replacement() {
        let mut builder = tar::Builder::new(Vec::new());
        for name in [b"bad\xff.txt".as_slice(), "bad�.txt".as_bytes()] {
            let mut header = tar::Header::new_gnu();
            header.as_mut_bytes()[..name.len()].copy_from_slice(name);
            header.set_size(4); header.set_mode(0o644); header.set_cksum();
            builder.append(&header, b"old\n".as_slice()).unwrap();
        }
        let tar = builder.into_inner().unwrap();
        let kind = Kind::Tar(Compression::None);
        assert!(members(kind, &tar).unwrap_err().contains("non-UTF-8"));
        assert!(extract(kind, &tar, "bad�.txt").unwrap_err().contains("non-UTF-8"));
        // The ChangeSet must explain why it kept the outer archive, rather than call this packing-only.
        let d = tempfile::tempdir().unwrap();
        let (left, right) = (d.path().join("old.tar"), d.path().join("new.tar"));
        std::fs::write(&left, &tar).unwrap();
        let mut changed = tar.clone();
        let at = changed.windows(4).position(|bytes| bytes == b"old\n").unwrap();
        changed[at..at + 3].copy_from_slice(b"new");
        std::fs::write(&right, changed).unwrap();
        let set = crate::changeset::ChangeSet::load(&crate::changeset::Spec::Plain { left, right }, d.path()).unwrap();
        assert_eq!(set.comparisons.len(), 1);
        assert!(!set.comparisons[0].in_archive());
        assert!(set.note(0).unwrap().contains("non-UTF-8"));
    }

}

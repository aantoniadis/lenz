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
/// The read size when fingerprinting members.
const BUFFER: usize = 64 << 10;
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

/// What identifies a member's content: its size and the SHA-256 of the bytes read from it. Never a stored claim (zip's
/// central directory) or a CRC-32, which an archive's author can match with different bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Fingerprint {
    pub size: u64,
    digest: [u8; 32],
}

impl Fingerprint {
    /// `buffer` is reused across members: zeroing a new 64 KiB one per member cost more than hashing a small member.
    fn read(content: &mut dyn Read, buffer: &mut [u8]) -> io::Result<Self> {
        use sha2::Digest;
        let (mut hasher, mut size) = (sha2::Sha256::new(), 0);
        loop {
            let n = content.read(buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(buffer.get(..n).unwrap_or_default());
            size += n as u64;
        }
        Ok(Fingerprint { size, digest: hasher.finalize().into() })
    }

    #[cfg(test)]
    fn of(bytes: &[u8]) -> Self {
        Fingerprint::read(&mut &bytes[..], &mut [0; 64]).unwrap()
    }
}

/// Every member file by normalized path, as extracting would write it; directories are left out. An archive whose
/// headers could make an extractor write something else is an error, never a shorter or different listing: headers
/// that name a member two ways, a path listed twice, a member with no path inside the archive, or an entry of a kind
/// Lenz does not read.
pub(crate) fn members(kind: Kind, bytes: &[u8]) -> Result<BTreeMap<String, Fingerprint>, String> {
    let mut out = BTreeMap::new();
    match kind {
        Kind::Zip => {
            let (mut zip, entries) = zip_entries(bytes)?;
            return zip_fingerprints(&mut zip, &entries, MAX_UNPACKED, |_| false);
        }
        Kind::Tar(compression) => {
            let mut buffer = vec![0; BUFFER];
            each_tar(compression, bytes, |name, content| {
                out.insert(name, Fingerprint::read(content, &mut buffer)?);
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

/// The members of two zips as [`members`] lists them, leaving out of both each same-name pair proven identical without
/// decompressing it: equal compression method, flags, sizes and CRC-32 and equal stored (still compressed) bytes are
/// equal inputs to the decoder, so they extract to the same content or fail alike. The CRC-32 is never what makes them
/// equal. Every other member is decompressed and fingerprinted, as [`members`] does; every member's headers are checked.
pub(crate) fn changed_zip_members(left: &[u8], right: &[u8]) -> Result<[BTreeMap<String, Fingerprint>; 2], String> {
    let ((mut left_zip, left), (mut right_zip, right)) = (zip_entries(left)?, zip_entries(right)?);
    let same = |name: &String| left.get(name).zip(right.get(name)).is_some_and(|(l, r)| l.stored == r.stored);
    Ok([
        zip_fingerprints(&mut left_zip, &left, MAX_UNPACKED, same)?,
        zip_fingerprints(&mut right_zip, &right, MAX_UNPACKED, same)?,
    ])
}

/// A zip member as its headers describe it.
struct ZipEntry<'a> {
    index: usize,
    /// Everything the decoder reads to produce the member's content.
    stored: Stored<'a>,
}

#[derive(PartialEq, Eq)]
struct Stored<'a> {
    method: zip::CompressionMethod,
    flags: u16,
    size: u64,
    crc32: u32,
    raw: &'a [u8],
}

/// Every member file of a zip by normalized path, from its headers alone: refused as [`members`] describes, and when a
/// member is encrypted or uses a compression method Lenz does not read, as decompressing it would be.
fn zip_entries(bytes: &[u8]) -> Result<(Zip<'_>, BTreeMap<String, ZipEntry<'_>>), String> {
    let error = Kind::Zip.error();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| error(e.to_string()))?;
    // extractors read local headers, and differ on a repeated name: both must agree with the central directory
    zip.validate_header_consistency().map_err(|e| error(e.to_string()))?;
    let mut out = BTreeMap::new();
    for index in 0..zip.len() {
        let file = zip.by_index_raw(index).map_err(|e| error(e.to_string()))?;
        let raw = file.name().map_err(|e| error(e.to_string()))?.into_owned();
        if file.is_dir() {
            // UnZip writes a name ending in `\` as a file, and a directory holding data is not one
            if file.size() != 0 || file.name_raw().ends_with(b"\\") {
                return Err(error(format!("member {raw:?} is a directory name holding a file")));
            }
            continue;
        }
        let name = normalize(&raw).ok_or_else(|| error(format!("member {raw:?} has no path inside the archive")))?;
        if file.encrypted() {
            return Err(error(format!("{raw}: {}", zip::result::ZipError::PASSWORD_REQUIRED)));
        }
        // the variant is what this build of the crate cannot decode, whatever its features
        #[allow(deprecated)]
        let unsupported = matches!(file.compression(), zip::CompressionMethod::Unsupported(_));
        if unsupported {
            return Err(error(format!("{raw}: compression method {} is not supported", file.compression())));
        }
        // the bytes the crate's raw reader yields: its data start, as far as the stored size reaches
        let start = file.data_start().ok_or_else(|| error(format!("{raw}: no data offset")))?;
        let raw_bytes = usize::try_from(start)
            .ok()
            .zip(usize::try_from(file.compressed_size()).ok())
            .and_then(|(start, len)| bytes.get(start..start.checked_add(len)?))
            .ok_or_else(|| error(format!("{raw}: stored data past the end of the archive")))?;
        let stored = Stored { method: file.compression(), flags: file.flags().as_u16(), size: file.size(), crc32: file.crc32(), raw: raw_bytes };
        if out.insert(name, ZipEntry { index, stored }).is_some() {
            return Err(error(format!("member {raw:?} appears more than once")));
        }
        if out.len() > MAX_MEMBERS {
            return Err(format!("more than {MAX_MEMBERS} files"));
        }
    }
    Ok((zip, out))
}

/// The fingerprint of each of `entries` but those `skip` names, decompressed (the reader checks the CRC-32) with at most
/// `left` bytes unpacked in all; a skipped member counts its stated size.
fn zip_fingerprints(
    zip: &mut Zip<'_>,
    entries: &BTreeMap<String, ZipEntry<'_>>,
    mut left: u64,
    skip: impl Fn(&String) -> bool,
) -> Result<BTreeMap<String, Fingerprint>, String> {
    let error = Kind::Zip.error();
    let too_large = || error(format!("larger than {} MiB unpacked", MAX_UNPACKED >> 20));
    let (mut out, mut buffer) = (BTreeMap::new(), vec![0; BUFFER]);
    for (name, entry) in entries {
        if skip(name) {
            left = left.checked_sub(entry.stored.size).ok_or_else(too_large)?;
            continue;
        }
        let mut file = zip.by_index(entry.index).map_err(|e| error(format!("{name}: {e}")))?;
        let mut limited = Limit { inner: &mut file, left };
        let fingerprint = Fingerprint::read(&mut limited, &mut buffer).map_err(|e| error(format!("{name}: {e}")))?;
        left = limited.left;
        out.insert(name.clone(), fingerprint);
    }
    Ok(out)
}

/// Member `name`'s bytes (as [`members`] names it); a symbolic or hard link's content is its target, as git stores one.
pub(crate) fn extract(kind: Kind, bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let missing = || format!("lenz: {name} is not in the {}", kind.label());
    let too_large = || format!("lenz: {name} is larger than {} MiB", MAX_MEMBER >> 20);
    match kind {
        Kind::Zip => {
            let (mut zip, index) = zip_member(bytes, name)?;
            let file = zip.by_index(index).map_err(|e| format!("lenz: {name}: {e}"))?;
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

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

/// Zip member `name` (as [`members`] names it), found from the central directory alone.
fn zip_member<'a>(bytes: &'a [u8], name: &str) -> Result<(Zip<'a>, usize), String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Kind::Zip.error()(e.to_string()))?;
    let mut found = None;
    for index in 0..zip.len() {
        let file = zip.by_index_raw(index).map_err(|e| Kind::Zip.error()(e.to_string()))?;
        if !file.is_dir() && file.name().ok().and_then(|n| normalize(&n)).as_deref() == Some(name) {
            found = Some(index);
        }
    }
    let index = found.ok_or_else(|| format!("lenz: {name} is not in the {}", Kind::Zip.label()))?;
    Ok((zip, index))
}

/// Member `name`'s size, read from the archive's headers.
pub(crate) fn size(kind: Kind, bytes: &[u8], name: &str) -> Result<u64, String> {
    match kind {
        // the central directory's record: decompressing every member to find one size would cost a whole listing
        Kind::Zip => {
            let (mut zip, index) = zip_member(bytes, name)?;
            zip.by_index_raw(index).map(|file| file.size()).map_err(|e| kind.error()(e.to_string()))
        }
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
        Compression::Zstd => Box::new(ZstdFrames { rest: bytes, frame: None }),
    })
}

/// Every zstd frame in turn, skipping skippable frames, as `zstd -d` reads a file; ruzstd's decoder reads one frame.
struct ZstdFrames<'a> {
    rest: &'a [u8],
    frame: Option<ruzstd::decoding::StreamingDecoder<&'a [u8], ruzstd::decoding::FrameDecoder>>,
}

impl Read for ZstdFrames<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(frame) = &mut self.frame {
                let n = frame.read(buffer)?;
                if n > 0 {
                    return Ok(n);
                }
                self.rest = self.frame.take().map_or(self.rest, |frame| frame.into_inner());
            }
            if self.rest.is_empty() {
                return Ok(0);
            }
            // magic 0x184D2A5?, then a 32-bit length: metadata with no content
            if let Some((magic, length)) = self.rest.first_chunk::<8>().map(|h| h.split_at(4))
                && let (Ok(magic), Ok(length)) = (<[u8; 4]>::try_from(magic), <[u8; 4]>::try_from(length))
                && u32::from_le_bytes(magic) & 0xffff_fff0 == 0x184d_2a50
            {
                let size = u32::from_le_bytes(length) as usize;
                self.rest = self.rest[8..].get(size..).ok_or_else(|| io::Error::other("truncated zstd skippable frame"))?;
                continue;
            }
            // the frame header is read here, so every turn of the loop consumes input
            self.frame = Some(ruzstd::decoding::StreamingDecoder::new(self.rest).map_err(io::Error::other)?);
        }
    }
}

/// Calls `visit` with each member file's normalized path and content, in archive order. Fails where GNU tar or bsdtar
/// could write something the listing does not show (see [`members`]).
fn each_tar(
    compression: Compression,
    bytes: &[u8],
    mut visit: impl FnMut(String, &mut dyn Read) -> io::Result<()>,
) -> Result<(), String> {
    use tar::EntryType;
    let mut archive = tar::Archive::new(Limit { inner: stream(compression, bytes)?, left: MAX_UNPACKED });
    let mut seen = std::collections::HashSet::new();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        one_name(&mut entry, kind)?;
        if kind.is_pax_global_extensions() {
            continue;
        }
        // GNU tar and bsdtar give these no data and read the next block as a header; the tar crate skips `size` bytes
        let no_data = kind.is_dir() || kind.is_symlink() || kind.is_hard_link() || kind.is_character_special()
            || kind.is_block_special() || kind.is_fifo();
        if no_data && entry.size() != 0 {
            return Err(format!("the archive holds data on a {:?} entry, which extractors read as more members", kind.as_byte() as char));
        }
        let path = entry.path_bytes();
        let path = std::str::from_utf8(&path).map_err(|_| "non-UTF-8 archive member paths are unsupported".to_string())?;
        let Some(name) = normalize(path) else {
            // a root `./` directory names nothing; any other entry is written somewhere
            if kind.is_dir() {
                continue;
            }
            return Err(format!("member {path:?} has no path inside the archive"));
        };
        if kind.is_dir() {
            continue;
        }
        // extracting keeps the last copy, but a hard link made before it keeps an earlier one
        if !seen.insert(name.clone()) {
            return Err(format!("member {name:?} appears more than once"));
        }
        if seen.len() > MAX_MEMBERS {
            return Err(format!("more than {MAX_MEMBERS} files"));
        }
        // the tar crate expands a GNU sparse map into the entry's reader
        if kind.is_file() || kind == EntryType::Continuous || kind.is_gnu_sparse() {
            visit(name, &mut entry)
        } else if kind.is_symlink() || kind.is_hard_link() {
            let target = entry.link_name_bytes().map(|t| t.into_owned()).unwrap_or_default();
            visit(name, &mut target.as_slice())
        } else if kind.is_character_special() || kind.is_block_special() || kind.is_fifo() {
            continue;
        } else {
            // GNU tar writes an unknown type as a regular file
            return Err(format!("member {name:?} has tar entry type {:?}, which Lenz does not read", kind.as_byte() as char));
        }
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Fails unless the tar crate names `entry` as GNU tar and bsdtar would. They take a PAX path over a GNU long name and
/// the last of repeated records, GNU tar applies a global header's path, and the tar crate passes through, as entries,
/// extension headers that lack ustar magic.
fn one_name(entry: &mut tar::Entry<impl Read>, kind: tar::EntryType) -> Result<(), String> {
    let conflict = |what: &str| Err(format!("the archive's headers disagree about a member: {what}"));
    if kind.is_gnu_longname() || kind.is_gnu_longlink() || kind.is_pax_local_extensions() {
        return conflict("an extension header has no ustar magic");
    }
    // extension headers before a global one would be spent on it here, but on the next member by GNU tar
    if kind.is_pax_global_extensions() && entry.raw_header_position() != 0 {
        return conflict("a PAX global header is not the first");
    }
    let (path, link) = (entry.path_bytes().into_owned(), entry.link_name_bytes().map(|l| l.into_owned()));
    let Some(fields) = entry.pax_extensions().map_err(|e| e.to_string())? else { return Ok(()) };
    let (mut paths, mut links, mut sizes) = (Vec::new(), Vec::new(), 0);
    for field in fields {
        let field = field.map_err(|e| e.to_string())?;
        let key = field.key_bytes();
        // sparse PAX members hold a map the tar crate does not expand, and may carry their real name in `GNU.sparse.name`
        if key.starts_with(b"GNU.sparse") || key == b"SCHILY.realsize" {
            return conflict("a PAX sparse member");
        }
        if kind.is_pax_global_extensions() && matches!(key, b"path" | b"linkpath" | b"size") {
            return conflict("a PAX global header sets a member's path, link or size");
        }
        match key {
            b"path" => paths.push(field.value_bytes().to_vec()),
            b"linkpath" => links.push(field.value_bytes().to_vec()),
            // the tar crate ignores a size it cannot parse, bsdtar reads its leading digits
            b"size" if field.value().ok().filter(|v| v.bytes().all(|b| b.is_ascii_digit())).and_then(|v| v.parse::<u64>().ok()).is_none() => {
                return conflict("a PAX size is not a plain number");
            }
            b"size" => sizes += 1,
            _ => {}
        }
    }
    if paths.len() > 1 || links.len() > 1 || sizes > 1 {
        return conflict("a PAX header repeats a path, link or size");
    }
    if paths.first().is_some_and(|p| *p != path) || links.first().is_some_and(|l| Some(l) != link.as_ref()) {
        return conflict("a GNU long name differs from the PAX one");
    }
    Ok(())
}

/// `name` relative and `/`-separated, without empty or `.` parts; `None` for one that climbs out with `..`, names
/// nothing, or holds a NUL (extractors end the name there).
fn normalize(name: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part if part.contains('\0') => return None,
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
        let zip = testing::zip(&[("dir/", b""), ("dir/a.txt", b"alpha\n"), ("./b.txt", b"beta\n")]);
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
            let tar = testing::tar(&[("a.txt", b"two\n"), ("sub/b.bin", b"\0\x01")], compression);
            let kind = Kind::Tar(compression);
            let list = members(kind, &tar).unwrap();
            assert_eq!(list.keys().collect::<Vec<_>>(), ["a.txt", "sub/b.bin"], "{compression:?}");
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

    /// A Change Set of `old` and `new`, written as `old.<ext>` and `new.<ext>` in the returned directory.
    fn load(ext: &str, old: &[u8], new: &[u8]) -> (tempfile::TempDir, crate::changeset::ChangeSet) {
        let d = tempfile::tempdir().unwrap();
        let (left, right) = (d.path().join(format!("old.{ext}")), d.path().join(format!("new.{ext}")));
        std::fs::write(&left, old).unwrap();
        std::fs::write(&right, new).unwrap();
        let set = crate::changeset::ChangeSet::load(&crate::changeset::Spec::Plain { left, right }, d.path()).unwrap();
        (d, set)
    }

    /// The Change Set kept the archive whole and says why: never "only its packing differs", never a partial list.
    fn refused(ext: &str, old: &[u8], new: &[u8]) -> String {
        let (_d, set) = load(ext, old, new);
        assert_eq!(set.comparisons.len(), 1);
        assert!(!set.comparisons[0].in_archive());
        let note = set.note(0).unwrap();
        assert!(note.starts_with("not shown as a folder"), "{note}");
        note.to_string()
    }

    /// `(file name, status)` of each Comparison.
    fn listed(set: &crate::changeset::ChangeSet) -> Vec<(String, crate::changeset::Status)> {
        set.comparisons.iter().map(|c| (c.path.rsplit('/').next().unwrap().to_string(), c.status.clone())).collect()
    }

    /// A tar of `entries` exactly as given, sizes and checksums set, then the end blocks.
    fn raw_tar(entries: Vec<(tar::Header, Vec<u8>)>) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (mut header, data) in entries {
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder.append(&header, data.as_slice()).unwrap();
        }
        builder.into_inner().unwrap()
    }

    /// A header of type `kind` named `name` in its 100-byte name field; `new` picks the magic (ustar, GNU or none).
    fn header(new: fn() -> tar::Header, name: &str, kind: u8) -> tar::Header {
        let mut header = new();
        header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
        header.set_entry_type(tar::EntryType::new(kind));
        header.set_mode(0o644);
        header
    }

    /// PAX extended header records.
    fn pax(records: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (key, value) in records {
            let body = format!(" {key}={value}\n");
            // the length counts its own digits
            let mut len = body.len() + 1;
            while len.to_string().len() + body.len() != len {
                len = len.to_string().len() + body.len();
            }
            out.extend(format!("{len}{body}").bytes());
        }
        out
    }

    fn good() -> Vec<u8> {
        raw_tar(vec![(header(tar::Header::new_ustar, "install.sh", b'0'), b"echo good\n".to_vec())])
    }

    #[test]
    fn tar_headers_that_name_a_member_two_ways_are_refused() {
        let ustar = tar::Header::new_ustar;
        let payload = || b"echo EVIL\n".to_vec();
        let member = || (header(ustar, "notes.txt", b'0'), payload());
        let cases = [
            // GNU tar and bsdtar take the PAX path; the tar crate the GNU long name
            ("long name vs PAX path", vec![(header(tar::Header::new_gnu, "././@LongLink", b'L'), b"notes.txt\0".to_vec()),
                (header(ustar, "PaxHeaders/x", b'x'), pax(&[("path", "install.sh")])), member()]),
            ("long name '.' vs PAX path", vec![(header(tar::Header::new_gnu, "././@LongLink", b'L'), b".\0".to_vec()),
                (header(ustar, "PaxHeaders/x", b'x'), pax(&[("path", "install.sh")])), member()]),
            // extractors take the last record; the tar crate the first
            ("repeated PAX path", vec![(header(ustar, "PaxHeaders/x", b'x'), pax(&[("path", "notes.txt"), ("path", "install.sh")])), member()]),
            ("repeated PAX size", vec![(header(ustar, "PaxHeaders/x", b'x'), pax(&[("size", "10"), ("size", "10")])), member()]),
            // GNU tar applies a global path; the tar crate ignores it
            ("PAX global path", vec![(header(ustar, "pax_global_header", b'g'), pax(&[("path", "install.sh")])), member()]),
            ("PAX global header after a member", vec![member(), (header(ustar, "pax_global_header", b'g'), pax(&[("comment", "x")]))]),
            // without magic the tar crate returns the header as an entry; extractors apply it
            ("long name without magic", vec![(header(tar::Header::new_old, "././@LongLink", b'L'), b"install.sh\0".to_vec()), member()]),
            ("PAX header without magic", vec![(header(tar::Header::new_old, "PaxHeaders/x", b'x'), pax(&[("path", "install.sh")])), member()]),
            // the real name and content of a sparse PAX member are in its records and map
            ("PAX sparse member", vec![(header(ustar, "PaxHeaders/x", b'x'), pax(&[("GNU.sparse.name", "install.sh")])), member()]),
        ];
        for (case, entries) in cases {
            let tar = raw_tar(entries);
            let error = members(Kind::Tar(Compression::None), &tar).unwrap_err();
            assert!(error.contains("headers disagree"), "{case}: {error}");
            refused("tar", &good(), &tar);
        }
    }

    #[test]
    fn zip_local_and_central_names_must_agree() {
        // bsdtar extracts by the local header's name, which the central directory listing never sees
        let old = testing::zip(&[("install.sh", b"echo good\n")]);
        let mut new = testing::zip(&[("notes0.txt", b"echo EVIL\n")]);
        let local = new.windows(10).position(|w| w == b"notes0.txt").unwrap();
        new[local..local + 10].copy_from_slice(b"install.sh");
        assert!(refused("zip", &old, &new).contains("header inconsistency"));
    }

    #[test]
    fn tar_long_names_global_headers_and_sparse_members_still_list() {
        let long = format!("{}/install.sh", "deep".repeat(30));
        let mut builder = tar::Builder::new(Vec::new());
        let mut gnu = tar::Header::new_gnu();
        gnu.set_size(5);
        gnu.set_mode(0o644);
        // a GNU header's name field holds 100 bytes: the builder writes an `L` long name
        builder.append_data(&mut gnu, &long, b"long\n".as_slice()).unwrap();
        let gnu_long = builder.into_inner().unwrap();
        let pax_long = raw_tar(vec![
            (header(tar::Header::new_ustar, "pax_global_header", b'g'), pax(&[("comment", "0123abcd")])),
            (header(tar::Header::new_ustar, "PaxHeaders/x", b'x'), pax(&[("path", &long), ("mtime", "1.5")])),
            (header(tar::Header::new_ustar, "truncated", b'0'), b"long\n".to_vec()),
        ]);
        let both = raw_tar(vec![
            (header(tar::Header::new_gnu, "././@LongLink", b'L'), format!("{long}\0").into_bytes()),
            (header(tar::Header::new_ustar, "PaxHeaders/x", b'x'), pax(&[("path", &long)])),
            (header(tar::Header::new_ustar, "truncated", b'0'), b"long\n".to_vec()),
            (header(tar::Header::new_ustar, "dir/", b'5'), Vec::new()),
            (header(tar::Header::new_ustar, "pipe", b'6'), Vec::new()),
        ]);
        for tar in [gnu_long, pax_long, both] {
            let list = members(Kind::Tar(Compression::None), &tar).unwrap();
            assert_eq!(list.keys().collect::<Vec<_>>(), [&long]);
            assert_eq!(list[&long], Fingerprint::of(b"long\n"));
        }
        // GNU sparse: the tar crate expands the map, so the member is listed by what extracting writes
        let mut sparse = header(tar::Header::new_gnu, "hook.sh", b'S');
        let gnu = sparse.as_gnu_mut().unwrap();
        gnu.sparse[0].set_offset(4);
        gnu.sparse[0].set_length(6);
        gnu.set_real_size(10);
        let tar = raw_tar(vec![(sparse, b"PWNED\n".to_vec())]);
        let kind = Kind::Tar(Compression::None);
        assert_eq!(members(kind, &tar).unwrap()["hook.sh"], Fingerprint::of(b"\0\0\0\0PWNED\n"));
        assert_eq!(extract(kind, &tar, "hook.sh").unwrap(), b"\0\0\0\0PWNED\n");
    }

    #[test]
    fn duplicate_members_are_refused() {
        // a hard link made before the copy that replaces its target keeps the earlier bytes
        let ustar = tar::Header::new_ustar;
        let mut link = header(ustar, "run.sh", b'1');
        link.set_link_name("install.sh").unwrap();
        let tar = raw_tar(vec![
            (header(ustar, "install.sh", b'0'), b"echo EVIL\n".to_vec()),
            (link, Vec::new()),
            (header(ustar, "install.sh", b'0'), b"echo good\n".to_vec()),
        ]);
        assert!(refused("tar", &good(), &tar).contains("appears more than once"));
        let plain = testing::tar(&[("a.txt", b"one\n"), ("a.txt", b"two\n")], Compression::Gzip);
        assert!(members(Kind::Tar(Compression::Gzip), &plain).unwrap_err().contains("appears more than once"));

        // zip indexes a repeated stored name at its first position with its last record's data
        let old = testing::zip(&[("install.sh", b"echo good\n")]);
        let mut zip = testing::zip(&[("install.sh", b"echo good\n"), ("./install.sh", b"echo good\n"), ("xnstall.sh", b"echo EVIL\n")]);
        let mut at = 0;
        while let Some(found) = zip[at..].windows(10).position(|w| w == b"xnstall.sh") {
            zip[at + found] = b'i';
            at += found + 1;
        }
        assert!(refused("zip", &old, &zip).contains("duplicate"));
        // different stored names, one extracted path
        let zip = testing::zip(&[("install.sh", b"echo good\n"), ("./install.sh", b"echo EVIL\n")]);
        assert!(refused("zip", &old, &zip).contains("appears more than once"));
    }

    #[test]
    fn members_extractors_write_are_never_skipped() {
        let ustar = tar::Header::new_ustar;
        let base = || (header(ustar, "README", b'0'), b"v1\n".to_vec());
        let old = raw_tar(vec![base()]);
        // GNU tar writes an unknown type as a regular file; bsdtar too
        for kind in *b"Z8" {
            let new = raw_tar(vec![base(), (header(ustar, "hook.sh", kind), b"echo PWNED\n".to_vec())]);
            assert!(refused("tar", &old, &new).contains("entry type"));
        }
        let new = raw_tar(vec![base(), (header(ustar, "../hook.sh", b'0'), b"echo PWNED\n".to_vec())]);
        assert!(refused("tar", &old, &new).contains("no path inside the archive"));

        let old = testing::zip(&[("install.sh", b"echo good\n")]);
        for (name, reason) in [("../hook.sh", "no path inside the archive"), ("hook.sh\\", "directory name holding a file"),
            ("hook/", "directory name holding a file")] {
            let new = testing::zip(&[("install.sh", b"echo good\n"), (name, b"echo PWNED\n")]);
            assert!(refused("zip", &old, &new).contains(reason), "{name}");
        }
    }

    /// A member's header and content blocks, without the end blocks: placed inside another entry's data.
    fn hidden_member() -> Vec<u8> {
        let mut block = raw_tar(vec![(header(tar::Header::new_ustar, "hook.sh", b'0'), b"echo PWNED\n".to_vec())]);
        block.truncate(block.len() - 1024);
        block
    }

    #[test]
    fn data_on_an_entry_that_extractors_give_none_is_refused() {
        let ustar = tar::Header::new_ustar;
        let base = || (header(ustar, "README", b'0'), b"v1\n".to_vec());
        let old = raw_tar(vec![base()]);
        // GNU tar and bsdtar read the block after these headers as the next header
        for (name, kind) in [("dir/", b'5'), ("link", b'2'), ("hard", b'1'), ("tty", b'3'), ("disk", b'4'), ("pipe", b'6')] {
            let mut entry = header(ustar, name, kind);
            if matches!(kind, b'1' | b'2') {
                entry.set_link_name("README").unwrap();
            }
            let new = raw_tar(vec![base(), (entry, hidden_member())]);
            assert!(refused("tar", &old, &new).contains("data on a"), "{}", kind as char);
        }
        // the same through a PAX size, the header's own size zero
        let mut new = raw_tar(vec![base(), (header(ustar, "PaxHeaders/x", b'x'), pax(&[("size", "1024")])), (header(ustar, "dir/", b'5'), Vec::new())]);
        new.truncate(new.len() - 1024);
        new.extend(hidden_member());
        new.extend([0; 1024]);
        assert!(refused("tar", &old, &new).contains("data on a"));
    }

    #[test]
    fn a_pax_size_that_is_not_a_plain_number_is_refused() {
        let ustar = tar::Header::new_ustar;
        for size in ["20x", "+11", "", " 11", "11 ", "-0", "0x11", "99999999999999999999999"] {
            let tar = raw_tar(vec![(header(ustar, "PaxHeaders/x", b'x'), pax(&[("size", size)])), (header(ustar, "notes.txt", b'0'), b"echo EVIL\n".to_vec())]);
            let error = members(Kind::Tar(Compression::None), &tar).unwrap_err();
            assert!(error.contains("headers disagree"), "{size:?}: {error}");
            refused("tar", &good(), &tar);
        }
        let plain = raw_tar(vec![(header(ustar, "PaxHeaders/x", b'x'), pax(&[("size", "10")])), (header(ustar, "notes.txt", b'0'), b"echo good\n".to_vec())]);
        assert_eq!(members(Kind::Tar(Compression::None), &plain).unwrap()["notes.txt"], Fingerprint::of(b"echo good\n"));
    }

    #[test]
    fn a_member_name_with_a_nul_is_refused() {
        let ustar = tar::Header::new_ustar;
        let member = || (header(ustar, "notes.txt", b'0'), b"echo EVIL\n".to_vec());
        let cases = [
            vec![(header(ustar, "PaxHeaders/x", b'x'), pax(&[("path", "install.sh\0")])), member()],
            vec![(header(tar::Header::new_gnu, "././@LongLink", b'L'), b"install.sh\0\0".to_vec()), member()],
        ];
        for entries in cases {
            let mut all = vec![(header(ustar, "install.sh", b'0'), b"echo good\n".to_vec())];
            all.extend(entries);
            assert!(refused("tar", &good(), &raw_tar(all)).contains("no path inside the archive"));
        }
        assert_eq!(normalize("a/b\0"), None);
        let zip = testing::zip(&[("install.sh", b"echo good\n"), ("install.sh\0", b"echo EVIL\n")]);
        assert!(refused("zip", &testing::zip(&[("install.sh", b"echo good\n")]), &zip).contains("no path inside the archive"));
    }

    #[test]
    fn members_are_compared_by_content_not_size_and_crc() {
        // equal size and CRC-32, different bytes
        let a = [33, 90, 232, 108, 111, 206, 0, 228];
        let b = [252, 81, 220, 85, 53, 59, 68, 216];
        assert_eq!(crc32fast::hash(&a), crc32fast::hash(&b));
        let zip = |files: &[(&str, &[u8])]| testing::zip(files);
        let tar = |files: &[(&str, &[u8])]| testing::tar(files, Compression::Gzip);
        for (ext, make) in [("zip", &zip as &dyn Fn(&[(&str, &[u8])]) -> Vec<u8>), ("tar.gz", &tar)] {
            let (_d, set) = load(ext, &make(&[("README", b"v1\n"), ("lib.bin", &a)]), &make(&[("README", b"v2\n"), ("lib.bin", &b)]));
            let modified = crate::changeset::Status::Modified;
            assert_eq!(listed(&set), [("README".into(), modified.clone()), ("lib.bin".into(), modified)]);
            // not paired as a rename with identical content
            let (_d, set) = load(ext, &make(&[("old.bin", &a)]), &make(&[("new.bin", &b)]));
            assert!(set.comparisons.iter().all(|c| !matches!(c.status, crate::changeset::Status::Renamed { .. })), "{ext}");
            assert_eq!(set.comparisons.len(), 2, "{ext}");
        }
        // a zip whose stored CRC-32 claims the old bytes: the reader checks it
        let stored = |content: &[u8]| {
            let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
            out.start_file("main.py", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            std::io::Write::write_all(&mut out, content).unwrap();
            out.finish().unwrap().into_inner()
        };
        let old = stored(b"verify_signature = 1\n");
        let mut forged = old.clone();
        let at = forged.windows(18).position(|w| w == b"verify_signature =").unwrap();
        forged[at + 19] = b'0';
        assert!(refused("zip", &old, &forged).contains("checksum"));
        // an encrypted member can't be read without its password, so it can't be compared
        let mut encrypted = stored(b"verify_signature = 0\n");
        let central = encrypted.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        encrypted[6] |= 1;
        encrypted[central + 8] |= 1;
        assert!(refused("zip", &old, &encrypted).contains("assword"));
    }

    /// A zip of `files`, each stored or deflated as given.
    fn zip_of(files: &[(&str, &[u8], zip::CompressionMethod)]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, content, method) in files {
            out.start_file(*path, zip::write::SimpleFileOptions::default().compression_method(*method)).unwrap();
            std::io::Write::write_all(&mut out, content).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// Applies `patch` to the general-purpose flags and method of member `name`'s local and central headers alike.
    fn patch_member(zip: &mut [u8], name: &str, patch: impl Fn(&mut [u8; 4])) {
        // local header: flags at 6, method at 8, name at 30; central: flags at 8, method at 10, name at 46
        for (magic, at, name_at) in [(b"PK\x03\x04", 6, 30), (b"PK\x01\x02", 8, 46)] {
            let header = (0..zip.len() - name_at)
                .find(|&i| &zip[i..i + 4] == magic && zip[i + name_at..].starts_with(name.as_bytes()))
                .unwrap();
            let fields: &mut [u8; 4] = (&mut zip[header + at..header + at + 4]).try_into().unwrap();
            patch(fields);
        }
    }

    #[test]
    fn zip_stored_bytes_are_what_the_raw_reader_reads() {
        let zip = zip_of(&[("a.txt", &b"stored\n".repeat(50), zip::CompressionMethod::Stored),
            ("b.txt", &b"deflated\n".repeat(50), zip::CompressionMethod::Deflated)]);
        let (mut archive, entries) = zip_entries(&zip).unwrap();
        assert_eq!(entries.len(), 2);
        for entry in entries.values() {
            let mut raw = Vec::new();
            archive.by_index_raw(entry.index).unwrap().read_to_end(&mut raw).unwrap();
            assert_eq!(entry.stored.raw, raw);
        }
    }

    #[test]
    fn identical_stored_zip_members_are_not_decompressed_and_the_rest_are() {
        let deflated = zip::CompressionMethod::Deflated;
        let old = zip_of(&[("same.txt", b"same\n", deflated), ("README", b"v1\n", deflated)]);
        let new = zip_of(&[("same.txt", b"same\n", deflated), ("README", b"v2\n", deflated), ("added", b"+\n", deflated)]);
        let [left, right] = changed_zip_members(&old, &new).unwrap();
        assert_eq!(left.keys().collect::<Vec<_>>(), ["README"]);
        assert_eq!(right.keys().collect::<Vec<_>>(), ["README", "added"]);
        assert_eq!(right["README"], Fingerprint::of(b"v2\n"));
        // the same content stored another way is decompressed and found equal
        let repacked = zip_of(&[("same.txt", b"same\n", zip::CompressionMethod::Stored), ("README", b"v1\n", deflated)]);
        let [left, right] = changed_zip_members(&old, &repacked).unwrap();
        assert_eq!(left, right);
        assert_eq!(left["same.txt"], Fingerprint::of(b"same\n"));
        // equal stored bytes under another flag or method take no shortcut
        let mut flagged = old.clone();
        patch_member(&mut flagged, "README", |f| f[0] ^= 1 << 1);
        assert!(changed_zip_members(&old, &flagged).unwrap()[0].contains_key("README"));
        let mut method = zip_of(&[("same.txt", b"same\n", deflated), ("README", b"v1\n", zip::CompressionMethod::Stored)]);
        let old_stored = method.clone();
        patch_member(&mut method, "README", |f| f[2] = 12); // bzip2
        assert!(changed_zip_members(&old_stored, &method).is_err());
    }

    #[test]
    fn zip_members_with_equal_size_crc_and_stored_size_are_still_compared_by_content() {
        // equal size and CRC-32, different bytes; stored, so the compressed sizes match too
        let a = [33, 90, 232, 108, 111, 206, 0, 228];
        let b = [252, 81, 220, 85, 53, 59, 68, 216];
        assert_eq!(crc32fast::hash(&a), crc32fast::hash(&b));
        let stored = zip::CompressionMethod::Stored;
        let same = b"unchanged\n".as_slice();
        let (_d, set) = load("zip", &zip_of(&[("lib.bin", &a, stored), ("same", same, stored)]),
            &zip_of(&[("lib.bin", &b, stored), ("same", same, stored)]));
        assert_eq!(listed(&set), [("lib.bin".into(), crate::changeset::Status::Modified)]);
        let (_d, set) = load("zip", &zip_of(&[("old.bin", &a, stored), ("same", same, stored)]),
            &zip_of(&[("new.bin", &b, stored), ("same", same, stored)]));
        let added = (("new.bin".into(), crate::changeset::Status::Added), ("old.bin".into(), crate::changeset::Status::Deleted));
        assert_eq!(listed(&set), [added.0, added.1]);
    }

    #[test]
    fn zip_renames_are_found_beside_unchanged_members() {
        let deflated = zip::CompressionMethod::Deflated;
        let (_d, set) = load("zip", &zip_of(&[("a", b"alpha\n", deflated), ("b", b"beta\n", deflated), ("old", b"moved\n", deflated)]),
            &zip_of(&[("a", b"alpha\n", deflated), ("b", b"beta\n", deflated), ("new", b"moved\n", deflated)]));
        assert_eq!(listed(&set), [("new".into(), crate::changeset::Status::Renamed { similarity: 100 })]);
    }

    #[test]
    fn zip_headers_are_checked_for_unchanged_members_too() {
        let stored = zip::CompressionMethod::Stored;
        let old = zip_of(&[("secret", b"key = 1\n", stored), ("README", b"v1\n", stored)]);
        let new = zip_of(&[("secret", b"key = 1\n", stored), ("README", b"v2\n", stored)]);
        // an encrypted member can't be read without its password, even when it is the same on both sides
        let (mut old_encrypted, mut new_encrypted) = (old.clone(), new.clone());
        for zip in [&mut old_encrypted, &mut new_encrypted] {
            patch_member(zip, "secret", |f| f[0] |= 1);
        }
        assert!(refused("zip", &old_encrypted, &new_encrypted).contains("assword"));
        // a method this build cannot decode
        let (mut old_unknown, mut new_unknown) = (old.clone(), new.clone());
        for zip in [&mut old_unknown, &mut new_unknown] {
            patch_member(zip, "secret", |f| f[2] = 97);
        }
        assert!(refused("zip", &old_unknown, &new_unknown).contains("not supported"));
        // owner decision pending: an unchanged member whose stored CRC-32 is wrong is no longer read, so it no longer
        // refuses the archive; a changed one still does (see members_are_compared_by_content_not_size_and_crc)
        let (mut old_bad, mut new_bad) = (old.clone(), new.clone());
        for zip in [&mut old_bad, &mut new_bad] {
            let at = zip.windows(8).position(|w| w == b"key = 1\n").unwrap();
            zip[at + 6] = b'0';
        }
        let (_d, set) = load("zip", &old_bad, &new_bad);
        assert_eq!(listed(&set), [("README".into(), crate::changeset::Status::Modified)]);
        assert!(members(Kind::Zip, &new_bad).unwrap_err().contains("checksum"));
    }

    #[test]
    fn every_zstd_frame_is_read() {
        let first = testing::tar(&[("README", b"v2\n")], Compression::None);
        // without its end blocks, the first frame ends after a whole member
        let first = &first[..first.len() - 1024];
        let second = testing::tar(&[("evil.sh", b"echo EVIL\n")], Compression::None);
        let skippable = [0x50, 0x2A, 0x4D, 0x18, 3, 0, 0, 0, 1, 2, 3];
        let new = [zstd_stored_frame(first), skippable.to_vec(), zstd_stored_frame(&second)].concat();
        let kind = Kind::Tar(Compression::Zstd);
        assert_eq!(members(kind, &new).unwrap().keys().collect::<Vec<_>>(), ["README", "evil.sh"]);
        assert_eq!(extract(kind, &new, "evil.sh").unwrap(), b"echo EVIL\n");
        assert_eq!(unpack(Compression::Zstd, &new, 1 << 20).unwrap().unwrap(), [first, &second[..]].concat());
        let old = zstd_stored_frame(&testing::tar(&[("README", b"v1\n")], Compression::None));
        let (_d, set) = load("tar.zst", &old, &new);
        let status = |s: &str| (s.to_string(), if s == "README" { crate::changeset::Status::Modified } else { crate::changeset::Status::Added });
        assert_eq!(listed(&set), [status("README"), status("evil.sh")]);
        // a skippable frame cut short is an error, not the end of the archive
        let cut = [zstd_stored_frame(first), skippable[..9].to_vec()].concat();
        assert!(unpack(Compression::Zstd, &cut, 1 << 20).unwrap_err().contains("truncated zstd skippable frame"));
    }

    /// A zip of `files` deflated at level 1, where [`testing::zip`] uses the default level.
    fn repacked(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1));
        for (path, content) in files {
            out.start_file(*path, options).unwrap();
            std::io::Write::write_all(&mut out, content).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    /// Change Set load of two changed archives that differ in one member: many small members, and a few large ones.
    /// `cargo test --release -p lenz --no-default-features --lib archive_change_set_release_probe -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn archive_change_set_release_probe() {
        // compressible text, about 3:1 under deflate, as source trees are
        let text = |seed: u64, len: usize| {
            let words = ["fn ", "let ", "self", ".read(", "archive", " = ", "member", "();\n", "    ", "Result<", ">", "match "];
            let (mut out, mut state) = (Vec::with_capacity(len + 16), seed.wrapping_mul(6364136223846793005) | 1);
            while out.len() < len {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                out.extend(words[(state >> 59) as usize % words.len()].as_bytes());
            }
            out.truncate(len);
            out
        };
        let status = |key: &str| {
            let status = std::fs::read_to_string("/proc/self/status").ok()?;
            status.lines().find_map(|l| l.strip_prefix(key)?.trim().strip_suffix("kB")?.trim().parse::<f64>().ok()).map(|kb| kb / 1024.)
        };
        let d = tempfile::tempdir().unwrap();
        let mut fixtures = Vec::new();
        for (label, count, len) in [("5000x2KiB", 5000, 2 << 10), ("32x2MiB", 32, 2 << 20)] {
            let old: Vec<(String, Vec<u8>)> = (0..count).map(|i| (format!("src/m{i:05}.rs"), text(i as u64, len))).collect();
            let mut new = old.clone();
            new[count / 2].1[len / 2] ^= 1;
            let changed = new[count / 2].0.clone();
            fn borrow(files: &[(String, Vec<u8>)]) -> Vec<(&str, &[u8])> {
                files.iter().map(|(n, c)| (n.as_str(), c.as_slice())).collect()
            }
            // `repacked.zip`: the new side deflated at another level, so no member's stored bytes match
            for ext in ["zip", "repacked.zip", "tar.gz", "tar.zst"] {
                let make = |files: &[(&str, &[u8])]| match ext {
                    "zip" => testing::zip(files),
                    "repacked.zip" => testing::zip(files),
                    "tar.gz" => testing::tar(files, Compression::Gzip),
                    // stored zstd blocks: measures framing and member hashing, not zstd decompression
                    _ => zstd_stored_frame(&testing::tar(files, Compression::None)),
                };
                let (left, right) = (d.path().join(format!("{label}-old.{ext}")), d.path().join(format!("{label}-new.{ext}")));
                std::fs::write(&left, make(&borrow(&old))).unwrap();
                let new_bytes = if ext == "repacked.zip" { repacked(&borrow(&new)) } else { make(&borrow(&new)) };
                std::fs::write(&right, new_bytes).unwrap();
                fixtures.push((format!("{label}.{ext}"), count * len, changed.clone(), left, right));
            }
        }
        let mut cases = Vec::new();
        for (case, unpacked, changed, left, right) in fixtures {
            let spec = crate::changeset::Spec::Plain { left: left.clone(), right: right.clone() };
            let (mut ms, mut peak, mut absolute) = (Vec::new(), Vec::new(), Vec::new());
            for sample in 0..12 {
                // resets VmHWM to the current RSS, so the peak below is this load's own
                let _ = std::fs::write("/proc/self/clear_refs", "5");
                let before = status("VmRSS:");
                let started = std::time::Instant::now();
                let set = crate::changeset::ChangeSet::load(&spec, d.path()).unwrap();
                let elapsed = started.elapsed().as_secs_f64() * 1000.;
                assert_eq!(set.comparisons.len(), 1, "{case}");
                assert!(set.comparisons[0].in_archive() && set.comparisons[0].path.ends_with(&changed), "{case}");
                assert_eq!(set.comparisons[0].status, crate::changeset::Status::Modified);
                // the first sample warms the page cache and allocator
                if sample > 0 {
                    ms.push(elapsed);
                    peak.extend(before.zip(status("VmHWM:")).map(|(before, peak)| peak - before));
                    absolute.extend(status("VmHWM:"));
                }
            }
            let bytes = [std::fs::metadata(&left).unwrap().len(), std::fs::metadata(&right).unwrap().len()];
            cases.push(serde_json::json!({"case": case, "archive_bytes": bytes, "unpacked_bytes": unpacked, "load_ms": ms,
                "peak_rss_growth_mib": peak, "peak_rss_mib": absolute, "rss_mib": crate::perf::rss_mib()}));
        }
        println!("LENZ_ARCHIVE_PROBE {}", serde_json::json!({"version": 2, "loadavg": std::fs::read_to_string("/proc/loadavg").ok(), "cases": cases}));
    }

}

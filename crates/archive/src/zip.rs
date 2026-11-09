use std::{collections::BTreeMap, io::{Cursor, Read}, sync::Arc};
use ::zip::{CompressionMethod, ZipArchive, ZipWriter};
use crate::{Error, Limits, Member, MemberId, MemberKind, check, codecs, invalid, normalize, read_bounded};

type Archive = ZipArchive<Cursor<Arc<[u8]>>>;
#[derive(Clone)]
pub(crate) struct Data { archive: Archive, bytes: Arc<[u8]>, pub(crate) metadata_bytes: usize }

fn zip_error(error: ::zip::result::ZipError) -> Error {
    match error {
        ::zip::result::ZipError::UnsupportedArchive(reason) => Error::Unsupported(reason.into()),
        ::zip::result::ZipError::CompressionMethodNotSupported(method) => Error::Unsupported(format!("ZIP compression method {method} is unsupported")),
        error => invalid(error),
    }
}

pub(crate) fn open(bytes: Arc<[u8]>, limits: Limits, revision: u64, cancel: &dyn Fn() -> bool) -> Result<(Data, Vec<Member>, Option<String>), Error> {
    // Check the raw declared count before the ZIP crate allocates its name-keyed metadata map.
    let end = end_info(&bytes)?;
    if end.count > limits.max_members as u64 { return Err(Error::Limit("member count")); }
    let mut archive = ZipArchive::new(Cursor::new(bytes.clone())).map_err(invalid)?;
    let mut blocked = if archive.has_duplicate_names() {
        Some("ZIP has duplicate stored names; browsing resolves the last name and preserving edits is unsupported".to_owned())
    } else if archive.offset() != 0 {
        Some("preserving a ZIP executable/prefix while editing is unsupported".to_owned())
    } else if end.trailing {
        Some("preserving trailing ZIP data while editing is unsupported".to_owned())
    } else if archive.raw_zip64_extensible_data_sector().is_some_and(|v| !v.is_empty()) {
        Some("preserving ZIP64 extensible metadata while editing is unsupported".to_owned())
    } else { None };
    let mut members = Vec::new();
    let mut total = 0u64;
    let mut metadata_bytes = 0usize;
    let mut central_end = 0usize;
    let mut first_local = u64::MAX;
    for ordinal in 0..archive.len() {
        check(cancel)?;
        let file = archive.by_index_raw(ordinal).map_err(invalid)?;
        let name = file.name().map_err(invalid)?.into_owned();
        let path = normalize(&name);
        let mode = file.unix_mode().unwrap_or(0) & 0o170000;
        let kind = if file.is_dir() { MemberKind::Directory }
            else if mode == 0o120000 { MemberKind::Symlink }
            else if mode == 0 || mode == 0o100000 { MemberKind::File }
            else { MemberKind::Other };
        let metadata = Metadata::read(&bytes, file.header_start(), file.central_header_start())?;
        metadata_bytes = metadata_bytes.saturating_add(metadata.local_extra.len().saturating_add(metadata.central_extra.len()).saturating_add(metadata.comment.len()).saturating_add(metadata.name.len()).saturating_mul(3)).saturating_add(512);
        central_end = central_end.max(metadata.central_end);
        first_local = first_local.min(file.header_start());
        if metadata.name != metadata.local_name {
            blocked = Some("ZIP local and central stored names disagree; preserving edits is unsupported".into());
        }
        if metadata.ambiguous {
            blocked = Some("ZIP local and central integrity metadata disagree; preserving edits is unsupported".into());
        }
        if extra_fields(metadata.local_extra).is_err() || extra_fields(metadata.central_extra).is_err() {
            blocked = Some("preserving opaque ZIP extra-field padding or malformed optional metadata while editing is unsupported".into());
        }
        if metadata.comment != file.comment().as_bytes() {
            blocked = Some("preserving a non-UTF-8 ZIP member comment while editing is unsupported".into());
        }
        if file.encrypted() { blocked = Some("preserving encrypted ZIP entries while editing is unsupported".into()); }
        total = total.checked_add(file.size()).ok_or(Error::Limit("unpacked bytes"))?;
        if total > limits.max_unpacked { return Err(Error::Limit("unpacked bytes")); }
        let writable_method = matches!(file.compression(), CompressionMethod::Stored | CompressionMethod::Deflated | CompressionMethod::Bzip2 | CompressionMethod::Xz);
        let reason = if kind != MemberKind::File { Some("only regular archive files can be edited") }
            else if path.is_none() { Some("archive member has no supported safe virtual path") }
            else if !writable_method { Some("this ZIP compression method cannot be written while preserving its format") }
            else if std::str::from_utf8(file.name_raw()).ok() != Some(name.as_str()) { Some("editing this ZIP filename encoding is unsupported") }
            else if file.size() > limits.max_member { Some("archive member exceeds the member byte limit") }
            else { None };
        members.push(Member { id: MemberId { revision, ordinal }, stored_path: file.name_raw().to_vec(), path, kind, size: file.size(), crc32: Some(file.crc32()), read_only_reason: reason.map(str::to_owned) });
    }
    if first_local != u64::MAX && first_local != 0 { blocked.get_or_insert_with(|| "preserving a ZIP prefix while editing is unsupported".into()); }
    let central_start = usize::try_from(archive.central_directory_start()).map_err(|_| invalid("ZIP central directory offset overflow"))?;
    if central_start >= 16 && bytes.get(central_start - 16..central_start) == Some(&b"APK Sig Block 42"[..]) {
        blocked = Some("editing a signed APK/ZIP container is unsupported".into());
    }
    if central_end < end.at && central_end != 0 {
        let tail = bytes.get(central_end..end.at).ok_or_else(|| invalid("ZIP central directory extends past its footer"))?;
        if !tail.is_empty() && !tail.starts_with(b"PK\x06\x06") {
            blocked = Some("preserving ZIP signatures or unknown central-directory records while editing is unsupported".into());
        }
    }
    Ok((Data { archive, bytes, metadata_bytes }, members, blocked))
}

pub(crate) fn read(data: &Data, member: &Member, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    let mut archive = data.archive.clone(); // Reader and parsed metadata share immutable Arc ownership.
    let mut raw = archive.by_index_raw(member.id.ordinal()).map_err(invalid)?;
    if raw.encrypted() { return Err(Error::Unsupported("encrypted ZIP members require a password and cannot be opened".into())); }
    let method = raw.compression();
    let mut output = Vec::new();
    if method == CompressionMethod::Xz {
        let mut decoder = lzma_rust2::XzReader::new_mem_limit(&mut raw, false, (codecs::DECODER_MEMORY >> 10) as u32);
        read_bounded(&mut decoder, &mut output, limits.max_member, "member bytes", cancel)?;
    } else {
        if method == CompressionMethod::Lzma {
            let mut header = [0; 9];
            raw.read_exact(&mut header)?;
            if u16::from_le_bytes([header[2], header[3]]) != 5 { return Err(invalid("invalid ZIP LZMA properties")); }
            let dictionary = u32::from_le_bytes(header[5..9].try_into().map_err(invalid)?) as u64;
            if dictionary > codecs::DECODER_MEMORY { return Err(Error::Limit("decoder memory")); }
        }
        drop(raw);
        let mut file = archive.by_index(member.id.ordinal()).map_err(zip_error)?;
        read_bounded(&mut file, &mut output, limits.max_member, "member bytes", cancel)?;
    }
    if output.len() as u64 != member.size || crc32fast::hash(&output) != member.crc32.unwrap_or(0) {
        return Err(invalid("ZIP member size or CRC mismatch"));
    }
    check(cancel)?;
    Ok(output)
}

pub(crate) fn replace(data: &Data, members: &[Member], updates: &BTreeMap<usize, &[u8]>, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    let mut archive = data.archive.clone();
    let mut writer = ZipWriter::new(codecs::Output::new(limits.max_archive, "stored bytes"));
    writer.set_raw_comment(archive.comment().into()).map_err(invalid)?;
    for (ordinal, member) in members.iter().enumerate() {
        check(cancel)?;
        let file = archive.by_index_raw(ordinal).map_err(invalid)?;
        if let Some(bytes) = updates.get(&ordinal) {
            let metadata = Metadata::read(&data.bytes, file.header_start(), file.central_header_start())?;
            let mut options = file.options().large_file(bytes.len() as u64 >= u32::MAX as u64).into_full_options().with_file_comment(file.comment());
            let local = extra_fields(metadata.local_extra)?;
            let central = extra_fields(metadata.central_extra)?;
            for &(id, value) in &local {
                if id != 1 { options.add_extra_field(id, value, false).map_err(invalid)?; }
            }
            for &(id, value) in &central {
                if id != 1 && !local.contains(&(id, value)) { options.add_extra_field(id, value, true).map_err(invalid)?; }
            }
            let name = std::str::from_utf8(&member.stored_path).map_err(invalid)?;
            writer.start_file(name, options).map_err(invalid)?;
            codecs::write_chunks(&mut writer, bytes, cancel)?;
        } else {
            // Raw compressed data, not a decompressed buffer. Cancellation is checked before and
            // after each copy and between the bounded comparison chunks during verification.
            writer.raw_copy_file(file).map_err(invalid)?;
        }
    }
    let output = writer.finish().map_err(invalid)?.finish()?;
    check(cancel)?;
    verify(data, &output, members, updates, limits, cancel)?;
    Ok(output)
}

fn verify(data: &Data, output: &[u8], members: &[Member], updates: &BTreeMap<usize, &[u8]>, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    let mut original = data.archive.clone();
    let mut candidate = ZipArchive::new(Cursor::new(output)).map_err(invalid)?;
    if candidate.len() != members.len() || candidate.has_duplicate_names() || candidate.comment() != original.comment() {
        return Err(invalid("ZIP rewrite changed entry identities or archive comment"));
    }
    let mut total = 0u64;
    for (ordinal, member) in members.iter().enumerate() {
        check(cancel)?;
        let mut old = original.by_index_raw(ordinal).map_err(invalid)?;
        let mut new = candidate.by_index_raw(ordinal).map_err(invalid)?;
        total = total.checked_add(new.size()).ok_or(Error::Limit("unpacked bytes"))?;
        if total > limits.max_unpacked { return Err(Error::Limit("unpacked bytes")); }
        let old_meta = Metadata::read(&data.bytes, old.header_start(), old.central_header_start())?;
        let new_meta = Metadata::read(output, new.header_start(), new.central_header_start())?;
        if !old_meta.preserved(&new_meta)? || old.name_raw() != new.name_raw() || old.compression() != new.compression() {
            return Err(Error::Unsupported("the ZIP writer cannot preserve this entry's original metadata; archive left unchanged".into()));
        }
        if let Some(expected) = updates.get(&ordinal) {
            drop(new);
            let mut decoded = candidate.by_index(ordinal).map_err(invalid)?;
            codecs::verify_reader(&mut decoded, expected, cancel)?;
        } else {
            if old.crc32() != new.crc32() || old.size() != new.size() || old.compressed_size() != new.compressed_size() {
                return Err(invalid("ZIP rewrite changed an untouched member"));
            }
            // Byte-for-byte raw payload comparison avoids decompressing any untouched member.
            let mut a = [0; 64 << 10];
            let mut b = [0; 64 << 10];
            let mut left = old.compressed_size();
            while left != 0 {
                check(cancel)?;
                let count = a.len().min(left.min(usize::MAX as u64) as usize);
                old.read_exact(&mut a[..count])?;
                new.read_exact(&mut b[..count])?;
                if a[..count] != b[..count] { return Err(invalid("ZIP rewrite changed an untouched compressed payload")); }
                left -= count as u64;
            }
        }
        if member.stored_path != old.name_raw() { return Err(invalid("ZIP member identity changed")); }
    }
    Ok(())
}

struct Metadata<'a> {
    name: &'a [u8], local_name: &'a [u8], comment: &'a [u8],
    local_extra: &'a [u8], central_extra: &'a [u8],
    attributes: &'a [u8], stamp: &'a [u8], local_stamp: &'a [u8], system: u8,
    central_end: usize,
    ambiguous: bool,
}
impl<'a> Metadata<'a> {
    fn read(bytes: &'a [u8], local: u64, central: u64) -> Result<Self, Error> {
        let local = usize::try_from(local).map_err(|_| invalid("ZIP local offset overflow"))?;
        let central = usize::try_from(central).map_err(|_| invalid("ZIP central offset overflow"))?;
        let l = bytes.get(local..).ok_or_else(|| invalid("invalid ZIP local header offset"))?;
        let c = bytes.get(central..).ok_or_else(|| invalid("invalid ZIP central header offset"))?;
        if l.get(..4) != Some(b"PK\x03\x04") || c.get(..4) != Some(b"PK\x01\x02") { return Err(invalid("invalid ZIP local or central header")); }
        let ln = word(l, 26)? as usize;
        let le = word(l, 28)? as usize;
        let cn = word(c, 28)? as usize;
        let ce = word(c, 30)? as usize;
        let cc = word(c, 32)? as usize;
        let local_name = l.get(30..30 + ln).ok_or_else(|| invalid("truncated ZIP local name"))?;
        let local_extra = l.get(30 + ln..30 + ln + le).ok_or_else(|| invalid("truncated ZIP local metadata"))?;
        let name = c.get(46..46 + cn).ok_or_else(|| invalid("truncated ZIP central name"))?;
        let central_extra = c.get(46 + cn..46 + cn + ce).ok_or_else(|| invalid("truncated ZIP central metadata"))?;
        let comment = c.get(46 + cn + ce..46 + cn + ce + cc).ok_or_else(|| invalid("truncated ZIP comment"))?;
        let descriptor = word(l, 6)? & 8 != 0;
        let ambiguous = word(l, 6)? != word(c, 8)? || word(l, 8)? != word(c, 10)? || (!descriptor && (
            dword(l, 14)? != dword(c, 16)?
            || (dword(l, 18)? != u32::MAX && dword(c, 20)? != u32::MAX && dword(l, 18)? != dword(c, 20)?)
            || (dword(l, 22)? != u32::MAX && dword(c, 24)? != u32::MAX && dword(l, 22)? != dword(c, 24)?)));
        Ok(Self { name, local_name, comment, local_extra, central_extra,
            attributes: c.get(36..42).ok_or_else(|| invalid("truncated ZIP attributes"))?,
            stamp: c.get(12..16).ok_or_else(|| invalid("truncated ZIP timestamp"))?,
            local_stamp: l.get(10..14).ok_or_else(|| invalid("truncated ZIP timestamp"))?,
            system: *c.get(5).ok_or_else(|| invalid("truncated ZIP version"))?, central_end: central + 46 + cn + ce + cc, ambiguous })
    }
    fn preserved(&self, other: &Self) -> Result<bool, Error> {
        // ZIP64's size/offset fields must be rebuilt for the candidate layout. Every other extra
        // retains its exact bytes and order, including unknown application-specific metadata.
        Ok(self.name == other.name && self.local_name == other.local_name && self.comment == other.comment
            && self.attributes == other.attributes && self.stamp == other.stamp && self.local_stamp == other.local_stamp && self.system == other.system
            && non_zip64_extras(self.local_extra)? == non_zip64_extras(other.local_extra)? && non_zip64_extras(self.central_extra)? == non_zip64_extras(other.central_extra)?)
    }
}

fn non_zip64_extras(raw: &[u8]) -> Result<Vec<(u16, &[u8])>, Error> {
    Ok(extra_fields(raw)?.into_iter().filter(|(id, _)| *id != 1).collect())
}

fn extra_fields(mut bytes: &[u8]) -> Result<Vec<(u16, &[u8])>, Error> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let id = word(bytes, 0)?;
        let size = word(bytes, 2)? as usize;
        let value = bytes.get(4..4 + size).ok_or_else(|| invalid("truncated ZIP extra field"))?;
        out.push((id, value));
        bytes = &bytes[4 + size..];
    }
    Ok(out)
}
fn word(bytes: &[u8], at: usize) -> Result<u16, Error> {
    let b = bytes.get(at..at + 2).ok_or_else(|| invalid("truncated ZIP header"))?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}
fn wide(bytes: &[u8], at: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(bytes.get(at..at + 8).ok_or_else(|| invalid("truncated ZIP64 footer"))?.try_into().map_err(invalid)?))
}
fn dword(bytes: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(bytes.get(at..at + 4).ok_or_else(|| invalid("truncated ZIP header"))?.try_into().map_err(invalid)?))
}
struct End { at: usize, count: u64, trailing: bool }
fn end_info(bytes: &[u8]) -> Result<End, Error> {
    let at = (bytes.len().saturating_sub(65_557)..bytes.len().saturating_sub(21)).rev().find(|at| {
        bytes.get(*at..*at + 4) == Some(b"PK\x05\x06") && word(bytes, *at + 20).ok().is_some_and(|n| *at + 22 + n as usize <= bytes.len())
    }).ok_or_else(|| invalid("ZIP end-of-directory record not found"))?;
    let mut count = word(bytes, at + 10)? as u64;
    if count == u16::MAX as u64 {
        let locator = at.checked_sub(20).ok_or_else(|| invalid("missing ZIP64 locator"))?;
        if bytes.get(locator..locator + 4) == Some(b"PK\x06\x07") {
            let position = usize::try_from(wide(bytes, locator + 8)?).map_err(|_| invalid("ZIP64 offset overflow"))?;
            if bytes.get(position..position.saturating_add(4)) != Some(b"PK\x06\x06") { return Err(invalid("invalid ZIP64 end record")); }
            count = wide(bytes, position + 32)?;
        }
    }
    Ok(End { at, count, trailing: at + 22 + word(bytes, at + 20)? as usize != bytes.len() })
}

use std::{collections::BTreeMap, io::Write, ops::Range, sync::Arc};
use crate::{Compression, Error, Limits, Member, MemberId, MemberKind, check, codecs, invalid, normalize};

#[derive(Clone)]
pub(crate) struct Data {
    pub(crate) raw: Arc<[u8]>,
    records: Arc<[Record]>,
    compression: Compression,
    pub(crate) gzip_header: Option<Arc<[u8]>>,
}
#[derive(Clone)]
struct Record { span: Range<usize>, content: Range<usize>, link: Option<Vec<u8>>, readable: bool }

type Pax = BTreeMap<Vec<u8>, Vec<u8>>;

pub(crate) fn open(compression: Compression, bytes: Arc<[u8]>, limits: Limits, revision: u64, cancel: &dyn Fn() -> bool) -> Result<(Data, Vec<Member>, Option<String>), Error> {
    let (raw, gzip_header, blocked) = if compression == Compression::None {
        if bytes.len() as u64 > limits.max_unpacked { return Err(Error::Limit("unpacked bytes")); }
        (bytes, None, None)
    } else {
        let decoded = codecs::decode(compression, &bytes, limits, cancel)?;
        (Arc::from(decoded.bytes), decoded.gzip_header.map(Arc::from), decoded.blocked)
    };
    let (records, members) = parse(&raw, limits, revision, cancel)?;
    Ok((Data { raw, records: records.into(), compression, gzip_header }, members, blocked))
}

fn parse(raw: &[u8], limits: Limits, revision: u64, cancel: &dyn Fn() -> bool) -> Result<(Vec<Record>, Vec<Member>), Error> {
    let mut records = Vec::new();
    let mut members = Vec::new();
    let mut global = Pax::new();
    let mut local = Pax::new();
    let (mut long_path, mut long_link) = (None, None);
    let mut at = 0usize;
    let mut metadata_bytes = 0u64;
    while at < raw.len() {
        check(cancel)?;
        let block = raw.get(at..at.checked_add(512).ok_or_else(|| invalid("tar position overflow"))?).ok_or_else(|| invalid("truncated tar header"))?;
        if block.iter().all(|b| *b == 0) { break; } // preserve every trailing byte, including padding.
        if records.len() >= limits.max_members { return Err(Error::Limit("member count")); }
        let header = ::tar::Header::from_byte_slice(block);
        let sum: u32 = block.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u32 }).sum();
        if header.cksum()? != sum { return Err(invalid("tar header checksum mismatch")); }
        let entry_type = header.entry_type();
        let code = entry_type.as_byte();
        let extension = matches!(code, b'x' | b'g' | b'L' | b'K');
        let field = |key: &[u8]| local.get(key).or_else(|| global.get(key)).filter(|v| !v.is_empty());
        let pax_size = if extension { None } else { field(b"size").map(|v| decimal(v)).transpose()? };
        let size = pax_size.unwrap_or(header.entry_size()?);
        let mut data_start = at + 512;
        // Old GNU sparse extension blocks precede the stored sparse payload and are not in size.
        if entry_type.is_gnu_sparse() && header.as_gnu().is_some_and(|h| h.is_extended()) {
            loop {
                check(cancel)?;
                let extension = raw.get(data_start..data_start.checked_add(512).ok_or_else(|| invalid("tar sparse position overflow"))?).ok_or_else(|| invalid("truncated GNU sparse extension"))?;
                data_start += 512;
                if extension[504] == 0 { break; }
            }
        }
        let size_usize = usize::try_from(size).map_err(|_| Error::Limit("unpacked bytes"))?;
        let end = data_start.checked_add(size_usize).ok_or_else(|| invalid("tar member size overflow"))?;
        let padded = size_usize.checked_add(511).ok_or_else(|| invalid("tar padding overflow"))? & !511;
        let next = data_start.checked_add(padded).ok_or_else(|| invalid("tar position overflow"))?;
        let content = raw.get(data_start..end).ok_or_else(|| invalid("truncated tar member"))?;
        if next > raw.len() { return Err(invalid("truncated tar member padding")); }
        let stored_path = header.path_bytes().into_owned();
        let effective_path = if extension { &stored_path[..] } else { field(b"path").map(Vec::as_slice).or(long_path.as_deref()).unwrap_or(&stored_path) };
        let path = std::str::from_utf8(effective_path).ok().and_then(normalize);
        let header_link = header.link_name_bytes().map(|n| n.into_owned());
        let link = field(b"linkpath").cloned().or_else(|| long_link.clone()).or(header_link);
        // A global PAX path/link override can repeat a large value across thousands of tiny raw
        // headers. Bound owned index metadata as well as the raw stream, before retaining each copy.
        let cost = 512u64.saturating_add(stored_path.len() as u64).saturating_add(effective_path.len() as u64).saturating_add(link.as_ref().map_or(0, |v| v.len() as u64));
        metadata_bytes = metadata_bytes.checked_add(cost).ok_or(Error::Limit("index metadata bytes"))?;
        if metadata_bytes > limits.max_unpacked { return Err(Error::Limit("index metadata bytes")); }
        let sparse = entry_type.is_gnu_sparse() || global.keys().chain(local.keys()).any(|k| k.starts_with(b"GNU.sparse") || k.as_slice() == b"SCHILY.realsize")
            || field(b"SCHILY.filetype").is_some_and(|v| v == b"sparse");
        let kind = if extension { MemberKind::Metadata }
            else if sparse { MemberKind::Other }
            else if entry_type.is_file() || code == b'7' { MemberKind::File }
            else if entry_type.is_dir() { MemberKind::Directory }
            else if entry_type.is_symlink() { MemberKind::Symlink }
            else if entry_type.is_hard_link() { MemberKind::HardLink }
            else { MemberKind::Other };
        let read_only_reason = if sparse { Some("editing sparse tar members is unsupported") }
            else if kind != MemberKind::File { Some("only regular archive files can be edited") }
            else if path.is_none() { Some("archive member has no supported safe UTF-8 virtual path") }
            else if pax_size.is_some() { Some("editing tar members with a PAX size override is unsupported") }
            else if size > limits.max_member { Some("archive member exceeds the member byte limit") }
            else { None };
        let display_size = if matches!(kind, MemberKind::Symlink | MemberKind::HardLink) { link.as_ref().map_or(0, |v| v.len() as u64) } else { size };
        members.push(Member { id: MemberId { revision, ordinal: records.len() }, stored_path, path, kind, size: display_size, crc32: None, read_only_reason: read_only_reason.map(str::to_owned) });
        records.push(Record { span: at..next, content: data_start..end, link: if matches!(kind, MemberKind::Symlink | MemberKind::HardLink) { link } else { None }, readable: kind != MemberKind::Other });
        match code {
            b'x' => update_pax(&mut local, content, false)?,
            b'g' => update_pax(&mut global, content, true)?,
            b'L' => { if content.len() > 1 << 20 { return Err(Error::Limit("metadata bytes")); } long_path = Some(trim_nul(content).to_vec()); }
            b'K' => { if content.len() > 1 << 20 { return Err(Error::Limit("metadata bytes")); } long_link = Some(trim_nul(content).to_vec()); }
            _ => { local.clear(); long_path = None; long_link = None; }
        }
        at = next;
    }
    Ok((records, members))
}

fn trim_nul(value: &[u8]) -> &[u8] { value.strip_suffix(&[0]).unwrap_or(value) }
fn decimal(value: &[u8]) -> Result<u64, Error> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) { return Err(invalid("invalid PAX decimal value")); }
    value.iter().try_fold(0u64, |n, b| n.checked_mul(10).and_then(|n| n.checked_add((b - b'0') as u64)).ok_or_else(|| invalid("PAX decimal overflow")))
}
fn update_pax(fields: &mut Pax, mut bytes: &[u8], global: bool) -> Result<(), Error> {
    if bytes.len() > 16 << 20 { return Err(Error::Limit("metadata bytes")); }
    while !bytes.is_empty() {
        let space = bytes.iter().position(|b| *b == b' ').ok_or_else(|| invalid("invalid PAX record length"))?;
        let length = usize::try_from(decimal(&bytes[..space])?).map_err(|_| invalid("PAX record overflow"))?;
        if length <= space + 1 { return Err(invalid("invalid PAX record length")); }
        let record = bytes.get(space + 1..length).ok_or_else(|| invalid("truncated PAX record"))?;
        let record = record.strip_suffix(b"\n").ok_or_else(|| invalid("unterminated PAX record"))?;
        let equals = record.iter().position(|b| *b == b'=').ok_or_else(|| invalid("invalid PAX key/value"))?;
        let (key, value) = (&record[..equals], &record[equals + 1..]);
        if key.is_empty() { return Err(invalid("empty PAX key")); }
        // Unknown metadata stays in its original raw record. Retain only fields needed to interpret
        // following payloads, preventing an extension with millions of unknown keys allocating a map.
        if matches!(key, b"path" | b"linkpath" | b"size" | b"SCHILY.filetype" | b"SCHILY.realsize") || key.starts_with(b"GNU.sparse") {
            if value.len() > 1 << 20 { return Err(Error::Limit("metadata bytes")); }
            if global && value.is_empty() { fields.remove(key); }
            else { fields.insert(key.to_vec(), value.to_vec()); }
        }
        bytes = &bytes[length..];
    }
    Ok(())
}

pub(crate) fn read(data: &Data, member: &Member, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    check(cancel)?;
    let record = data.records.get(member.id.ordinal()).ok_or(Error::StaleMember)?;
    if !record.readable { return Err(Error::Unsupported("reading sparse or special tar members as regular files is unsupported".into())); }
    let bytes = record.link.as_deref().unwrap_or(&data.raw[record.content.clone()]);
    if bytes.len() as u64 > limits.max_member { return Err(Error::Limit("member bytes")); }
    let mut output = Vec::new();
    crate::read_bounded(&mut &bytes[..], &mut output, limits.max_member, "member bytes", cancel)?;
    Ok(output)
}

pub(crate) fn replace(data: &Data, updates: &BTreeMap<usize, &[u8]>, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    let mut output = codecs::Output::new(limits.max_unpacked, "unpacked bytes");
    for (ordinal, record) in data.records.iter().enumerate() {
        check(cancel)?;
        if let Some(bytes) = updates.get(&ordinal) {
            let mut header = ::tar::Header::new_old();
            header.as_mut_bytes().copy_from_slice(&data.raw[record.span.start..record.span.start + 512]);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            output.write_all(header.as_bytes())?;
            codecs::write_chunks(&mut output, bytes, cancel)?;
            let padding = (512 - bytes.len() % 512) % 512;
            output.write_all(&[0; 512][..padding])?;
        } else { codecs::write_chunks(&mut output, &data.raw[record.span.clone()], cancel)?; }
    }
    let end = data.records.last().map_or(0, |r| r.span.end);
    codecs::write_chunks(&mut output, &data.raw[end..], cancel)?;
    let raw = output.finish()?;
    let (records, _) = parse(&raw, limits, 0, cancel)?;
    if records.len() != data.records.len() { return Err(invalid("tar rewrite changed the physical entry count")); }
    for (&ordinal, expected) in updates {
        if &raw[records[ordinal].content.clone()] != *expected { return Err(invalid("tar rewrite failed content verification")); }
    }
    let encoded = codecs::encode(data.compression, &raw, data.gzip_header.as_deref(), limits, cancel)?;
    // Compressed wrapper checksums and all concatenated frames are validated on the candidate.
    if data.compression != Compression::None {
        codecs::verify_encoded(data.compression, &encoded, &raw, cancel)?;
    }
    Ok(encoded)
}

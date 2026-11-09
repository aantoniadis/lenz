use std::io::{self, Read, Seek, SeekFrom, Write};
use crate::{Compression, Error, Limits, check, invalid, read_bounded};

// Payload limits alone do not bound an LZMA/Zstd dictionary declared in an untrusted header.
pub(crate) const DECODER_MEMORY: u64 = 128 << 20;

pub(crate) struct Decoded { pub bytes: Vec<u8>, pub gzip_header: Option<Vec<u8>>, pub blocked: Option<String> }

pub(crate) fn decode(compression: Compression, bytes: &[u8], limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Decoded, Error> {
    let mut output = Vec::new();
    let mut gzip_header = None;
    let mut blocked = None;
    match compression {
        Compression::None => read_bounded(&mut &bytes[..], &mut output, limits.max_unpacked, "unpacked bytes", cancel)?,
        Compression::Gzip => {
            gzip_header = Some(gzip_header_bytes(bytes)?.to_vec());
            let mut remaining = bytes;
            let mut count = 0;
            while !remaining.is_empty() {
                check(cancel)?;
                let mut decoder = flate2::bufread::GzDecoder::new(remaining);
                read_bounded(&mut decoder, &mut output, limits.max_unpacked, "unpacked bytes", cancel)?;
                let rest = decoder.into_inner();
                if rest.len() >= remaining.len() { return Err(invalid("gzip decoder made no progress")); }
                remaining = rest;
                count += 1;
            }
            if count > 1 { blocked = Some("preserving a multi-stream gzip wrapper while editing is unsupported".into()); }
        }
        Compression::Bzip2 => {
            let mut decoder = bzip2::bufread::MultiBzDecoder::new(bytes);
            read_bounded(&mut decoder, &mut output, limits.max_unpacked, "unpacked bytes", cancel)?;
            if !decoder.into_inner().is_empty() { return Err(invalid("trailing data after bzip2 stream")); }
        }
        Compression::Xz => {
            let mut decoder = lzma_rust2::XzReader::new_mem_limit(bytes, true, (DECODER_MEMORY >> 10) as u32);
            read_bounded(&mut decoder, &mut output, limits.max_unpacked, "unpacked bytes", cancel)?;
            if !decoder.into_inner().is_empty() { return Err(invalid("trailing data after xz stream")); }
        }
        Compression::Zstd => {
            let mut remaining = bytes;
            while !remaining.is_empty() {
                check(cancel)?;
                // Skippable frames carry arbitrary application metadata; retain browsing but refuse
                // a rewrite that would discard them when constructing a fresh compressed wrapper.
                if remaining.len() >= 8 {
                    let magic = u32::from_le_bytes(remaining[..4].try_into().map_err(invalid)?);
                    if magic & 0xfffffff0 == 0x184d2a50 {
                        let size = u32::from_le_bytes(remaining[4..8].try_into().map_err(invalid)?) as usize;
                        remaining = remaining.get(8..).and_then(|r| r.get(size..)).ok_or_else(|| invalid("truncated zstd skippable frame"))?;
                        blocked = Some("preserving zstd skippable-frame metadata while editing is unsupported".into());
                        continue;
                    }
                }
                let mut decoder = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(remaining, DECODER_MEMORY).map_err(invalid)?;
                read_bounded(&mut decoder, &mut output, limits.max_unpacked, "unpacked bytes", cancel)?;
                let rest = decoder.into_inner();
                if rest.len() >= remaining.len() { return Err(invalid("zstd decoder made no progress")); }
                remaining = rest;
            }
        }
    }
    Ok(Decoded { bytes: output, gzip_header, blocked })
}

pub(crate) fn encode(compression: Compression, bytes: &[u8], gzip_header: Option<&[u8]>, limits: Limits, cancel: &dyn Fn() -> bool) -> Result<Vec<u8>, Error> {
    check(cancel)?;
    let mut output = Output::new(limits.max_archive, "stored bytes");
    match compression {
        Compression::None => write_chunks(&mut output, bytes, cancel)?,
        Compression::Gzip => {
            output.write_all(gzip_header.unwrap_or(&[31, 139, 8, 0, 0, 0, 0, 0, 0, 255]))?;
            let mut encoder = flate2::write::DeflateEncoder::new(output, flate2::Compression::default());
            write_chunks(&mut encoder, bytes, cancel)?;
            output = encoder.finish()?;
            output.write_all(&crc32fast::hash(bytes).to_le_bytes())?;
            output.write_all(&(bytes.len() as u32).to_le_bytes())?;
        }
        Compression::Bzip2 => {
            let mut encoder = bzip2::write::BzEncoder::new(output, bzip2::Compression::default());
            write_chunks(&mut encoder, bytes, cancel)?;
            output = encoder.finish()?;
        }
        Compression::Xz => {
            let mut encoder = lzma_rust2::XzWriter::new(output, lzma_rust2::XzOptions::default())?;
            write_chunks(&mut encoder, bytes, cancel)?;
            output = encoder.finish()?;
        }
        Compression::Zstd => {
            // ruzstd unwraps encoder IO. This source and sink never return an IO error: cancellation
            // becomes EOF and excess output is discarded, then reported explicitly after finishing.
            let mut source = Cancellable { bytes, cancel, cancelled: false };
            ruzstd::encoding::compress(&mut source, &mut output, ruzstd::encoding::CompressionLevel::Fastest);
            if source.cancelled { return Err(Error::Cancelled); }
        }
    }
    check(cancel)?;
    output.finish()
}

pub(crate) fn write_chunks(writer: &mut dyn Write, bytes: &[u8], cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    for chunk in bytes.chunks(64 << 10) { check(cancel)?; writer.write_all(chunk)?; }
    Ok(())
}

pub(crate) fn verify_encoded(compression: Compression, bytes: &[u8], expected: &[u8], cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    let mut reader: Box<dyn Read + '_> = match compression {
        Compression::None => Box::new(bytes),
        Compression::Gzip => Box::new(flate2::read::MultiGzDecoder::new(bytes)),
        Compression::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(bytes)),
        Compression::Xz => Box::new(lzma_rust2::XzReader::new_mem_limit(bytes, true, (DECODER_MEMORY >> 10) as u32)),
        // The encoder above emits exactly one frame; source indexing separately handles multi-frame input.
        Compression::Zstd => Box::new(ruzstd::decoding::StreamingDecoder::new_with_max_window_size(bytes, DECODER_MEMORY).map_err(invalid)?),
    };
    verify_reader(&mut reader, expected, cancel)
}

pub(crate) fn verify_reader(reader: &mut dyn Read, expected: &[u8], cancel: &dyn Fn() -> bool) -> Result<(), Error> {
    let mut buffer = [0; 64 << 10];
    let mut at = 0usize;
    loop {
        check(cancel)?;
        let n = reader.read(&mut buffer)?;
        if n == 0 { break; }
        let end = at.checked_add(n).ok_or_else(|| invalid("archive verification position overflow"))?;
        if expected.get(at..end) != Some(&buffer[..n]) { return Err(invalid("archive rewrite failed exact content verification")); }
        at = end;
    }
    if at != expected.len() { return Err(invalid("archive rewrite failed exact size verification")); }
    Ok(())
}

struct Cancellable<'a> { bytes: &'a [u8], cancel: &'a dyn Fn() -> bool, cancelled: bool }
impl Read for Cancellable<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if (self.cancel)() { self.cancelled = true; return Ok(0); }
        let count = buffer.len().min(64 << 10).min(self.bytes.len());
        buffer[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        Ok(count)
    }
}

/// An infallible codec sink with a strict allocation cap. Writers can seek/finish even after an
/// overflow; no partial result is returned. This also avoids ruzstd's panic on sink IO errors.
pub(crate) struct Output { bytes: Vec<u8>, at: u64, maximum: u64, overflow: bool, what: &'static str }
impl Output {
    pub(crate) fn new(maximum: u64, what: &'static str) -> Self { Self { bytes: Vec::new(), at: 0, maximum, overflow: false, what } }
    pub(crate) fn finish(self) -> Result<Vec<u8>, Error> {
        if self.overflow { Err(Error::Limit(self.what)) } else { Ok(self.bytes) }
    }
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self.at.checked_add(bytes.len() as u64).ok_or_else(|| io::Error::other("archive output position overflow"))?;
        if end > self.maximum || end > usize::MAX as u64 { self.overflow = true; }
        let accepted_end = end.min(self.maximum).min(usize::MAX as u64);
        if self.at < accepted_end {
            let start = self.at as usize;
            let end = accepted_end as usize;
            if self.bytes.len() < end { self.bytes.resize(end, 0); }
            self.bytes[start..end].copy_from_slice(&bytes[..end - start]);
        }
        self.at = end;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Output {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(at) => at as i128,
            SeekFrom::Current(by) => self.at as i128 + by as i128,
            SeekFrom::End(by) => self.bytes.len() as i128 + by as i128,
        };
        self.at = u64::try_from(next).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid archive output seek"))?;
        Ok(self.at)
    }
}

fn gzip_header_bytes(bytes: &[u8]) -> Result<&[u8], Error> {
    if bytes.len() < 10 || bytes[..3] != [31, 139, 8] || bytes[3] & 0xe0 != 0 { return Err(invalid("invalid gzip header")); }
    let flags = bytes[3];
    let mut at = 10usize;
    if flags & 4 != 0 {
        let size = bytes.get(at..at + 2).ok_or_else(|| invalid("truncated gzip extra field"))?;
        at += 2 + u16::from_le_bytes([size[0], size[1]]) as usize;
    }
    for flag in [8, 16] {
        if flags & flag != 0 {
            let end = bytes.get(at..).and_then(|s| s.iter().position(|b| *b == 0)).ok_or_else(|| invalid("unterminated gzip header field"))?;
            at += end + 1;
        }
    }
    if flags & 2 != 0 { at += 2; }
    bytes.get(..at).ok_or_else(|| invalid("truncated gzip header"))
}

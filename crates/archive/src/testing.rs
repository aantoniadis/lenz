//! Small in-memory fixtures for dependent crates' tests. Enabled only by the `testing` feature.

use std::io::{Cursor, Write};
use crate::Compression;

pub fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ::zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = ::zip::write::SimpleFileOptions::default().compression_method(::zip::CompressionMethod::Deflated);
    for (path, content) in files {
        writer.start_file(*path, options).unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

pub fn tar(compression: Compression, files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = ::tar::Builder::new(Vec::new());
    for (path, content) in files {
        let mut header = ::tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append_data(&mut header, path, *content).unwrap();
    }
    let raw = builder.into_inner().unwrap();
    crate::codecs::encode(compression, &raw, None, crate::Limits::default(), &|| false).unwrap()
}

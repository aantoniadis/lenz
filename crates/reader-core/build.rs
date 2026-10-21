use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut hash = Sha256::new();
    // Parser dependencies, extraction and decoding all participate in persisted-record compatibility.
    for path in ["../../Cargo.lock", "../content/src/highlight.rs", "../content/src/encoding.rs", "src/document.rs", "src/index.rs", "src/index_cache.rs", "build.rs"] {
        println!("cargo:rerun-if-changed={path}");
        hash.update(std::fs::read(path)?);
    }
    println!("cargo:rustc-env=LENZ_INDEX_EXTRACTOR={:x}", hash.finalize());
    Ok(())
}

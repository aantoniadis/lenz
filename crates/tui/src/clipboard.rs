//! OSC 52 clipboard output. Never interpolate source text into a terminal escape.
use std::io::{self, Write};
fn encoded(text: &str) -> String {base64(text.as_bytes())}
/// Standard padded base64, for terminal escapes (OSC 52, kitty graphics).
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8;64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out=String::with_capacity(bytes.len().div_ceil(3)*4);
    for chunk in bytes.chunks(3) {
        let n=(u32::from(chunk[0])<<16)|(u32::from(*chunk.get(1).unwrap_or(&0))<<8)|u32::from(*chunk.get(2).unwrap_or(&0));
        for (i,shift) in [18,12,6,0].into_iter().enumerate(){out.push(if i>chunk.len(){'='}else{ALPHABET[((n>>shift)&63)as usize]as char});}
    }
    out
}
pub fn copy(text:&str)->io::Result<()> {let mut out=io::stdout().lock();write!(out,"\x1b]52;c;{}\x07",encoded(text))?;out.flush()}
#[cfg(test)] mod tests {#[test]fn clipboard_encodes_controls_as_data(){for(s,want)in[("",""),("f","Zg=="),("fo","Zm8="),("foo","Zm9v"),("\x1b","Gw==")]{assert_eq!(super::encoded(s),want);}}}

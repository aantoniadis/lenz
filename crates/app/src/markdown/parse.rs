//! Two documents parsed under one budget (Lenz's comparisons); one document is parsed by
//! [`lenz_content::markdown::parse`].
use super::*;

pub fn parse_pair(
    sources: [Arc<str>; 2],
    limits: Limits,
    current: &dyn Fn() -> bool,
) -> Result<ParsedPair, MarkdownError> {
    if !current() {
        return Err(MarkdownError::Cancelled);
    }
    if sources.iter().any(|s| s.len() > limits.bytes_per_side) {
        return Err(MarkdownError::Limit("source bytes"));
    }
    let [left, right] = sources;
    let mut remaining = limits.segments_per_pair;
    let left = parse(left, limits, &mut remaining, current)?;
    let right = parse(right, limits, &mut remaining, current)?;
    if !current() {
        return Err(MarkdownError::Cancelled);
    }
    Ok(ParsedPair {
        sides: [left, right],
    })
}

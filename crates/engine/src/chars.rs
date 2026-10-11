// Ported from IntelliJ IDEA Community Edition (commit bc34875):
//   com.intellij.diff.comparison.CharacterUtils, TrimUtil (isPunctuation, isAlpha, isWhiteSpaceCodePoint,
//   isContinuousScript), StringsKmp (equalsIgnoreWhitespaces).
// Copyright 2000-2025 JetBrains s.r.o. and contributors. Licensed under the Apache License, Version 2.0.
// Modified: translated from Kotlin to Rust; `isAlphabetic` (letter categories + Other_Alphabetic) is Rust's
// `char::is_alphabetic` (the Unicode Alphabetic property, the same set); case folding for Ignore Case is ours.

use crate::by_line::is_ws;

/// ASCII punctuation, except `_` (TrimUtil.isPunctuation).
pub fn is_punctuation(c: char) -> bool {
    matches!(c, '!'..='/' | ':'..='@' | '['..='`' | '{'..='~') && c != '_'
}

/// Neither ASCII whitespace nor punctuation: part of a word, or a word on its own (TrimUtil.isAlpha).
pub fn is_alpha(c: char) -> bool {
    !is_ws(c) && !is_punctuation(c)
}

/// Scripts written without spaces between words: each code point is a word of its own.
pub fn is_continuous_script(c: char) -> bool {
    let c32 = c as u32;
    if c32 < 128 || in_ranges(c32, DECIMAL_DIGIT) {
        return false;
    }
    if c32 > 0xFFFF || in_ranges(c32, IDEOGRAPHIC) || !c.is_alphabetic() {
        return true;
    }
    [HIRAGANA, KATAKANA, THAI, JAVANESE].iter().any(|t| in_ranges(c32, t))
}

/// Ignore Case: Unicode simple lowercase per char (`İ` has no single-char full lowercase; its simple one is `i`).
pub fn fold(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

pub fn fold_str(s: &str) -> String {
    s.chars().map(fold).collect()
}

/// Equal once all ASCII whitespace is dropped (StringsKmp.equalsIgnoreWhitespaces).
pub fn equals_ignore_ws(a: &str, b: &str) -> bool {
    a.chars().filter(|&c| !is_ws(c)).eq(b.chars().filter(|&c| !is_ws(c)))
}

fn in_ranges(c: u32, ranges: &[(u32, u32)]) -> bool {
    match ranges.binary_search_by(|&(s, _)| s.cmp(&c)) {
        Ok(_) => true,
        Err(0) => false,
        Err(i) => c <= ranges[i - 1].1,
    }
}

// Ranges extracted by JetBrains from https://www.unicode.org/Public/UCD/latest/ucd/PropList.txt and Scripts.txt (16.0.0).

const IDEOGRAPHIC: &[(u32, u32)] = &[
    (0x3006, 0x3006), (0x3007, 0x3007), (0x3021, 0x3029), (0x3038, 0x303a), (0x3400, 0x4dbf), (0x4e00, 0x9fff),
    (0xf900, 0xfa6d), (0xfa70, 0xfad9), (0x16fe4, 0x16fe4), (0x17000, 0x187f7), (0x18800, 0x18cd5), (0x18cff, 0x18d08),
    (0x1b170, 0x1b2fb), (0x20000, 0x2a6df), (0x2a700, 0x2b739), (0x2b740, 0x2b81d), (0x2b820, 0x2cea1), (0x2ceb0, 0x2ebe0),
    (0x2ebf0, 0x2ee5d), (0x2f800, 0x2fa1d), (0x30000, 0x3134a), (0x31350, 0x323af),
];

const HIRAGANA: &[(u32, u32)] =
    &[(0x3041, 0x3096), (0x309d, 0x309e), (0x309f, 0x309f), (0x1b001, 0x1b11f), (0x1b132, 0x1b132), (0x1b150, 0x1b152), (0x1f200, 0x1f200)];

const KATAKANA: &[(u32, u32)] = &[
    (0x30a1, 0x30fa), (0x30fd, 0x30fe), (0x30ff, 0x30ff), (0x31f0, 0x31ff), (0x32d0, 0x32fe), (0x3300, 0x3357), (0xff66, 0xff6f),
    (0xff71, 0xff9d), (0x1aff0, 0x1aff3), (0x1aff5, 0x1affb), (0x1affd, 0x1affe), (0x1b000, 0x1b000), (0x1b120, 0x1b122),
    (0x1b155, 0x1b155), (0x1b164, 0x1b167),
];

const THAI: &[(u32, u32)] = &[
    (0x0e01, 0x0e30), (0x0e31, 0x0e31), (0x0e32, 0x0e33), (0x0e34, 0x0e3a), (0x0e40, 0x0e45), (0x0e46, 0x0e46), (0x0e47, 0x0e4e),
    (0x0e4f, 0x0e4f), (0x0e50, 0x0e59), (0x0e5a, 0x0e5b),
];

const JAVANESE: &[(u32, u32)] = &[
    (0xa980, 0xa982), (0xa983, 0xa983), (0xa984, 0xa9b2), (0xa9b3, 0xa9b3), (0xa9b4, 0xa9b5), (0xa9b6, 0xa9b9), (0xa9ba, 0xa9bb),
    (0xa9bc, 0xa9bd), (0xa9be, 0xa9c0), (0xa9c1, 0xa9cd), (0xa9d0, 0xa9d9), (0xa9de, 0xa9df),
];

const DECIMAL_DIGIT: &[(u32, u32)] = &[
    (0x0030, 0x0039), (0x0660, 0x0669), (0x06F0, 0x06F9), (0x07C0, 0x07C9), (0x0966, 0x096F), (0x09E6, 0x09EF), (0x0A66, 0x0A6F),
    (0x0AE6, 0x0AEF), (0x0B66, 0x0B6F), (0x0BE6, 0x0BEF), (0x0C66, 0x0C6F), (0x0CE6, 0x0CEF), (0x0D66, 0x0D6F), (0x0DE6, 0x0DEF),
    (0x0E50, 0x0E59), (0x0ED0, 0x0ED9), (0x0F20, 0x0F29), (0x1040, 0x1049), (0x1090, 0x1099), (0x17E0, 0x17E9), (0x1810, 0x1819),
    (0x1946, 0x194F), (0x19D0, 0x19D9), (0x1A80, 0x1A89), (0x1A90, 0x1A99), (0x1B50, 0x1B59), (0x1BB0, 0x1BB9), (0x1C40, 0x1C49),
    (0x1C50, 0x1C59), (0xA620, 0xA629), (0xA8D0, 0xA8D9), (0xA900, 0xA909), (0xA9D0, 0xA9D9), (0xA9F0, 0xA9F9), (0xAA50, 0xAA59),
    (0xABF0, 0xABF9), (0xFF10, 0xFF19), (0x104A0, 0x104A9), (0x10D30, 0x10D39), (0x10D40, 0x10D49), (0x11066, 0x1106F),
    (0x110F0, 0x110F9), (0x11136, 0x1113F), (0x111D0, 0x111D9), (0x112F0, 0x112F9), (0x11450, 0x11459), (0x114D0, 0x114D9),
    (0x11650, 0x11659), (0x116C0, 0x116C9), (0x116D0, 0x116E3), (0x11730, 0x11739), (0x118E0, 0x118E9), (0x11950, 0x11959),
    (0x11BF0, 0x11BF9), (0x11C50, 0x11C59), (0x11D50, 0x11D59), (0x11DA0, 0x11DA9), (0x11F50, 0x11F59), (0x16130, 0x16139),
    (0x16A60, 0x16A69), (0x16AC0, 0x16AC9), (0x16B50, 0x16B59), (0x16D70, 0x16D79), (0x1CCF0, 0x1CCF9), (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149), (0x1E2F0, 0x1E2F9), (0x1E4F0, 0x1E4F9), (0x1E5F1, 0x1E5FA), (0x1E950, 0x1E959), (0x1FBF0, 0x1FBF9),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_scripts_are_split_per_code_point_but_latin_greek_and_digits_are_not() {
        assert!(is_continuous_script('漢'));
        assert!(is_continuous_script('ひ'));
        assert!(is_continuous_script('ก'));
        assert!(is_continuous_script('😂'));
        assert!(is_continuous_script('½')); // not alphabetic, not a decimal digit
        assert!(!is_continuous_script('é'));
        assert!(!is_continuous_script('Ж'));
        assert!(!is_continuous_script('٣'));
        assert!(!is_continuous_script('a'));
    }

    #[test]
    fn punctuation_is_ascii_only_and_excludes_underscore() {
        assert!(is_punctuation('.') && is_punctuation('{') && is_punctuation('~') && is_punctuation('@'));
        assert!(!is_punctuation('_') && !is_punctuation('a') && !is_punctuation('«'));
        assert!(is_alpha('\r') && !is_alpha('\t'));
    }

    #[test]
    fn ignore_whitespace_equality_drops_all_whitespace() {
        assert!(equals_ignore_ws("a b\n", "ab"));
        assert!(!equals_ignore_ws("a b", "ba"));
    }

    #[test]
    fn folding_is_per_char_simple_lowercase() {
        assert_eq!(fold_str("ÀB İ K"), "àb i k");
    }
}

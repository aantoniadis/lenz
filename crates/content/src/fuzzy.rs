//! Fuzzy file-path ranking shared by Lenz's file switcher and the code reader's file finder.
//!
//! A file name starting with the query beats one merely containing it, which beats a substring anywhere in the
//! path, which beats a scattered subsequence; consecutive characters and characters in the file name score more,
//! and shorter paths win ties. Matching is case-insensitive.

/// The score of `path` for `query`, or `None` when `query` isn't a subsequence of `path` at all.
pub fn score(query: &str, path: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    score_lowered(&query.to_lowercase(), &path.to_lowercase(), path.len())
}

/// [`score`] with both strings already lowercased and the original path's byte length, for callers that rank one
/// query against many paths they keep lowercased.
pub fn score_lowered(q: &str, p: &str, path_len: usize) -> Option<i64> {
    if q.is_empty() {
        return Some(0);
    }
    let name_start = p.rfind('/').map_or(0, |i| i + 1);
    let mut s: i64 = 0;
    let mut prev: Option<usize> = None;
    let mut from = 0;
    for qc in q.chars() {
        let at = from + p[from..].find(qc)?;
        s += if at >= name_start { 10 } else { 1 };
        if prev.is_some_and(|pr| pr + 1 == at) {
            s += 6;
        }
        prev = Some(at);
        from = at + qc.len_utf8();
    }
    let substring = if p[name_start..].starts_with(q) {
        30_000
    } else if p[name_start..].contains(q) {
        20_000
    } else if p.contains(q) {
        10_000
    } else {
        0
    };
    Some(substring + s * 100 - path_len as i64)
}

/// Whether `query` matches `text` at word starts only: each character continues the previous match or begins a word
/// (after a separator, a lowercase-to-uppercase step or a digit run), so `nsr` finds `NewSearchResult` and
/// `new_search` but not `inspector`. Case-insensitive; for mixed lists where scattered letters are noise.
pub fn word_starts(query: &str, text: &str) -> bool {
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().collect();
    let start = |j: usize| j == 0 || !t[j - 1].is_alphanumeric() || (t[j].is_uppercase() && t[j - 1].is_lowercase()) || (t[j].is_ascii_digit() && !t[j - 1].is_ascii_digit());
    // ponytail: backtracking, fine for labels and paths; a DP would bound pathological inputs.
    fn go(q: &[char], t: &[char], from: usize, joined: bool, start: &dyn Fn(usize) -> bool) -> bool {
        let Some(&c) = q.first() else { return true };
        (from..t.len()).any(|j| t[j].to_lowercase().eq(std::iter::once(c)) && ((joined && j == from) || start(j)) && go(&q[1..], t, j + 1, true, start))
    }
    go(&q, &t, 0, false, &start)
}

/// Indices of `paths` matching `query`, best first (ties keep the given order).
pub fn rank(query: &str, paths: &[&str]) -> Vec<usize> {
    let mut hits: Vec<(i64, usize)> = paths.iter().enumerate().filter_map(|(i, p)| score(query, p).map(|s| (s, i))).collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_prefix_beats_containment_beats_path_beats_scatter() {
        let paths = ["src/old_main.rs", "main/src/lib.rs", "src/main.rs", "m/a/i/n.rs", "docs/readme.md"];
        let order: Vec<&str> = rank("main", &paths).into_iter().map(|i| paths[i]).collect();
        assert_eq!(order, ["src/main.rs", "src/old_main.rs", "main/src/lib.rs", "m/a/i/n.rs"]);
        assert_eq!(score("zz", "src/main.rs"), None);
        assert_eq!(score("", "anything"), Some(0));
    }

    #[test]
    fn word_starts_take_humps_separators_and_runs_but_not_scattered_letters() {
        for (q, t) in [("nsr", "NewSearchResult"), ("nsr", "new_search_result"), ("main", "src/main.rs"), ("sw", "Soft Wrap"), ("soft wr", "Soft Wrap"), ("v2", "parse_v2"), ("", "x")] {
            assert!(word_starts(q, t), "{q} {t}");
        }
        for (q, t) in [("nsr", "inspector"), ("sftwrp", "Soft Wrap"), ("ae", "Save")] {
            assert!(!word_starts(q, t), "{q} {t}");
        }
    }

    #[test]
    fn case_insensitive_and_lowered_form_agrees() {
        for (q, p) in [("Main", "src/MAIN.rs"), ("ÉT", "Été/été.txt"), ("lib", "src/lib.rs")] {
            assert_eq!(score(q, p), score_lowered(&q.to_lowercase(), &p.to_lowercase(), p.len()), "{q} {p}");
            assert!(score(q, p).is_some());
        }
    }
}

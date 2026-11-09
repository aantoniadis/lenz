//! New Scratch File's kinds, as IntelliJ's list: a few common ones first, then every language Lenz reads, by name.
//! Choosing one creates the next free `scratch_N.<extension>` in the personal scratch folder.
use crate::sql_dialect::Dialect;

/// One kind of scratch file: its label, the extension that gives it its language, and a SQL dialect to set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub label: &'static str,
    pub extension: &'static str,
    pub dialect: Option<Dialect>,
}

const fn kind(label: &'static str, extension: &'static str) -> Kind {
    Kind { label, extension, dialect: None }
}

/// How many kinds lead the list before the rest, by name (a separator follows them in the full list).
pub const PINNED: usize = 4;

/// Every kind, pinned ones first. Each extension makes Lenz read the file as that language.
pub const KINDS: &[Kind] = &[
    kind("Plain text", "txt"),
    kind("JSON", "json"),
    kind("Ruby", "rb"),
    Kind { label: "PostgreSQL", extension: "sql", dialect: Some(Dialect::PostgreSql) },
    kind("Bash", "sh"),
    kind("C", "c"),
    kind("C#", "cs"),
    kind("C++", "cpp"),
    kind("CSS", "css"),
    kind("CSV", "csv"),
    kind("Dockerfile", "dockerfile"),
    Kind { label: "DuckDB", extension: "sql", dialect: Some(Dialect::DuckDb) },
    kind("Environment file", "env"),
    kind("ERB template", "html.erb"),
    kind("Go", "go"),
    kind("HAML template", "haml"),
    kind("HTML", "html"),
    kind("Java", "java"),
    kind("JavaScript", "js"),
    kind("Kotlin", "kt"),
    kind("Lua", "lua"),
    kind("Makefile", "mk"),
    kind("Markdown", "md"),
    kind("PHP", "php"),
    kind("Python", "py"),
    kind("Rust", "rs"),
    kind("TOML", "toml"),
    kind("TSX", "tsx"),
    kind("TypeScript", "ts"),
    kind("XML", "xml"),
    kind("YAML", "yml"),
    kind("Zig", "zig"),
];

/// The kinds `query` picks (indices into [`KINDS`]): all of them in order for an empty query, else those whose label
/// or extension holds its letters in order, best match first (a label starting with it, then containing it).
pub fn matching(query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() { return (0..KINDS.len()).collect(); }
    let mut found: Vec<(i64, usize)> = KINDS.iter().enumerate().filter_map(|(i, k)| {
        let label = lenz_content::fuzzy::score(query, k.label);
        let extension = lenz_content::fuzzy::score(query, k.extension).map(|s| s - 1);
        label.max(extension).map(|score| (score, i))
    }).collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    found.into_iter().map(|(_, i)| i).collect()
}

/// The next free scratch name for `extension`: `scratch_N.<extension>`, N one more than any `scratch_N.*` in `names`.
pub fn next_name<'a>(names: impl IntoIterator<Item = &'a str>, extension: &str) -> String {
    let taken = names.into_iter().filter_map(|name| {
        name.strip_prefix("scratch_")?.split('.').next()?.parse::<u64>().ok()
    }).max().unwrap_or(0);
    format!("scratch_{}.{extension}", taken + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_reads_as_its_language() {
        use lenz_content::highlight::detect;
        for k in KINDS {
            let name = format!("scratch_1.{}", k.extension);
            let language = detect(std::path::Path::new(&name), "");
            match k.extension {
                // Plain text, a CSV table, and templates read through Lenz's template support, not a grammar.
                "txt" | "csv" | "haml" | "html.erb" => assert_eq!(language, None, "{name}"),
                _ => assert!(language.is_some(), "{} ({name}) is not read as a language", k.label),
            }
        }
        assert!(KINDS[PINNED..].windows(2).all(|w| w[0].label.to_lowercase() <= w[1].label.to_lowercase()), "the rest by name");
    }

    #[test]
    fn a_query_picks_kinds_by_label_or_extension() {
        let labels = |q: &str| matching(q).into_iter().map(|i| KINDS[i].label).collect::<Vec<_>>();
        assert_eq!(labels("").len(), KINDS.len());
        assert_eq!(labels("rub").first(), Some(&"Ruby"));
        assert_eq!(labels("rs").first(), Some(&"Rust"), "by extension");
        assert_eq!(labels("pg").first(), Some(&"PostgreSQL"));
        assert!(labels("zzz").is_empty());
    }

    #[test]
    fn names_count_on_from_the_highest_scratch_number() {
        assert_eq!(next_name([], "rb"), "scratch_1.rb");
        assert_eq!(next_name(["scratch_1.rb", "scratch_7.json", "notes.txt", "scratch_x.txt"], "rb"), "scratch_8.rb");
        assert_eq!(next_name(["scratch_2.html.erb"], "html.erb"), "scratch_3.html.erb");
    }
}

//! Which SQL dialect a file is written for: a project default, a per-file override, or nothing chosen yet.
//! Choosing or opening one only records a name. It never opens a database, a DuckDB file or a process.

/// The dialects Lenz accepts. They are distinct: PostgreSQL similarity never stands in for DuckDB.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Dialect {
    PostgreSql,
    DuckDb,
}

impl Dialect {
    pub const ALL: [Self; 2] = [Self::PostgreSql, Self::DuckDb];

    /// The remembered name, as the settings file spells it.
    pub fn id(self) -> &'static str {
        match self {
            Self::PostgreSql => "postgresql",
            Self::DuckDb => "duckdb",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PostgreSql => "PostgreSQL",
            Self::DuckDb => "DuckDB",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.id() == id)
    }
}

/// Where the active dialect comes from, highest precedence first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Chosen for this file (a standalone file's only source).
    File,
    /// The open project's default.
    Project,
    /// Nobody chose one: Lenz does not assume PostgreSQL.
    Unset,
}

/// A file's effective SQL dialect and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub dialect: Option<Dialect>,
    pub origin: Origin,
}

impl Selection {
    pub fn unset() -> Self {
        Self { dialect: None, origin: Origin::Unset }
    }

    /// This file's own choice, then the project default, and no guess from the text.
    pub fn resolve(file: Option<Dialect>, project: Option<Dialect>) -> Self {
        match (file, project) {
            (Some(dialect), _) => Self { dialect: Some(dialect), origin: Origin::File },
            (None, Some(dialect)) => Self { dialect: Some(dialect), origin: Origin::Project },
            (None, None) => Self::unset(),
        }
    }

    /// For dialogs: `DuckDB (this file)`, `PostgreSQL (project default)`, `not set`.
    pub fn label(self) -> String {
        match (self.dialect, self.origin) {
            (Some(d), Origin::File) => format!("{} (this file)", d.label()),
            (Some(d), Origin::Project) => format!("{} (project default)", d.label()),
            _ => "not set".into(),
        }
    }

    /// For a status bar, which has less room: `DuckDB (this file)`, `PostgreSQL (project)`, `not set`.
    pub fn status(self) -> String {
        match (self.dialect, self.origin) {
            (Some(d), Origin::File) => format!("{} (this file)", d.label()),
            (Some(d), Origin::Project) => format!("{} (project)", d.label()),
            _ => "not set".into(),
        }
    }
}

/// What an SQL assistance request (and its result) is about: the chosen dialect, the session's dialect generation
/// and the exact document version. A result may publish only while it is still current.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub dialect: Dialect,
    pub generation: u64,
    pub key: crate::text::SnapshotKey,
}

/// What this build compiles for a dialect offline, and what it deliberately does not do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assistance {
    pub name: &'static str,
    pub offers: &'static str,
    pub limits: &'static str,
}

const ASSISTANCE: &[(Dialect, Assistance)] = &[
    (Dialect::PostgreSql, Assistance {
        name: "PostgreSQL offline assistance",
        offers: "query-local completion, syntax diagnostics, formatting and curated function help",
        limits: "syntax and query-local names only, no database is contacted and no statement is executed",
    }),
    (Dialect::DuckDb, Assistance {
        name: "DuckDB offline assistance",
        offers: "query-local completion; its native parser, formatter and function help need an explicitly chosen executable",
        limits: "native requests use an in-memory engine with external access disabled; no metadata database is opened and no edited statement is executed",
    }),
];

/// The offline assistance compiled for `dialect` (`None`: this build has no adapter for it).
pub fn assistance(dialect: Dialect) -> Option<Assistance> {
    assistance_in(dialect, ASSISTANCE)
}

fn assistance_in(dialect: Dialect, table: &[(Dialect, Assistance)]) -> Option<Assistance> {
    table.iter().find(|(d, _)| *d == dialect).map(|(_, a)| *a)
}

/// One row for a details dialog: what can assist the chosen dialect, or that nothing can. A dialect the build has
/// no adapter for says so instead of pretending assistance exists.
pub fn assistance_note(selection: Selection) -> String {
    note(selection.dialect, selection.dialect.and_then(assistance))
}

fn note(dialect: Option<Dialect>, assistance: Option<Assistance>) -> String {
    match (dialect, assistance) {
        (None, _) => "Assistance: none — no dialect is chosen, so no SQL assistance runs and Lenz does not assume PostgreSQL.".into(),
        (Some(_), Some(a)) => format!("Assistance: {} — {}; {}.", a.name, a.offers, a.limits),
        (Some(d), None) => format!("Assistance: no offline {} assistance in this build; choosing the dialect only records the choice and connects to nothing.", d.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_choice_outranks_the_project_default_and_unset_stays_unset() {
        let (pg, duck) = (Some(Dialect::PostgreSql), Some(Dialect::DuckDb));
        assert_eq!(Selection::resolve(None, None), Selection { dialect: None, origin: Origin::Unset });
        assert_eq!(Selection::resolve(None, None).label(), "not set");
        assert_eq!(Selection::resolve(pg, duck), Selection { dialect: pg, origin: Origin::File });
        assert_eq!(Selection::resolve(pg, duck).label(), "PostgreSQL (this file)");
        assert_eq!(Selection::resolve(None, duck).label(), "DuckDB (project default)");
        assert_eq!(Selection::resolve(None, Some(Dialect::PostgreSql)).label(), "PostgreSQL (project default)");
        assert_eq!(Dialect::ALL.map(Dialect::id), ["postgresql", "duckdb"]);
        assert_eq!(Dialect::from_id("duckdb"), Some(Dialect::DuckDb));
        assert_eq!(Dialect::from_id("sqlite"), None);
    }

    #[test]
    fn a_dialect_without_an_adapter_reports_that_honestly() {
        assert_eq!(note(None, None), "Assistance: none — no dialect is chosen, so no SQL assistance runs and Lenz does not assume PostgreSQL.");
        assert_eq!(assistance_in(Dialect::DuckDb, &[]), None);
        assert_eq!(note(Some(Dialect::DuckDb), None), "Assistance: no offline DuckDB assistance in this build; choosing the dialect only records the choice and connects to nothing.");
        let note = assistance_note(Selection { dialect: Some(Dialect::PostgreSql), origin: Origin::Project });
        assert!(note.starts_with("Assistance: PostgreSQL offline assistance — query-local completion"), "{note}");
        assert!(note.ends_with("no database is contacted and no statement is executed."), "{note}");
    }
}

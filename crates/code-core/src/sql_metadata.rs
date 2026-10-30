//! Explicit catalog snapshots. These fixed operations never accept edited SQL.
use std::{io::Read, collections::BTreeMap, path::Path, time::Duration};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Column { pub name: String, pub data_type: String }
#[derive(Clone, Debug, Deserialize)]
pub struct Relation { pub name: String, pub columns: Vec<Column> }
#[derive(Clone, Debug, Deserialize)]
pub struct Function { pub name: String, pub arguments: String, pub result: String }
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Catalog { pub relations: Vec<Relation>, pub functions: Vec<Function>, #[serde(skip)] pub file: Option<(FileVersion, std::path::PathBuf, Option<FileVersion>)> }
impl Catalog {
    pub fn file_current(&self) -> bool { self.file.as_ref().is_none_or(|(file,wal_path,wal)| FileVersion::read(&file.path).as_ref() == Ok(file) && FileVersion::wal(wal_path).as_ref() == wal.as_ref()) }
    pub fn names(&self) -> BTreeMap<String, Vec<String>> {
        self.relations.iter().map(|r| (r.name.clone(), r.columns.iter().map(|c| c.name.clone()).collect())).collect()
    }
    pub fn documentation(&self, name: &str) -> Option<String> {
        self.relations.iter().find(|r| r.name == name).map(|r| r.columns.iter().map(|c| format!("{}: {}", c.name, c.data_type)).collect::<Vec<_>>().join("\n"))
            .or_else(|| { let columns = self.relations.iter().flat_map(|r| r.columns.iter().filter(move |c| c.name == name).map(move |c| format!("{}.{}: {}",r.name,c.name,c.data_type))).collect::<Vec<_>>(); (!columns.is_empty()).then(|| columns.join("\n")) })
            .or_else(|| self.functions.iter().find(|f| f.name == name || f.name.ends_with(&format!(".{name}"))).map(|f| format!("{}({}) → {}", f.name, f.arguments, f.result)))
    }
}

pub fn valid_service(service: &str) -> bool {
    !service.is_empty() && service.len() <= 128 && service.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

// Only pg_catalog functions and relations; no EXPLAIN, row data, user function calls or source interpolation.
// The extra row makes truncation an explicit failure rather than a silently incomplete snapshot.
pub const POSTGRES_QUERY: &str = r#"BEGIN READ ONLY;
SET LOCAL statement_timeout = '3000ms';
SET LOCAL lock_timeout = '500ms';
SET LOCAL search_path = pg_catalog;
WITH rels AS (
 SELECT c.oid, quote_ident(n.nspname)||'.'||quote_ident(c.relname) AS name
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE c.relkind IN ('r','p','v','m','f') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' AND n.nspname NOT LIKE 'pg_temp%'
 ORDER BY n.nspname,c.relname LIMIT 513
), relations AS (
 SELECT r.name, COALESCE((SELECT json_agg(json_build_object('name',quote_ident(a.attname),'data_type',pg_catalog.format_type(a.atttypid,a.atttypmod)) ORDER BY a.attnum) FROM pg_catalog.pg_attribute a WHERE a.attrelid=r.oid AND a.attnum>0 AND NOT a.attisdropped),'[]'::json) AS columns FROM rels r
), funcs AS (
 SELECT quote_ident(n.nspname)||'.'||quote_ident(p.proname) AS name, pg_catalog.pg_get_function_arguments(p.oid) AS arguments, pg_catalog.pg_get_function_result(p.oid) AS result
 FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_temp%' AND p.prokind IN ('f','w') ORDER BY n.nspname,p.proname,p.oid LIMIT 513
)
SELECT json_build_object('relations',COALESCE((SELECT json_agg(relations) FROM relations),'[]'::json),'functions',COALESCE((SELECT json_agg(funcs) FROM funcs),'[]'::json));
COMMIT;
"#;

pub fn postgres(executable: &Path, service: &str, current: &dyn Fn() -> bool) -> Result<Catalog, String> {
    if !valid_service(service) { return Err("Use a libpq service name (letters, digits, underscore, dot or hyphen), not a connection string".into()); }
    let env = vec![("PGSERVICE".into(),service.into()),("PGCONNECT_TIMEOUT".into(),"3".into()),("PGAPPNAME".into(),"lenz-metadata".into()),("PGOPTIONS".into(),"-c default_transaction_read_only=on".into()),("PGCLIENTENCODING".into(),"UTF8".into())];
    let output = crate::sql_process::run(executable, &["-X","-w","-q","-A","-t","-v","ON_ERROR_STOP=1"], POSTGRES_QUERY, &env, Duration::from_secs(5), 1048576, current)?;
    let catalog: Catalog = serde_json::from_str(output.trim()).map_err(|_| "PostgreSQL returned an invalid metadata snapshot".to_owned())?;
    if catalog.relations.len() > 512 || catalog.functions.len() > 512 { return Err("Catalog exceeds 512 relations or function overloads; choose a smaller database".into()); }
    Ok(catalog)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileVersion { path: std::path::PathBuf, bytes: u64, modified: std::time::SystemTime, #[cfg(unix)] device: u64, #[cfg(unix)] inode: u64 }
impl FileVersion {
    // SQLite may create an empty WAL during a read-only attach; it contains no database changes.
    fn wal(path: &Path) -> Option<Self> { Self::read(path).ok().filter(|version| version.bytes != 0) }
    fn read(path: &Path) -> Result<Self, String> {
        let m = std::fs::metadata(path).map_err(|_| "Cannot read the selected metadata file".to_owned())?;
        #[cfg(unix)] use std::os::unix::fs::MetadataExt;
        Ok(Self { path:path.to_owned(),bytes:m.len(),modified:m.modified().map_err(|_| "Cannot identify the selected metadata file".to_owned())?, #[cfg(unix)] device:m.dev(), #[cfg(unix)] inode:m.ino() })
    }
}

// This query reads definitions, including macros, without binding or calling them.
pub const DUCKDB_QUERY: &str = r#"WITH rels AS (
 SELECT schema_name,table_name,list(struct_pack(name:=column_name,data_type:=data_type) ORDER BY column_index) AS columns
 FROM duckdb_columns() WHERE database_name='lenz_catalog' GROUP BY schema_name,table_name ORDER BY schema_name,table_name LIMIT 513
), funcs AS (
 SELECT '"'||replace(schema_name,'"','""')||'"."'||replace(function_name,'"','""')||'"' AS name,
 array_to_string(list_transform(parameters,lambda p: '"'||replace(p,'"','""')||'"'),', ') AS arguments, coalesce(return_type,'macro result (not evaluated)') AS result
 FROM duckdb_functions() WHERE database_name='lenz_catalog' ORDER BY schema_name,function_name LIMIT 513
)
SELECT to_json(struct_pack(
 relations:=(SELECT coalesce(list(struct_pack(name:='"'||replace(schema_name,'"','""')||'"."'||replace(table_name,'"','""')||'"',columns:=columns)),[]) FROM rels),
 functions:=(SELECT coalesce(list(struct_pack(name:=name,arguments:=coalesce(arguments,''),result:=result)),[]) FROM funcs)
)) AS catalog;"#;

pub fn duckdb(executable: &Path, file: &Path, current: &dyn Fn() -> bool) -> Result<Catalog, String> {
    if !file.is_absolute() { return Err("Choose an absolute local DuckDB or SQLite file path".into()); }
    let original = file.to_owned();
    let file = file.canonicalize().map_err(|_| "Selected database file does not exist".to_owned())?;
    if !file.is_file() { return Err("Select a DuckDB or SQLite file, not a folder".into()); }
    let path = file.to_str().filter(|s| !s.chars().any(|c| c.is_control())).ok_or("Metadata file path must be UTF-8 without control characters")?;
    let mut header = [0; 16];
    let sqlite = std::fs::File::open(&file).and_then(|mut f| f.read_exact(&mut header)).is_ok() && &header == b"SQLite format 3\0";
    let before = FileVersion::read(&original)?;
    let suffix = if sqlite { "-wal" } else { ".wal" };
    let wal = std::path::PathBuf::from(format!("{path}{suffix}"));
    let wal_before = FileVersion::wal(&wal);
    let literal = path.replace('\'',"''");
    // Load only the installed SQLite extension before restricting access to the selected database.
    let load = if sqlite { "LOAD sqlite;" } else { "" };
    let kind = if sqlite { "TYPE SQLITE, " } else { "" };
    let sidecars = if sqlite { format!(",'{literal}-shm'") } else { String::new() };
    let input = format!("SET threads=1; SET memory_limit='128MB'; SET autoinstall_known_extensions=false; SET autoload_known_extensions=false; {load} SET allowed_paths=['{literal}','{literal}{suffix}'{sidecars}]; SET enable_external_access=false; ATTACH '{literal}' AS lenz_catalog ({kind}READ_ONLY); {DUCKDB_QUERY}\n");
    let output = crate::sql_process::run(executable, &["-no-agent","-init","/dev/null","-bail","-json",":memory:"], &input, &[], Duration::from_secs(5), crate::sql_process::Capture { limit: 1048576, diagnostics: true }, current)
        .map_err(|why| if sqlite { format!("{why}\nSQLite metadata needs DuckDB's sqlite extension. If it is missing, run INSTALL sqlite in the selected DuckDB executable, then reconnect.") } else { why })?;
    let mut rows: Vec<serde_json::Value> = serde_json::from_str(&output).map_err(|_| "Invalid DuckDB metadata response".to_owned())?;
    let value = rows.pop().and_then(|mut r| r.get_mut("catalog").map(serde_json::Value::take)).ok_or("Missing DuckDB catalog response")?;
    let mut catalog: Catalog = serde_json::from_value(value).map_err(|_| "Invalid DuckDB catalog definitions".to_owned())?;
    if catalog.relations.len()>512 || catalog.functions.len()>512 { return Err("Catalog exceeds 512 relations or function overloads".into()); }
    if FileVersion::read(&original)? != before || FileVersion::wal(&wal) != wal_before { return Err("Metadata file changed during the read; Refresh explicitly".into()); }
    for r in &mut catalog.relations { for c in &mut r.columns { c.name = duck_identifier(&c.name); } }
    catalog.file = Some((before,wal,wal_before));
    Ok(catalog)
}

fn duck_identifier(name: &str) -> String {
    let simple = name.as_bytes().first().is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_') && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if simple && sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::DuckDbDialect {}, &format!("SELECT 1 AS {name}")).is_ok() { name.to_owned() } else { format!("\"{}\"",name.replace('"',"\"\"")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires LENZ_TEST_DUCKDB with the installed sqlite extension"]
    fn native_sqlite_metadata_uses_duckdb_read_only_and_tracks_wal() {
        let executable = std::path::PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
        for journal in ["DELETE", "WAL"] {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("quoted'database.db");
            let literal = file.to_str().unwrap().replace('\'', "''");
            let setup = format!("LOAD sqlite; ATTACH '{literal}' AS fixture (TYPE SQLITE, JOURNAL_MODE '{journal}'); CREATE TABLE fixture.example(id INTEGER, \"odd name\" TEXT); INSERT INTO fixture.example VALUES (42, 'unchanged');");
            crate::sql_process::run(&executable, &["-no-agent", "-init", "/dev/null", "-bail", ":memory:"], &setup, &[], Duration::from_secs(5), 1048576, &|| true).unwrap();
            let before = std::fs::read(&file).unwrap();
            let catalog = duckdb(&executable, &file, &|| true).unwrap();
            assert_eq!(catalog.relations[0].name, "\"main\".\"example\"");
            assert_eq!(catalog.relations[0].columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "\"odd name\""]);
            assert_eq!(std::fs::read(&file).unwrap(), before);
            assert!(catalog.file_current());
            if journal == "WAL" { assert_eq!(std::fs::metadata(format!("{}-wal", file.display())).unwrap().len(), 0); }
            std::fs::write(format!("{}-wal", file.display()), b"changed").unwrap();
            assert!(!catalog.file_current());
            let bad = dir.path().join("bad.db");
            std::fs::write(&bad, b"not a database").unwrap();
            let error = duckdb(&executable, &bad, &|| true).unwrap_err();
            assert!(error.contains("SQL tool exited") && error.contains("Error"), "{error}");
        }
    }
    #[test]
    #[ignore = "requires LENZ_TEST_DUCKDB and a checkpointed LENZ_TEST_SQLITE_BACKUP"]
    fn native_sqlite_backup_metadata_and_completion() {
        let executable = std::path::PathBuf::from(std::env::var_os("LENZ_TEST_DUCKDB").unwrap());
        let source = std::path::PathBuf::from(std::env::var_os("LENZ_TEST_SQLITE_BACKUP").unwrap());
        let original = std::fs::read(&source).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("backup.db");
        std::fs::write(&copy, &original).unwrap();
        let catalog = duckdb(&executable, &copy, &|| true).unwrap();
        assert!(catalog.file_current());
        let relation = catalog.relations.iter().find(|r| !r.columns.is_empty()).unwrap();
        let query = format!("SELECT t. FROM {} t", relation.name);
        let completion = crate::duckdb_assistance::complete_with_catalog(&query, "SELECT t.".len(), &catalog.names()).unwrap();
        assert!(completion.contains(&relation.columns[0].name));
        assert_eq!(std::fs::read(&copy).unwrap(), original);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        eprintln!("Loaded {} relations; completion matched; source and copy database bytes unchanged", catalog.relations.len());
    }
    #[test]
    fn empty_wal_creation_is_not_a_catalog_change_but_nonempty_wal_changes_are() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.db");
        let wal = dir.path().join("test.db-wal");
        std::fs::write(&file, b"unchanged database").unwrap();
        let before = FileVersion::read(&file).unwrap();
        let clean = Catalog { file: Some((before.clone(), wal.clone(), None)), ..Default::default() };
        std::fs::write(&wal, b"").unwrap();
        assert!(clean.file_current(), "a read-only SQLite connection may create an empty WAL");
        std::fs::write(&wal, b"committed changes").unwrap();
        assert!(!clean.file_current(), "a new WAL with content must invalidate the snapshot");
        let changed = Catalog { file: Some((before, wal.clone(), Some(FileVersion::read(&wal).unwrap()))), ..Default::default() };
        assert!(changed.file_current());
        std::fs::write(&wal, b"").unwrap();
        assert!(!changed.file_current(), "truncating a populated WAL must invalidate the snapshot");
        assert!(clean.file_current());
        std::fs::remove_file(wal).unwrap();
        assert!(clean.file_current(), "removing an empty WAL does not change the catalog");
    }
    #[test]
    fn catalog_file_identity_rejects_replaced_targets_and_wal_changes() {
        let dir=tempfile::tempdir().unwrap();let file=dir.path().join("test.duckdb");let wal=dir.path().join("test.duckdb.wal");
        std::fs::write(&file,b"original").unwrap();
        let catalog=Catalog{file:Some((FileVersion::read(&file).unwrap(),wal.clone(),None)),..Default::default()};
        assert!(catalog.file_current());
        std::fs::write(&wal,b"new WAL").unwrap();assert!(!catalog.file_current());
        std::fs::remove_file(&wal).unwrap();assert!(catalog.file_current());
        let new=dir.path().join("new");std::fs::write(&new,b"new contents").unwrap();std::fs::rename(new,&file).unwrap();assert!(!catalog.file_current());
    }
}

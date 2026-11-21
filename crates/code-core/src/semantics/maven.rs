//! Maven profiles for JDT LS's import of a project. JDT LS has no profile setting; a Maven settings file holding only
//! `<activeProfiles>`, given as `java.configuration.maven.globalSettings`, activates them, and Maven adds it to the
//! user's own `~/.m2/settings.xml` (`userSettings` would replace that file: its mirrors and credentials). Verified
//! against JDT LS 1.63.0: the import uses them at start, including when it reuses its workspace-data folder.
use std::path::{Path, PathBuf};

/// The largest pom.xml read for its profiles.
const POM_LIMIT: u64 = 1 << 20;

/// Parsed files by path, kept while their modification time and size stay: menus ask on every frame they are shown.
pub struct Cache<T>(std::cell::RefCell<std::collections::HashMap<PathBuf, (Option<(std::time::SystemTime, u64)>, T)>>);

impl<T> Default for Cache<T> { fn default() -> Self { Self(Default::default()) } }

impl<T: Clone> Cache<T> {
    /// `read(path)`, or its last result while the file is unchanged.
    pub fn get(&self, path: &Path, read: fn(&Path) -> T) -> T {
        let stamp = std::fs::metadata(path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
        let mut cache = self.0.borrow_mut();
        if let Some((seen, ids)) = cache.get(path) && *seen == stamp && stamp.is_some() { return ids.clone(); }
        let ids = read(path);
        if cache.len() > 64 { cache.clear(); }
        cache.insert(path.to_owned(), (stamp, ids.clone()));
        ids
    }
}

/// The profile ids `pom` declares, in order, without duplicates; none when it cannot be read.
pub fn pom_profiles(pom: &Path) -> Vec<String> {
    let Ok(file) = std::fs::File::open(pom) else { return Vec::new() };
    let mut text = String::new();
    if std::io::Read::read_to_string(&mut std::io::Read::take(file, POM_LIMIT), &mut text).is_err() { return Vec::new(); }
    profiles_in(&text)
}

/// [`pom_profiles`] of pom.xml text: each `<profile>`'s own `<id>` inside `<profiles>`, comments skipped.
pub(crate) fn profiles_in(text: &str) -> Vec<String> {
    let text = without_comments(text);
    let mut ids = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("<profiles>") {
        rest = &rest[at + "<profiles>".len()..];
        let end = rest.find("</profiles>").unwrap_or(rest.len());
        let (section, after) = rest.split_at(end);
        for profile in section.split("<profile>").skip(1) {
            let profile = profile.split("</profile>").next().unwrap_or_default();
            // The profile's own id: the sections that hold ids of their own (executions, repositories) go first.
            let profile = without_sections(profile, &["build", "repositories", "pluginRepositories", "distributionManagement", "reporting"]);
            if let Some(id) = profile.split_once("<id>").and_then(|(_, r)| r.split_once("</id>")).map(|(id, _)| id.trim())
                && !id.is_empty() && !ids.iter().any(|i| i == id)
            {
                ids.push(id.to_owned());
            }
        }
        rest = after;
    }
    ids
}

/// Each profile of `pom` and the source folders it adds (`build-helper-maven-plugin`'s `<source>`, or its own
/// `<sourceDirectory>`/`<testSourceDirectory>`), relative to the project as written, `${project.basedir}/` dropped.
pub fn pom_profile_sources(pom: &Path) -> Vec<(String, Vec<String>)> {
    let Ok(file) = std::fs::File::open(pom) else { return Vec::new() };
    let mut text = String::new();
    if std::io::Read::read_to_string(&mut std::io::Read::take(file, POM_LIMIT), &mut text).is_err() { return Vec::new(); }
    profile_sources_in(&text)
}

pub(crate) fn profile_sources_in(text: &str) -> Vec<(String, Vec<String>)> {
    let text = without_comments(text);
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("<profiles>") {
        rest = &rest[at + "<profiles>".len()..];
        let end = rest.find("</profiles>").unwrap_or(rest.len());
        let (section, after) = rest.split_at(end);
        for profile in section.split("<profile>").skip(1) {
            let profile = profile.split("</profile>").next().unwrap_or_default();
            let own = without_sections(profile, &["build", "repositories", "pluginRepositories", "distributionManagement", "reporting"]);
            let Some(id) = own.split_once("<id>").and_then(|(_, r)| r.split_once("</id>")).map(|(id, _)| id.trim().to_owned()).filter(|id| !id.is_empty()) else { continue };
            let mut folders = Vec::new();
            for tag in ["source", "sourceDirectory", "testSourceDirectory"] {
                let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
                for part in profile.split(open.as_str()).skip(1) {
                    if let Some((folder, _)) = part.split_once(close.as_str()) {
                        let folder = folder.trim().trim_start_matches("${project.basedir}/").trim_start_matches("${basedir}/").trim_end_matches('/');
                        if !folder.is_empty() && !folder.contains("${") && !folders.iter().any(|f| f == folder) { folders.push(folder.to_owned()); }
                    }
                }
            }
            if let Some(known) = out.iter_mut().find(|(i, _)| *i == id) { known.1.extend(folders); } else { out.push((id, folders)); }
        }
        rest = after;
    }
    out
}

/// The profile among `sources` that adds a folder holding `file` (`root`'s project), with that folder.
pub fn profile_for(sources: &[(String, Vec<String>)], root: &Path, file: &Path) -> Option<(String, String)> {
    sources.iter().find_map(|(id, folders)| folders.iter().find(|f| file.starts_with(root.join(f))).map(|f| (id.clone(), f.clone())))
}

fn without_sections(text: &str, tags: &[&str]) -> String {
    let mut text = text.to_owned();
    for tag in tags {
        let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
        while let Some(at) = text.find(&open) {
            let end = text[at..].find(&close).map_or(text.len(), |e| at + e + close.len());
            text.replace_range(at..end, "");
        }
    }
    text
}

fn without_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        rest = rest[at..].find("-->").map_or("", |end| &rest[at + end + 3..]);
    }
    out.push_str(rest);
    out
}

/// A Maven settings file activating `profiles`, ids escaped.
pub fn settings_xml(profiles: &[String]) -> String {
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- Written by Lenz Code: Maven profiles for Eclipse JDT LS's import (Maven Profiles…). -->\n<settings>\n  <activeProfiles>\n");
    for p in profiles { xml += &format!("    <activeProfile>{}</activeProfile>\n", escape(p)); }
    xml += "  </activeProfiles>\n</settings>\n";
    xml
}

/// The profiles a settings file Lenz wrote activates.
pub fn active_in(xml: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(xml) else { return Vec::new() };
    let unescape = |s: &str| s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
    text.split("<activeProfile>").skip(1).filter_map(|p| p.split_once("</activeProfile>")).map(|(id, _)| unescape(id.trim())).filter(|id| !id.is_empty()).collect()
}

/// Where Lenz keeps `root`'s profile settings: beside the provider settings, so they outlive caches.
pub fn settings_path(providers: &Path, root: &Path) -> PathBuf {
    let name = root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    providers.parent().unwrap_or(Path::new(".")).join("maven").join(format!("{name}-{:016x}.xml", super::provider::fnv_path(root)))
}

/// Profile ids typed as a list ("a, b c"): split on commas and spaces, duplicates dropped.
pub fn parse_list(input: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in input.split([',', ' ', '\t']).map(str::trim).filter(|s| !s.is_empty()) {
        if !ids.iter().any(|i| i == id) { ids.push(id.to_owned()); }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_ids_come_from_profiles_only_and_skip_comments_and_nested_ids() {
        let pom = r#"<project><id>not-a-profile</id>
  <!-- <profiles><profile><id>commented</id></profile></profiles> -->
  <profiles>
    <profile><id>benchmarks</id><build><plugins><plugin><executions><execution><id>add-jmh-sources</id></execution></executions></plugin></plugins></build></profile>
    <profile>
      <repositories><repository><id>snapshots</id></repository></repositories>
      <id> release </id>
    </profile>
    <profile><id>benchmarks</id></profile>
  </profiles>
</project>"#;
        assert_eq!(profiles_in(pom), ["benchmarks", "release"]);
        assert!(profiles_in("<project/>").is_empty());
    }

    #[test]
    fn a_profile_that_adds_a_files_folder_is_found() {
        let pom = r#"<project><profiles>
  <profile><id>security</id></profile>
  <profile><id>benchmarks</id><build><plugins><plugin><artifactId>build-helper-maven-plugin</artifactId>
    <executions><execution><id>add-jmh-sources</id><goals><goal>add-test-source</goal></goals>
      <configuration><sources><source>${project.basedir}/src/jmh/java/</source></sources></configuration></execution></executions>
  </plugin></plugins></build></profile>
</profiles></project>"#;
        let sources = profile_sources_in(pom);
        assert_eq!(sources, [("security".to_owned(), vec![]), ("benchmarks".to_owned(), vec!["src/jmh/java".to_owned()])]);
        let root = Path::new("/p/service");
        assert_eq!(profile_for(&sources, root, &root.join("src/jmh/java/x/Bench.java")), Some(("benchmarks".into(), "src/jmh/java".into())));
        assert_eq!(profile_for(&sources, root, &root.join("src/main/java/x/App.java")), None);
    }

    #[test]
    fn settings_round_trip_escaped_ids_and_typed_lists_split_on_commas_and_spaces() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("s.xml");
        let ids = vec!["benchmarks".to_owned(), "a&b<c>".to_owned()];
        let xml = settings_xml(&ids);
        assert!(xml.contains("<activeProfile>a&amp;b&lt;c&gt;</activeProfile>"), "{xml}");
        std::fs::write(&file, xml).unwrap();
        assert_eq!(active_in(&file), ids);
        assert_eq!(parse_list(" benchmarks, release  benchmarks,,"), ["benchmarks", "release"]);
    }

    #[test]
    fn profiles_point_the_project_at_lenzs_file_and_never_replace_the_users_own() {
        use super::super::provider::{self, Settings};
        let dir = tempfile::tempdir().unwrap();
        let (file, root) = (dir.path().join("providers.json"), dir.path().join("proj"));
        std::fs::write(&file, r#"{"providers":{"jdtls":{"args":["--jvm-arg=-Xmx2G"]}},"trusted":[]}"#).unwrap();
        let xml = settings_path(&file, &root);
        provider::set_maven_profiles(&file, &root, &xml, &["benchmarks".into()]).unwrap();
        let settings = Settings::read(&file);
        assert_eq!(provider::maven_settings_owner(&settings, &root, &xml), Some(true));
        assert_eq!(active_in(&xml), ["benchmarks"]);
        assert_eq!(settings.providers["jdtls"].args.as_deref(), Some(&["--jvm-arg=-Xmx2G".to_owned()][..]), "other settings stay");
        // None: the override and the file go, and nothing empty is left behind.
        provider::set_maven_profiles(&file, &root, &xml, &[]).unwrap();
        assert!(!xml.exists());
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert!(value["providers"]["jdtls"].get("projects").is_none(), "{value}");
        // The user's own Maven settings file for the project stays theirs.
        let own = format!(r#"{{"providers":{{"jdtls":{{"projects":{{"{}":{{"settings":{{"java":{{"configuration":{{"maven":{{"globalSettings":"/etc/mine.xml"}}}}}}}}}}}}}}}}}}"#, root.display());
        std::fs::write(&file, &own).unwrap();
        assert_eq!(provider::maven_settings_owner(&Settings::read(&file), &root, &xml), Some(false));
        assert_eq!(provider::set_maven_profiles(&file, &root, &xml, &["benchmarks".into()]), Err(provider::OWN_MAVEN_SETTINGS.into()));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), own, "unchanged");
    }
}

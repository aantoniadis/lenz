//! Haml authoring shared by the GUI and terminal. Markup suggestions never come from an HTML server.
use super::*;
use crate::text::{EditKind, TextEdit};

impl Session {
    pub(super) fn haml_markup_available(&self) -> bool {
        let Some(snapshot) = self.state.current_source().filter(|d| crate::template::Kind::of(d.source()) == Some(crate::template::Kind::Haml) && !d.is_large()) else { return false };
        let Some(caret) = self.state.selection().filter(|s| s.anchor == s.head).map(|s| s.head) else { return false };
        let Some(line) = snapshot.line_range(snapshot.line_of(caret)).filter(|r| r.len() <= 16384) else { return false };
        snapshot.range(line.clone()).is_some_and(|text| crate::template::haml_markup(&text, caret.saturating_sub(line.start)).is_some())
    }

    pub(super) fn haml_completion(&mut self, trigger: CompletionTrigger) -> Option<Effect> {
        let tab = self.state.active?;
        let snapshot = self.state.view(tab)?.snapshot().clone();
        if crate::template::Kind::of(snapshot.source()) != Some(crate::template::Kind::Haml) || snapshot.is_large() { return None; }
        let caret = self.state.selection().filter(|s| s.anchor == s.head)?.head;
        let line = snapshot.line_range(snapshot.line_of(caret))?;
        // Completion inspects a bounded line window, never materializes the draft on input.
        if line.len() > 16384 { return None; }
        let text = snapshot.range(line.clone())?;
        let at = caret.checked_sub(line.start)?;
        let (word, end, names, attribute) = crate::template::haml_markup(&text, at)?;
        if self.text_action_disabled(CommandId::Completion).is_some() { return Some(Effect::default()); }
        self.cancel_completion();
        let items = names.iter().map(|&name| {
            let value = if attribute { format!("{name}=\"\"") } else { name.to_owned() };
            Suggestion::local(name.into(), Some(if attribute { "Haml attribute" } else { "Haml tag" }.into()), &value, line.start + word..line.start + end, name, false)
        }).collect();
        self.state.completion = Suggestions::local(tab, trigger, &snapshot, (line.start + word, caret), items);
        if self.state.completion.is_none() && trigger == CompletionTrigger::Manual { self.state.messages.push("Code completion: no Haml markup names match".into()); }
        Some(Effect { repaint: true, ..Effect::default() })
    }

    pub(super) fn haml_newline(&mut self, newline: &str) -> Option<Effect> {
        let (tab, expected, ranges) = self.selection_ranges()?;
        let snapshot = self.state.view(tab)?.snapshot().clone();
        if crate::template::Kind::of(snapshot.source()) != Some(crate::template::Kind::Haml) { return None; }
        let unit = self.indent_unit();
        let mut edits = Vec::with_capacity(ranges.len());
        for range in ranges {
            let start = snapshot.line_range(snapshot.line_of(range.start))?.start;
            if range.start.saturating_sub(start) > 16384 { return None; }
            let before = snapshot.range(start..range.start)?;
            let indent = crate::template::haml_indent(&before, unit.width);
            edits.push(TextEdit { range, text: format!("{newline}{indent}").into() });
        }
        Some(self.edit(tab, expected, edits, EditKind::Replace))
    }
}

/// Explicit formatting with the installed haml-lint and the project's .haml-lint.yml. The unsaved
/// template travels only on stdin. Haml is compiled for syntax validation, never rendered or evaluated.
/// Safe autocorrections are one draft transaction; haml-lint never writes the source file.
pub(super) fn format(doc: &Document, organize: bool) -> format_work::ToolResult {
    use format_work::ToolResult;
    if organize { return ToolResult::Unavailable("Haml templates have no imports to organize".into()); }
    let cwd = doc.source().parent().unwrap_or(Path::new("/"));
    if !cwd.ancestors().any(|d| d.join(".haml-lint.yml").is_file()) {
        return ToolResult::Unavailable("no .haml-lint.yml configures this template; install haml-lint and configure it for the project to format Haml".into());
    }
    let Some(ruby) = semantics::provider::find_executable(Path::new("ruby"), std::env::var_os("PATH").as_deref()) else {
        return ToolResult::Unavailable("Ruby with haml and haml_lint is required for Haml formatting; Lenz never installs gems".into());
    };
    // Haml's parser checks structure without running the generated Ruby. Exit 65 means haml-lint
    // made safe corrections (or found uncorrectable style offenses); syntax errors exit 1 here.
    let script = "require 'haml_lint'; require 'haml_lint/cli'; require 'stringio'; text = STDIN.read; Haml::Engine.new.call(text); $stdin = StringIO.new(text); $stdout = StringIO.new; status = HamlLint::CLI.new.run(ARGV); out = $stdout.string; abort('haml-lint returned no source (the file may be excluded)') if !text.empty? && out.empty?; Haml::Engine.new.call(out); STDOUT.write(out); exit status";
    let mut args = vec!["-E".into(), "UTF-8:UTF-8".into(), "-e".into(), script.into(), "--".into(), "--stdin".into(), doc.source().display().to_string(),
        "--stderr".into(), "--auto-correct-only".into(), "--no-parallel".into()];
    let bundled = cwd.ancestors().find_map(|d| std::fs::read_to_string(d.join("Gemfile.lock")).ok())
        .is_some_and(|lock| lock.contains("\n    haml_lint ("));
    let executable = if bundled {
        let Some(bundle) = semantics::provider::find_executable(Path::new("bundle"), std::env::var_os("PATH").as_deref()) else {
            return ToolResult::Failed("the project locks haml_lint but bundle is unavailable; no other formatter was run".into());
        };
        args.splice(0..0, ["exec".into(), ruby.display().to_string()]);
        bundle
    } else { ruby };
    match semantics::runtime::run_accepting(&executable, &args, doc.text(), cwd, std::time::Duration::from_secs(20), &[0, 65]) {
        Ok(text) => {
            // Autocorrection may normalize LF internally; each original line ending is preserved when
            // the line count stays equal. Otherwise use the document's first ending for new lines.
            let text = preserve_endings(doc.text(), &text);
            let edit = (text != doc.text().as_ref()).then(|| TextEdit { range: 0..doc.text().len(), text: text.into() });
            ToolResult::Formatted { tool: "haml-lint", edit }
        }
        Err(why) => ToolResult::Failed(format!("haml-lint: {why}")),
    }
}

fn preserve_endings(old: &str, new: &str) -> String {
    let endings: Vec<_> = old.split_inclusive('\n').map(|l| if l.ends_with("\r\n") { "\r\n" } else if l.ends_with('\n') { "\n" } else { "" }).collect();
    let count = new.split_inclusive('\n').count();
    let fallback = endings.iter().copied().find(|e| !e.is_empty()).unwrap_or("\n");
    let mut out = String::with_capacity(new.len());
    for (i, line) in new.split_inclusive('\n').enumerate() {
        out.push_str(line.trim_end_matches(['\r', '\n']));
        let ending = if count == endings.len() { endings.get(i).copied().unwrap_or(fallback) } else if line.ends_with('\n') { fallback } else { "" };
        out.push_str(ending);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_line_endings_and_missing_final_newline_survive_haml_formatting() {
        assert_eq!(preserve_endings("%p a\r\n%p b\n%p c", "%p A\n%p B\n%p C\n"), "%p A\r\n%p B\n%p C");
    }
    #[test]
    #[ignore = "requires Ruby with haml 7.5.1, haml_lint 0.78.0 and rubocop; run alone"]
    fn real_haml_formatter_never_executes_the_draft_and_refuses_missing_output() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("view.html.haml");
        let text = "= File.write('sentinel', 'executed')\r\n%p= 1+2";
        std::fs::write(&file, text).unwrap();
        let config = dir.path().join(".haml-lint.yml");
        std::fs::write(&config, "linters:\n  FinalNewline:\n    enabled: false\n").unwrap();
        let doc = Document::decode(file.clone(), 1, text.as_bytes());
        let format_work::ToolResult::Formatted { edit: Some(edit), .. } = format(&doc, false) else { panic!("haml-lint must format the fixture") };
        assert!(edit.text.contains("1 + 2"));
        assert!(!edit.text.ends_with('\n'));
        assert!(!dir.path().join("sentinel").exists(), "template code never executes");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        std::fs::write(&config, "exclude:\n  - '**/*.haml'\n").unwrap();
        let excluded = format(&doc, false);
        println!("excluded result: {excluded:?}");
        assert!(!matches!(&excluded, format_work::ToolResult::Formatted { edit: Some(e), .. } if e.text.is_empty()), "an excluded file must not become an empty replacement");
        let bad = Document::decode(file, 2, b"%p{invalid\n");
        assert!(matches!(format(&bad, false), format_work::ToolResult::Failed(_)));
    }
}

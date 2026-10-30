use super::*;
use lenz_content::highlight::Capture;

fn edit(s: &mut Session, edits: Vec<(std::ops::Range<usize>, &str)>) {
    let tab = s.state().active().unwrap().id();
    let expected = s.state().current_source().unwrap().key();
    s.dispatch(Command::Edit { tab, expected, edits: edits.into_iter().map(|(range, text)| crate::text::TextEdit { range, text: text.into() }).collect(), kind: crate::text::EditKind::Replace });
}

#[test]
fn pending_syntax_keeps_only_mapped_lexical_colours_through_edits_and_stale_replies() {
    let d = tempfile::tempdir().unwrap();
    let saved = "fn first() {}\r\nlet name = \"λ\";\r\n// tail\r\n";
    let path = write(d.path(), "colours.rs", saved.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "colours.rs");
    settle(&mut s, |s| s.state().highlights().is_some());
    assert_eq!(active_view(&s).style_at(0, 0), (Some(Capture::Keyword), 0));
    assert_eq!(active_view(&s).style_at(1, 12), (Some(Capture::String), 0));

    // Both split views use the same current draft, but each owns its presentation state.
    s.dispatch(Command::Split(Split::Columns));
    let views: Vec<_> = s.state().tabs().iter().map(|tab| tab.id()).collect();
    edit(&mut s, vec![(0..0, "// new λ\r\n"), (19..23, "other")]);
    for tab in &views {
        let view = s.state().view(*tab).unwrap();
        assert!(view.highlights().is_none(), "the parser-facing API stays current-only");
        assert_eq!(view.style_at(0, 0), (None, 0), "inserted text must not inherit an unrelated capture");
        assert_eq!(view.style_at(1, 0), (Some(Capture::Keyword), 0), "unchanged lines keep colour while parsing");
        assert_eq!(view.style_at(2, 4), (None, 0), "replacement text has no borrowed capture");
        assert_eq!(view.style_at(2, 13), (Some(Capture::String), 0));
        assert_eq!(view.style_at(3, 0), (Some(Capture::Comment), 0));
        assert!(view.tokens().is_none() && view.inlay_hints().is_none());
    }
    let rx = s.completions();
    let stale = loop {
        let completion = rx.recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Highlighted { .. })) { break completion; }
        s.dispatch(completion);
    };
    // A second edit removes the inserted line and changes the length before the string.
    edit(&mut s, vec![(0..11, ""), (30..35, "x")]);
    assert_eq!(active_view(&s).style_at(0, 0), (Some(Capture::Keyword), 0));
    assert_eq!(active_view(&s).style_at(1, 9), (Some(Capture::String), 0));
    s.dispatch(stale);
    assert!(active_view(&s).highlights().is_none(), "an older parse cannot paint a newer draft");
    assert_eq!(active_view(&s).style_at(1, 9), (Some(Capture::String), 0));
    // A failed parse keeps the same safe fallback, rather than flashing every line again.
    let version = active_view(&s).snapshot().key().version;
    s.dispatch(Completion::Highlighted { version, parsed: None, folds: None, declarations: None, problems: None });
    assert_eq!(active_view(&s).style_at(2, 0), (Some(Capture::Comment), 0));
    edit(&mut s, vec![(23..27, "\"updated\"")]);
    assert_eq!(active_view(&s).style_at(1, 9), (None, 0), "replaced string waits for its new capture");
    assert_eq!(active_view(&s).style_at(0, 0), (Some(Capture::Keyword), 0));
    settle(&mut s, |s| active_view(s).highlights().is_some());
    assert_eq!(active_view(&s).style_at(1, 9), (Some(Capture::String), 0));
    assert_eq!(std::fs::read(path).unwrap(), saved.as_bytes());
}

#[cfg(unix)]
#[test]
fn pending_syntax_never_keeps_old_semantic_token_modifiers() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "Cargo.toml", b"[package]\nname = \"colours\"\n");
    let path = write(d.path(), "colours.rs", b"fn old_main() { let mut name = 1; }\n");
    let (mut s, _) = fake_semantic_session(d.path());
    open(&mut s, Command::OpenRetained(path), "colours.rs");
    settle(&mut s, |s| active_view(s).tokens().is_some() && s.state().highlights().is_some());
    assert_eq!(active_view(&s).style_at(0, 3), (Some(Capture::Function), crate::decor::DEPRECATED));
    assert_eq!(active_view(&s).style_at(0, 24), (Some(Capture::Variable), crate::decor::MUTABLE));
    let parsed = Arc::downgrade(s.state().active().unwrap().highlights.as_ref().unwrap());
    edit(&mut s, vec![(0..0, "\n")]);
    assert!(active_view(&s).tokens().is_none());
    assert_eq!(active_view(&s).style_at(1, 0), (Some(Capture::Keyword), 0));
    assert_eq!(active_view(&s).style_at(1, 3), (Some(Capture::Function), 0));
    assert_eq!(active_view(&s).style_at(1, 24).1, 0);
    settle(&mut s, |s| active_view(s).highlights().is_some());
    assert!(parsed.upgrade().is_none(), "an accepted parse releases the previous lexical arena");
}

#[test]
fn java_member_colour_survives_typing_and_fresh_lexical_parses_in_both_views() {
    let d = tempfile::tempdir().unwrap();
    let saved = "public static void main(String[] args) {\n    System.out.println(\"Hello Lenz!\");\n    \n}\n";
    let path = write(d.path(), "Main.java", saved.as_bytes());
    let mut s = session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "Main.java");
    settle(&mut s, |s| s.state().highlights().is_some());
    let snapshot = active_view(&s).snapshot().clone();
    let doc = Document::decode(path.clone(), snapshot.key().version, saved.as_bytes());
    let legend = crate::decor::Legend::new(&serde_json::json!({"tokenTypes":["property"],"tokenModifiers":["mutable","deprecated"]}));
    let tokens = Arc::new(crate::decor::decode_tokens(br#"{"result":{"data":[1,11,3,0,3]}}"#, &doc, semantics::lsp::Encoding::Utf16, &legend));
    let tokens_owner = Arc::downgrade(&tokens);
    let tab = s.state.active.unwrap();
    s.tab_mut(tab).unwrap().tokens = Some((snapshot.key().version, tokens));
    s.tab_mut(tab).unwrap().decor_source = Some(SemanticSource::new(&snapshot));
    assert_eq!(active_view(&s).style_at(1, 11), (Some(Capture::Property), crate::decor::MUTABLE | crate::decor::DEPRECATED));
    s.dispatch(Command::Split(Split::Columns));
    for letter in ["a", "b", "c"] {
        let at = active_view(&s).snapshot().reader_line_range(2).start + 4;
        edit(&mut s, vec![(at..at, letter)]);
        for tab in s.state.tabs() {
            let view = s.state.view(tab.id()).unwrap();
            assert!(view.tokens().is_none(), "stale semantic data must not be exposed as current");
            assert_eq!(view.style_at(1, 11), (Some(Capture::Property), 0), "unchanged out keeps its colour, without stale modifiers");
        }
        settle(&mut s, |s| active_view(s).highlights().is_some());
        for tab in s.state.tabs() { assert_eq!(s.state.view(tab.id()).unwrap().style_at(1, 11), (Some(Capture::Property), 0), "lexical arrival must not flash out back to its lexical colour"); }
    }
    assert!(tokens_owner.upgrade().is_some());
    let at = active_view(&s).snapshot().reader_line_range(1).start + 12;
    edit(&mut s, vec![(at..at, "x")]);
    assert_eq!(active_view(&s).style_at(1, 12), (None, 0), "inserted token bytes have no borrowed colour");
    assert_ne!(active_view(&s).style_at(1, 11).0, Some(Capture::Property), "a changed token's surviving fragments lose its old semantic colour");
    s.dispatch(Command::SetSemanticHighlighting(false));
    for tab in s.state.tabs() { assert_ne!(s.state.view(tab.id()).unwrap().style_at(1, 11).0, Some(Capture::Property)); }
    assert!(tokens_owner.upgrade().is_none(), "explicit disable releases the retained payload");
    assert_eq!(std::fs::read(path).unwrap(), saved.as_bytes());
}

#[cfg(unix)]
#[test]
fn retained_semantic_colour_rejects_stale_payload_and_releases_on_current_payload_or_restart() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "Cargo.toml", b"[package]\nname = \"colour-fence\"\n");
    let saved = "fn sample() {}\n// edit here\n";
    let path = write(d.path(), "colours.rs", saved.as_bytes());
    let (mut s, _) = fake_semantic_session(d.path());
    open(&mut s, Command::OpenRetained(path.clone()), "colours.rs");
    settle(&mut s, |s| active_view(s).tokens().is_some() && active_view(s).highlights().is_some());
    let snapshot = active_view(&s).snapshot().clone();
    let doc = Document::decode(path.clone(), snapshot.key().version, saved.as_bytes());
    let legend = crate::decor::Legend::new(&serde_json::json!({"tokenTypes":["property"]}));
    let property = Arc::new(crate::decor::decode_tokens(br#"{"result":{"data":[0,3,6,0,0]}}"#, &doc, semantics::lsp::Encoding::Utf16, &legend));
    let old = Arc::downgrade(&property);
    let tab = s.state.active.unwrap();
    s.tab_mut(tab).unwrap().tokens = Some((snapshot.key().version, property));
    edit(&mut s, vec![(25..25, "a")]);
    assert_eq!(active_view(&s).style_at(0, 3), (Some(Capture::Property), 0));
    let rx = s.completions();
    let stale = loop {
        let completion = rx.recv_blocking_timeout().unwrap();
        if matches!(&completion, Completion::Analyzed { result, .. } if matches!(result.as_ref(), Completion::Decorated { data: crate::decor::Decorated::Tokens(_), .. })) { break completion; }
        s.dispatch(completion);
    };
    edit(&mut s, vec![(25..25, "b")]);
    s.dispatch(stale);
    assert!(active_view(&s).tokens().is_none());
    assert_eq!(active_view(&s).style_at(0, 3), (Some(Capture::Property), 0));
    settle(&mut s, |s| active_view(s).tokens().is_some());
    assert_eq!(active_view(&s).style_at(0, 3), (Some(Capture::Function), 0));
    assert!(old.upgrade().is_none(), "current semantic colours release the previous payload");
    edit(&mut s, vec![(25..25, "c")]);
    assert!(s.state.active().unwrap().held_tokens.is_some());
    s.dispatch(Command::RestartServer);
    assert!(s.state.active().unwrap().held_tokens.is_none(), "provider restart drops old colour provenance");
    assert_eq!(std::fs::read(path).unwrap(), saved.as_bytes());
}

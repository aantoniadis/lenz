use std::{cell::Cell, path::Path, sync::Arc};

use super::*;
use crate::{
    document::{Placeholder, TextDocument},
    view::Side,
};

fn parse(source: &str) -> MarkdownDocument {
    parse_pair([source.into(), "".into()], Limits::DEFAULT, &|| true)
        .unwrap()
        .sides
        .into_iter()
        .next()
        .unwrap()
}

#[test]
fn markdown_parser_normalizes_entities_without_fabricating_source_bytes() {
    let source: Arc<str> = "# Title\r\n\r\nA &amp; **漢字** e\u{301}\\*\r\n".into();
    let pair = parse_pair([source.clone(), source.clone()], Limits::DEFAULT, &|| true).unwrap();
    assert!(pair.sides[0].text.contains("A & 漢字 e\u{301}*"));
    assert!(matches!(
        pair.sides[0].blocks[0].kind,
        BlockKind::Heading(1)
    ));
    for segment in &pair.sides[0].segments {
        if let Origin::Direct(r) | Origin::Normalized(r) = &segment.origin {
            assert!(source.get(r.clone()).is_some());
        }
    }
    assert!(
        pair.sides[0]
            .segments
            .iter()
            .any(|s| matches!(&s.origin, Origin::Normalized(r) if &source[r.clone()] == "&amp;"))
    );
    assert!(Arc::ptr_eq(&source, &pair.sides[0].source));
    assert_eq!(pair.sides[0].line_starts, [0, 9, 11, source.len()]);
}

#[test]
fn markdown_parser_limits_and_cancellation_return_no_partial_document() {
    let limits = Limits {
        bytes_per_side: 8,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        parse_pair(["too much Markdown".into(), "".into()], limits, &|| true),
        Err(MarkdownError::Limit(_))
    ));
    assert!(matches!(
        parse_pair(["# x".into(), "".into()], Limits::DEFAULT, &|| false),
        Err(MarkdownError::Cancelled)
    ));
}

#[test]
fn markdown_parser_inherits_nested_lists_quotes_tasks_and_inline_styles() {
    let doc = parse(
        "> 3. [x] **bold _nested_** ~~old~~ [link](https://example.test)\n>    - [ ] child\n\n- tail\n",
    );
    assert_eq!(doc.blocks.len(), 3);
    assert_eq!(
        (doc.blocks[0].quote_depth, doc.blocks[0].list_depth),
        (1, 1)
    );
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::ListItem { marker, checked: Some(true) } if marker == "3.")
    );
    assert_eq!(
        (doc.blocks[1].quote_depth, doc.blocks[1].list_depth),
        (1, 2)
    );
    assert!(matches!(
        doc.blocks[1].kind,
        BlockKind::ListItem {
            checked: Some(false),
            ..
        }
    ));
    assert_eq!(
        (doc.blocks[2].quote_depth, doc.blocks[2].list_depth),
        (0, 1)
    );
    let runs = &doc.blocks[0].runs;
    assert!(
        runs.iter()
            .any(|r| &doc.text[r.text.clone()] == "nested" && r.style.strong && r.style.emphasis)
    );
    assert!(
        runs.iter()
            .any(|r| &doc.text[r.text.clone()] == "old" && r.style.strike)
    );
    assert!(
        runs.iter()
            .any(|r| &doc.text[r.text.clone()] == "link" && r.style.link)
    );
    assert!(
        doc.segments
            .iter()
            .any(|s| matches!(s.origin, Origin::Synthetic)
                && doc.text[s.text.clone()].contains("3."))
    );
}

#[test]
fn markdown_parser_preserves_table_cells_alignment_and_fenced_language() {
    let doc = parse(
        "| Left | Center | Right |\n| :--- | :---: | ---: |\n| **α** | `b` | c |\n\n```rust extra\nlet x = 1;\n```\n",
    );
    assert_eq!(doc.blocks.len(), 3);
    for (i, header) in [(0, true), (1, false)] {
        let BlockKind::TableRow {
            header: actual,
            cells,
            alignment,
        } = &doc.blocks[i].kind
        else {
            panic!("expected table row")
        };
        assert_eq!(*actual, header);
        assert_eq!(
            alignment,
            &[
                TableAlignment::Left,
                TableAlignment::Center,
                TableAlignment::Right
            ]
        );
        let text: Vec<_> = cells.iter().map(|r| &doc.text[r.clone()]).collect();
        assert_eq!(
            text,
            if header {
                vec!["Left", "Center", "Right"]
            } else {
                vec!["α", "b", "c"]
            }
        );
    }
    assert!(matches!(&doc.blocks[2].kind, BlockKind::Code { language: Some(l) } if l == "rust"));
    assert_eq!(&doc.text[doc.blocks[2].text.clone()], "let x = 1;\n");
    assert!(doc.blocks[2].runs.iter().all(|r| r.style.code));
}

#[test]
fn markdown_parser_distinguishes_breaks_and_valid_unicode_segments() {
    let doc = parse("a\r\nb  \r\nc\\\r\nd ` e  f ` &copy; \\*\n");
    assert!(doc.text.contains("a b\nc\nd e  f © *"));
    assert!(doc.segments.iter().any(
        |s| matches!(&s.origin, Origin::Normalized(r) if &doc.source[r.clone()] == "\r\n")
            && &doc.text[s.text.clone()] == " "
    ));
    for segment in &doc.segments {
        assert!(doc.text.get(segment.text.clone()).is_some());
        if let Origin::Direct(r) = &segment.origin {
            assert_eq!(&doc.source[r.clone()], &doc.text[segment.text.clone()]);
        }
    }
}

#[test]
fn markdown_parser_handles_empty_added_and_deleted_sides() {
    for sources in [["", ""], ["# added", ""], ["", "# deleted"]] {
        let pair = parse_pair(sources.map(Arc::from), Limits::DEFAULT, &|| true).unwrap();
        for (source, doc) in sources.iter().zip(&pair.sides) {
            assert_eq!(doc.source.as_ref(), *source);
            assert_eq!(doc.blocks.is_empty(), source.is_empty());
            if source.is_empty() {
                assert!(doc.text.is_empty());
                assert_eq!(doc.line_starts, [0]);
            }
        }
    }
}

#[test]
fn markdown_parser_keeps_images_and_unsupported_content_readable() {
    let doc = parse(
        "before ![alt **text**](https://example.test/a.png) after\n\n<div>raw &amp; HTML</div>\n\n$math$ and [^note]\n\n[^note]: literal footnote\n\n---\n",
    );
    assert!(doc.text.contains("before "));
    assert!(doc.text.contains("alt text"));
    assert!(doc.text.contains("https://example.test/a.png"));
    assert!(doc.text.contains(" after"));
    assert!(doc.blocks.iter().any(|b| matches!(&b.kind, BlockKind::Image { alt, destination } if alt == "alt text" && destination == "https://example.test/a.png")));
    assert!(
        doc.blocks
            .iter()
            .any(|b| matches!(b.kind, BlockKind::Literal)
                && doc.text[b.text.clone()].contains("<div>raw &amp; HTML</div>"))
    );
    assert!(doc.text.contains("$math$ and 1"), "{:?}", doc.text);
    assert!(doc.blocks.iter().any(|b| matches!(&b.kind, BlockKind::Footnote { label } if label == "1")
        && &doc.text[b.text.clone()] == "literal footnote\n"));
    assert!(doc.blocks.iter().any(|b| matches!(b.kind, BlockKind::Rule)));
}

#[test]
fn markdown_parser_retains_hidden_reference_and_comment_origins() {
    let doc = parse(
        "[label][target]\n\n[target]: https://example.test \"Title\"\n\n<!-- hidden\ncomment -->\n\nvisible <!-- inline --> end\n",
    );
    assert!(doc.text.contains("label"));
    assert!(doc.text.contains("visible  end"));
    assert!(!doc.text.contains("hidden"));
    assert!(!doc.text.contains("[target]:"));
    for hidden in [
        "[target]: https://example.test \"Title\"",
        "<!-- hidden\ncomment -->",
        "<!-- inline -->",
    ] {
        assert!(doc.segments.iter().any(|s| s.text.is_empty() && matches!(&s.origin, Origin::Normalized(r) | Origin::Direct(r) if doc.source[r.clone()].contains(hidden))), "missing hidden origin {hidden}");
    }
}

#[test]
fn markdown_parser_enforces_exact_bytes_events_and_pair_segments_boundaries() {
    let sources = [Arc::<str>::from("x"), Arc::<str>::from("y")];
    let doc = parse_pair(sources.clone(), Limits::DEFAULT, &|| true).unwrap();
    let segments = doc.sides.iter().map(|d| d.segments.len()).sum();
    assert!(
        parse_pair(
            sources.clone(),
            Limits {
                bytes_per_side: 1,
                events_per_side: 3,
                segments_per_pair: segments,
                ..Limits::DEFAULT
            },
            &|| true
        )
        .is_ok()
    );
    for limits in [
        Limits {
            bytes_per_side: 0,
            ..Limits::DEFAULT
        },
        Limits {
            events_per_side: 2,
            ..Limits::DEFAULT
        },
        Limits {
            segments_per_pair: segments - 1,
            ..Limits::DEFAULT
        },
    ] {
        assert!(matches!(
            parse_pair(sources.clone(), limits, &|| true),
            Err(MarkdownError::Limit(_))
        ));
    }
    let calls = Cell::new(0);
    assert!(matches!(
        parse_pair(["a **b** c".into(), "y".into()], Limits::DEFAULT, &|| {
            calls.set(calls.get() + 1);
            calls.get() < 5
        }),
        Err(MarkdownError::Cancelled)
    ));
}

#[test]
fn markdown_parser_eligibility_uses_document_language_and_rejects_placeholders() {
    for extension in ["md", "markdown", "mdown", "mkd", "MD"] {
        let name = format!("--name.{extension}");
        let doc = TextDocument::new("", "# named", Path::new("temporary"), Path::new(&name));
        assert!(eligible(&doc), "{name}");
        assert!(Arc::ptr_eq(
            &doc.text_arc(Side::Right),
            &doc.text_arc(Side::Right)
        ));
    }
    assert!(!eligible(&TextDocument::new(
        "# heading",
        "",
        Path::new("x.txt"),
        Path::new("y.txt")
    )));
    for placeholder in [
        Placeholder::Binary {
            sizes: [1, 2],
            differ: true,
        },
        Placeholder::Large { bytes: 5 },
        Placeholder::Unreadable("failed".into()),
    ] {
        assert!(!eligible(&TextDocument::placeholder(
            Path::new("x.md"),
            Path::new("y.md"),
            placeholder
        )));
    }
}

#[test]
fn markdown_parser_keeps_resolved_link_targets_and_interns_reference_metadata() {
    let doc = parse(
        "[one][ref] and [**two**][ref] and [inline](https://inline.test \"inline title\")\n\n[ref]: https://reference.test \"reference title\"\n",
    );
    assert_eq!(doc.links.len(), 3);
    for (link, label) in doc.links.iter().zip(["one", "two", "inline"]) {
        assert_eq!(&doc.text[link.text.clone()], label);
        assert!(doc.source.get(link.source.clone()).is_some());
    }
    assert_eq!(doc.links[0].destination.as_ref(), "https://reference.test");
    assert_eq!(doc.links[0].title.as_ref(), "reference title");
    assert!(Arc::ptr_eq(
        &doc.links[0].destination,
        &doc.links[1].destination
    ));
    assert!(Arc::ptr_eq(&doc.links[0].title, &doc.links[1].title));
    assert_eq!(doc.links[2].destination.as_ref(), "https://inline.test");
    assert_eq!(doc.links[2].title.as_ref(), "inline title");
}

#[test]
fn markdown_parser_rejects_reference_image_expansion_and_exact_output_boundaries() {
    let sources = [Arc::<str>::from("x"), Arc::<str>::from("y")];
    assert!(
        parse_pair(
            sources.clone(),
            Limits {
                rendered_bytes_per_side: 1,
                ..Limits::DEFAULT
            },
            &|| true
        )
        .is_ok()
    );
    assert!(matches!(
        parse_pair(
            sources,
            Limits {
                rendered_bytes_per_side: 0,
                ..Limits::DEFAULT
            },
            &|| true
        ),
        Err(MarkdownError::Limit(_))
    ));
    let source: Arc<str> = format!(
        "{}\n\n[image]: https://example.test/{}\n",
        "![alt][image]\n".repeat(200),
        "a".repeat(4096)
    )
    .into();
    assert!(source.len() < 8192);
    assert!(matches!(
        parse_pair(
            [source, "".into()],
            Limits {
                rendered_bytes_per_side: 8192,
                ..Limits::DEFAULT
            },
            &|| true
        ),
        Err(MarkdownError::Limit("rendered text bytes"))
    ));
    let placeholder = "[Image: alt (dest)]";
    assert!(
        parse_pair(
            ["![alt](dest)".into(), "".into()],
            Limits {
                rendered_bytes_per_side: placeholder.len(),
                ..Limits::DEFAULT
            },
            &|| true
        )
        .is_ok()
    );
    assert!(matches!(
        parse_pair(
            ["![alt](dest)".into(), "".into()],
            Limits {
                rendered_bytes_per_side: placeholder.len() - 1,
                ..Limits::DEFAULT
            },
            &|| true
        ),
        Err(MarkdownError::Limit(_))
    ));
}

#[test]
fn markdown_parser_marks_escape_origins_and_preserves_unemitted_source() {
    let doc = parse("\\* escaped\n\n[ref]: first\n[ref]: second\n\n<!-- hidden only -->\n");
    assert!(
        doc.segments
            .iter()
            .any(|s| matches!(&s.origin, Origin::Normalized(r) if &doc.source[r.clone()] == "\\*")),
        "{:?}",
        doc.segments
    );
    for text in ["[ref]: first", "[ref]: second"] {
        assert!(doc.segments.iter().any(|s| matches!(&s.origin, Origin::Normalized(r) | Origin::Direct(r) if doc.source[r.clone()].contains(text))) || doc.text.contains(text), "lost {text}");
    }
    let empty = parse("<!-- only hidden\ncomment -->\n");
    assert!(empty.blocks.is_empty(), "{:?}", empty.blocks);
    assert!(empty.text.is_empty());
}

#[test]
fn markdown_parser_preserves_footnote_definitions_in_source_order() {
    let doc = parse("before\n\n[^note]: first\n[^note]: second\n\nafter [^note]\n");
    assert!(doc.text.contains("after 1"));
    let first = doc.text.find("first").unwrap();
    let second = doc.text.find("second").unwrap();
    assert!(doc.text.find("before").unwrap() < first);
    assert!(first < second && second < doc.text.find("after").unwrap());
}

#[test]
fn markdown_parser_keeps_nested_empty_list_markers_and_table_images() {
    let doc = parse("-\n  - child\n\n| Image |\n| --- |\n| ![alt](dest) |\n");
    assert!(matches!(doc.blocks[0].kind, BlockKind::ListItem { .. }));
    assert_eq!(doc.blocks[0].list_depth, 1);
    assert!(matches!(doc.blocks[1].kind, BlockKind::ListItem { .. }));
    assert_eq!(doc.blocks[1].list_depth, 2);
    assert_eq!(&doc.text[doc.blocks[1].text.clone()], "• child");
    let BlockKind::TableRow { cells, .. } = &doc.blocks[3].kind else {
        panic!("image broke table row")
    };
    assert_eq!(cells.len(), 1);
    assert_eq!(&doc.text[cells[0].clone()], "[Image: alt (dest)]");
}

#[test]
fn markdown_parser_long_reference_links_share_targets_below_the_expansion_limit() {
    let source = format!(
        "{}\n\n[ref]: https://example.test/{}\n",
        "[label][ref] ".repeat(200),
        "a".repeat(256)
    );
    let doc = parse(&source);
    assert_eq!(doc.links.len(), 200);
    assert!(
        doc.links
            .iter()
            .all(|link| Arc::ptr_eq(&link.destination, &doc.links[0].destination))
    );
    assert_eq!(doc.text.matches("label").count(), 200);
}

#[test]
fn markdown_parser_rejects_upstream_reference_exhaustion_before_literal_degradation() {
    let source = format!(
        "{}\n\n[ref]: https://example.test/{}\n",
        "[label][ref] ".repeat(200),
        "a".repeat(4096)
    );
    assert!(matches!(
        parse_pair(
            ["complete left".into(), source.into()],
            Limits::DEFAULT,
            &|| true
        ),
        Err(MarkdownError::Limit("reference expansion"))
    ));
}

#[test]
fn markdown_parser_keeps_nested_image_alt_content_in_one_placeholder() {
    let doc = parse("![outer [link](https://link.test) ![inner](inner.png)](outer.png)\n");
    assert_eq!(doc.blocks.len(), 1, "{:?}", doc.blocks);
    assert!(doc.links.is_empty());
    assert!(
        matches!(&doc.blocks[0].kind, BlockKind::Image { alt, destination } if alt == "outer link inner" && destination == "outer.png")
    );
    assert_eq!(doc.text, "[Image: outer link inner (outer.png)]");
}

#[test]
fn markdown_parser_places_list_markers_before_nonparagraph_leaf_blocks() {
    let doc = parse("- ```rust\n  x\n  ```\n\n- # heading\n");
    assert!(matches!(doc.blocks[0].kind, BlockKind::ListItem { .. }));
    assert!(matches!(doc.blocks[1].kind, BlockKind::Code { .. }));
    assert!(matches!(doc.blocks[2].kind, BlockKind::ListItem { .. }));
    assert!(matches!(doc.blocks[3].kind, BlockKind::Heading(1)));
    assert!(doc.blocks.iter().all(|b| b.list_depth == 1));
}

#[test]
fn markdown_parser_reference_expansion_accounting_is_independent_per_side() {
    let source = |count| {
        Arc::<str>::from(format!(
            "{}\n\n[r]: {}\n",
            "[l][r] ".repeat(count),
            "a".repeat(1000)
        ))
    };
    assert!(parse_pair([source(99), source(99)], Limits::DEFAULT, &|| true).is_ok());
    assert!(matches!(
        parse_pair([source(100), "".into()], Limits::DEFAULT, &|| true),
        Err(MarkdownError::Limit("reference expansion"))
    ));
}

#[test]
fn markdown_parser_footnotes_keep_multiword_bodies_literal() {
    let source = "[^note]: paragraph with **bold** and [link](destination)\n";
    let doc = parse(source);
    assert_eq!(doc.blocks.len(), 1);
    assert!(matches!(doc.blocks[0].kind, BlockKind::Footnote { .. }));
    assert_eq!(doc.text, &source[9..]);
    assert!(
        doc.blocks[0]
            .runs
            .iter()
            .all(|run| run.style == InlineStyle::default())
    );
    assert!(doc.links.is_empty());
    assert!(
        matches!(&doc.segments[0].origin, Origin::Normalized(range) if &doc.source[range.clone()] == "[^note]: ")
    );
    assert!(
        matches!(&doc.segments[1].origin, Origin::Direct(range) if doc.source[range.clone()] == source[9..])
    );
}

#[test]
fn markdown_parser_footnotes_keep_indented_formatted_continuations_literal() {
    let literal = "[^note]: first\n    continuation **bold**\n\n    another *line*\n";
    let doc = parse(&format!("{literal}\nafter **formatted**\n"));
    let block = &doc.blocks[0];
    assert!(matches!(block.kind, BlockKind::Footnote { .. }));
    assert_eq!(&doc.text[block.text.clone()], &literal[9..]);
    assert_eq!(&doc.source[block.source.clone()], literal);
    assert!(
        block
            .runs
            .iter()
            .all(|run| run.style == InlineStyle::default())
    );
    assert_eq!(doc.text.matches("continuation").count(), 1);
    assert_eq!(doc.text.matches("another").count(), 1);
    assert_eq!(&doc.text[doc.blocks[1].text.clone()], "after formatted");
    assert!(doc.blocks[1].runs.iter().any(|run| run.style.strong));
}

#[test]
fn markdown_parser_footnotes_preserve_quote_and_list_context_and_wrapper_origins() {
    for (source, literal, quote_depth, list_depth) in [
        (
            "> [^note]: paragraph with **bold**\n>     continuation *kept*\n>\n> after\n",
            "[^note]: paragraph with **bold**\n>     continuation *kept*\n",
            1,
            0,
        ),
        (
            "- [^note]: paragraph with **bold**\n      continuation *kept*\n\n- after\n",
            "[^note]: paragraph with **bold**\n      continuation *kept*\n",
            0,
            1,
        ),
        (
            "> - [^note]: paragraph with **bold**\n>       continuation *kept*\n>\n> - after\n",
            "[^note]: paragraph with **bold**\n>       continuation *kept*\n",
            1,
            1,
        ),
        (
            "- > [^note]: paragraph with **bold**\n  >     continuation *kept*\n\n- after\n",
            "[^note]: paragraph with **bold**\n  >     continuation *kept*\n",
            1,
            1,
        ),
    ] {
        let doc = parse(source);
        let block = doc
            .blocks
            .iter()
            .find(|block| matches!(block.kind, BlockKind::Footnote { .. }))
            .unwrap();
        assert_eq!(
            (block.quote_depth, block.list_depth),
            (quote_depth, list_depth)
        );
        assert_eq!(&doc.text[block.text.clone()], &literal[9..]);
        assert_eq!(&doc.source[block.source.clone()], literal);
        assert!(
            block
                .runs
                .iter()
                .all(|run| run.style == InlineStyle::default())
        );
        assert_eq!(doc.text.matches("paragraph").count(), 1);
        assert_eq!(doc.text.matches("continuation").count(), 1);
        assert!(doc.text.contains("after"));
        let marker = source.find("[^note]").unwrap();
        for byte in 0..marker {
            assert!(doc.segments.iter().any(|segment| matches!(&segment.origin, Origin::Direct(range) | Origin::Normalized(range) if range.contains(&byte)))
                || doc.blocks.iter().any(|block| block.source.contains(&byte)), "missing wrapper byte {byte} in {source:?}");
        }
    }
}

#[test]
fn markdown_projection_keeps_changed_reference_targets_navigable() {
    let left = "Read [guide][g].\n\n[g]: https://example.invalid/old\n";
    let right = "Read [guide][g].\n\n[g]: https://example.invalid/new\n";
    let doc = TextDocument::new(left, right, Path::new("a.md"), Path::new("b.md"));
    let view = doc.view(&doc.line_diff(&lenz_engine::Options::default()), 20.);
    let count = view.change_count();
    let pair = Arc::new(
        parse_pair(
            [doc.text_arc(Side::Left), doc.text_arc(Side::Right)],
            Limits::DEFAULT,
            &|| true,
        )
        .unwrap(),
    );
    let projection = project(pair, source_diff(&doc, &view, 1), &|| true).unwrap();
    for id in 0..count {
        assert!(projection.represents_hunk(id));
    }
    assert!(projection.decorations.iter().any(|d| d.markup));
    assert!(projection.location(0, Side::Right).is_some());
}

fn projection_fixture(
    left: &str,
    right: &str,
    options: lenz_engine::Options,
) -> (Projection, SourceDiff) {
    let doc = TextDocument::new(left, right, Path::new("a.md"), Path::new("b.md"));
    let mut view = doc.view(&doc.line_diff(&options), 1000.);
    doc.refine_visible(&mut view, &options, &mut lenz_engine::Budget::default());
    let diff = source_diff(&doc, &view, 7);
    assert_eq!(diff.hunks.len(), view.change_count());
    let pair = Arc::new(
        parse_pair(
            [doc.text_arc(Side::Left), doc.text_arc(Side::Right)],
            Limits::DEFAULT,
            &|| true,
        )
        .unwrap(),
    );
    (project(pair, diff.clone(), &|| true).unwrap(), diff)
}

#[test]
fn hunks_inside_one_image_placeholder_compare_it_once() {
    // 12,000 alt-text lines, every other one differing only by a trailing space: 6,000 hunks that all map to the
    // whole placeholder. Compared per hunk this took tens of seconds; compared once it is a parse.
    let lines = |pad: bool| (0..12_000).map(|i| if pad && i % 2 == 0 { format!("line {i} \n") } else { format!("line {i}\n") }).collect::<String>();
    let (left, right) = (format!("![{}](x)", lines(false)), format!("![{}](x)", lines(true)));
    let started = std::time::Instant::now();
    let (projection, diff) = projection_fixture(&left, &right, lenz_engine::Options::default());
    assert!(diff.hunks.len() >= 6_000, "{} hunks", diff.hunks.len());
    assert!(projection.represents_hunk(0));
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

#[test]
fn markdown_projection_preserves_engine_hunks_in_all_comparison_modes() {
    use lenz_engine::{HighlightLevel, Options, WhitespaceMode};
    let fixtures = [
        ("**word**\n", "*word*\n"),
        ("<!-- old -->\n", "<!-- new -->\n"),
        (
            "first old\nsecond same\nthird old\n",
            "first new\nsecond same\nthird new\n",
        ),
        ("# before\n\n# after\n", "# before\n\ninserted\n\n# after\n"),
        (
            "> | A | B |\n> | --- | --- |\n> | x | y |\n",
            "> | A | B |\n> | --- | --- |\n> | z | y |\n",
        ),
        ("- same word\n\nending\n", "- same  word\n\nending\n"),
        (
            "start\r\n\r\n**α old**\r\nnext old\r\n",
            "start\r\n\r\n**α new**\r\nnext new\r\n",
        ),
        ("[![alt](old.png)](target)\n", "[![alt](new.png)](target)\n"),
        (
            "before ![alt](image.png) old\n",
            "before ![alt](image.png) new\n",
        ),
        (
            "before ![a](one.png) between ![b](two.png) old\n",
            "before ![a](one.png) between ![b](two.png) new\n",
        ),
        (
            "before ![alt](image.png) after\n",
            "before ![alt](image.png) after\n",
        ),
        (
            "before ![a](one.png) between ![b](two.png) after\n",
            "before ![a](one.png) between ![b](two.png) after\n",
        ),
        (
            "before\n![alt](image.png)\nafter\n",
            "before\ninserted\n![alt](image.png)\nafter\n",
        ),
        (
            "before\n![alt](image.png)\nafter\n",
            "before\n![alt](image.png)\ninserted\nafter\n",
        ),
        (
            "before ![alt](image.png) after\n",
            "inserted\n\nbefore ![alt](image.png) after\n",
        ),
        (
            "before ![alt](image.png) after\n",
            "before ![alt](image.png) after\n\ninserted\n",
        ),
        (
            "read [label][r]\n\n[r]: old\n",
            "read [label][r]\n\n[r]: new\n",
        ),
    ];
    for highlight in [
        HighlightLevel::Word,
        HighlightLevel::Char,
        HighlightLevel::Line,
    ] {
        for whitespace in [
            WhitespaceMode::Default,
            WhitespaceMode::Trim,
            WhitespaceMode::Ignore,
            WhitespaceMode::IgnoreTrimChunks,
        ] {
            for (left, right) in fixtures {
                let (projection, diff) = projection_fixture(
                    left,
                    right,
                    Options {
                        highlight,
                        whitespace,
                        ignore_case: false,
                    },
                );
                for hunk in &diff.hunks {
                    assert!(
                        projection.represents_hunk(hunk.id),
                        "{highlight:?} {whitespace:?} {left:?}"
                    );
                    for side in [Side::Left, Side::Right] {
                        assert_eq!(
                            projection.location(hunk.id, side).unwrap().byte,
                            hunk.bytes[side as usize].start
                        );
                    }
                }
                let mut ends = [0, 0];
                for group in &projection.groups {
                    for side in [Side::Left, Side::Right] {
                        let range = &group.blocks[side as usize];
                        assert_eq!(range.start, ends[side as usize]);
                        assert!(
                            range.start <= range.end
                                && range.end <= projection.pair.sides[side as usize].blocks.len()
                        );
                        ends[side as usize] = range.end;
                    }
                }
                assert_eq!(
                    ends,
                    [
                        projection.pair.sides[0].blocks.len(),
                        projection.pair.sides[1].blocks.len()
                    ]
                );
                for decoration in &projection.decorations {
                    let doc = &projection.pair.sides[decoration.side as usize];
                    assert!(doc.text.get(decoration.text.clone()).is_some());
                    assert!(doc.source.get(decoration.source.clone()).is_some());
                    assert_eq!(decoration.ignored, diff.hunks[decoration.hunk].ignored);
                }
            }
        }
    }
}

#[test]
fn markdown_projection_marks_unrendered_changes_and_merges_paragraph_hunks() {
    for (left, right) in [("**word**\n", "*word*\n"), ("<!-- a -->\n", "<!-- b -->\n")] {
        let (projection, diff) = projection_fixture(left, right, lenz_engine::Options::default());
        assert!(!diff.hunks.is_empty());
        assert!(projection.decorations.iter().all(|d| d.markup));
        assert!(projection.groups.iter().any(|g| !g.hunks.is_empty()));
    }
    let (projection, diff) = projection_fixture(
        "old\nsame\nold\n",
        "new\nsame\nnew\n",
        lenz_engine::Options::default(),
    );
    assert!(diff.hunks.len() > 1);
    let changed: Vec<_> = projection
        .groups
        .iter()
        .filter(|g| !g.hunks.is_empty())
        .collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(
        changed[0].hunks,
        diff.hunks.iter().map(|h| h.id).collect::<Vec<_>>()
    );
    assert_eq!(changed[0].blocks, [0..1, 0..1]);
}

#[test]
fn markdown_projection_anchors_insertions_without_consuming_the_next_block() {
    let (projection, diff) = projection_fixture(
        "# before\n\n# after\n",
        "# before\n\ninserted\n\n# after\n",
        lenz_engine::Options::default(),
    );
    let changed = projection
        .groups
        .iter()
        .find(|g| !g.hunks.is_empty())
        .unwrap();
    assert_eq!(changed.blocks, [1..1, 1..2]);
    assert_eq!(
        projection
            .location(diff.hunks[0].id, Side::Left)
            .unwrap()
            .byte,
        "# before\n\n".len()
    );
}

#[test]
fn markdown_projection_keeps_inline_image_wrappers_in_rendered_order() {
    for (source, count) in [
        ("before ![alt](image.png) after\n", 3),
        ("before ![a](one.png) between ![b](two.png) after\n", 5),
    ] {
        let (projection, diff) =
            projection_fixture(source, source, lenz_engine::Options::default());
        assert!(diff.hunks.is_empty());
        assert!(projection.decorations.is_empty());
        assert_eq!(projection.groups.len(), 1);
        assert_eq!(projection.groups[0].blocks, [0..count, 0..count]);
        assert!(projection.groups[0].hunks.is_empty());
        assert!(
            projection.pair.sides[0]
                .blocks
                .windows(2)
                .any(|blocks| blocks[0].source.start > blocks[1].source.start)
        );

        let left = format!("{source}\n# following\n");
        let right = format!("{source}\ninserted\n\n# following\n");
        let (projection, diff) = projection_fixture(&left, &right, lenz_engine::Options::default());
        let changed = projection
            .groups
            .iter()
            .find(|group| !group.hunks.is_empty())
            .unwrap();
        assert_eq!(changed.blocks, [count..count, count..count + 1]);
        for hunk in diff.hunks {
            assert!(projection.represents_hunk(hunk.id));
            assert_eq!(
                projection.location(hunk.id, Side::Left).unwrap().byte,
                source.len() + 1
            );
        }
    }
}

#[test]
fn markdown_projection_expands_insertions_inside_inline_image_wrappers() {
    let left = "before\n![alt](image.png)\nafter\n";
    for right in [
        "before\ninserted\n![alt](image.png)\nafter\n",
        "before\n![alt](image.png)\ninserted\nafter\n",
    ] {
        let (projection, diff) = projection_fixture(left, right, lenz_engine::Options::default());
        assert!(!diff.hunks.is_empty());
        assert_eq!(projection.groups.len(), 1);
        assert_eq!(projection.groups[0].blocks, [0..3, 0..3]);
        assert_eq!(
            projection.groups[0].hunks,
            diff.hunks.iter().map(|hunk| hunk.id).collect::<Vec<_>>()
        );
        for hunk in diff.hunks {
            assert!(hunk.bytes[0].is_empty());
            assert!(projection.represents_hunk(hunk.id));
            assert_eq!(
                projection.location(hunk.id, Side::Left).unwrap().byte,
                hunk.bytes[0].start
            );
        }
    }
}

#[test]
fn markdown_projection_maps_crlf_refinements_into_decoded_source() {
    let (projection, diff) = projection_fixture(
        "**α old**\r\nnext old\r\n",
        "**α new**\r\nnext new\r\n",
        lenz_engine::Options::default(),
    );
    for side in [Side::Left, Side::Right] {
        let doc = &projection.pair.sides[side as usize];
        let expected = if side == Side::Left { "old" } else { "new" };
        let spans = &diff.hunks[0].inline[side as usize];
        assert_eq!(spans.len(), 2);
        assert!(
            spans
                .iter()
                .all(|span| &doc.source[span.clone()] == expected)
        );
        let highlights: Vec<_> = projection
            .decorations
            .iter()
            .filter(|d| d.side == side && !d.markup)
            .collect();
        assert_eq!(highlights.len(), 2);
        assert!(
            highlights
                .iter()
                .all(|d| &doc.text[d.text.clone()] == expected)
        );
    }
}

#[test]
fn markdown_projection_reprojects_shared_parsing_and_rejects_invalid_maps_or_cancellation() {
    let (projection, mut diff) =
        projection_fixture("old\n", "new\n", lenz_engine::Options::default());
    let pair = projection.pair.clone();
    diff.revision += 1;
    let newer = project(pair.clone(), diff.clone(), &|| true).unwrap();
    assert!(Arc::ptr_eq(&pair, &newer.pair));
    assert_eq!(newer.revision, diff.revision);
    assert!(matches!(
        project(pair.clone(), diff.clone(), &|| false),
        Err(MarkdownError::Cancelled)
    ));
    diff.hunks[0].bytes[1].end = pair.sides[1].source.len() + 1;
    assert!(matches!(
        project(pair, diff, &|| true),
        Err(MarkdownError::InvalidMap)
    ));
}

#[test]
fn markdown_projection_rejects_corrupted_segment_and_line_maps() {
    let (projection, diff) = projection_fixture("old\n", "new\n", lenz_engine::Options::default());
    let sources = [
        projection.pair.sides[0].source.clone(),
        projection.pair.sides[1].source.clone(),
    ];
    for corruption in 0..4 {
        let mut pair = parse_pair(sources.clone(), Limits::DEFAULT, &|| true).unwrap();
        match corruption {
            0 => {
                pair.sides[0].segments.clear();
            }
            1 => {
                pair.sides[1].line_starts[0] = 1;
            }
            2 => {
                pair.sides[0].segments[0].text.start += 1;
            }
            3 => {
                pair.sides[1].segments[0].origin = Origin::Direct(0..2);
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                project(Arc::new(pair), diff.clone(), &|| true),
                Err(MarkdownError::InvalidMap)
            ),
            "corruption {corruption}"
        );
    }
}

#[test]
fn markdown_projection_keeps_empty_and_eof_boundaries_and_cancels_in_progress() {
    for (left, right) in [("", "# α\n"), ("# α\n", ""), ("# α\n", "# α\n\nβ\n")] {
        let (projection, diff) = projection_fixture(left, right, lenz_engine::Options::default());
        assert!(!diff.hunks.is_empty());
        for hunk in diff.hunks {
            for side in [Side::Left, Side::Right] {
                let anchor = projection.location(hunk.id, side).unwrap();
                assert!(
                    projection.pair.sides[side as usize]
                        .source
                        .is_char_boundary(anchor.byte)
                );
            }
        }
    }
    let left = "paragraph old\n\n".repeat(300);
    let right = "paragraph new\n\n".repeat(300);
    let (projection, diff) = projection_fixture(&left, &right, lenz_engine::Options::default());
    let calls = Cell::new(0);
    assert!(matches!(
        project(projection.pair, diff, &|| {
            calls.set(calls.get() + 1);
            calls.get() < 10
        }),
        Err(MarkdownError::Cancelled)
    ));
    assert_eq!(calls.get(), 10);
}

fn state_doc(name: &str, text: &str) -> Arc<TextDocument> {
    Arc::new(TextDocument::new(text, "# right\n", Path::new(name), Path::new(name)))
}
fn state_diff(doc: &TextDocument, revision: u64) -> SourceDiff {
    let view = doc.view(&doc.line_diff(&lenz_engine::Options::default()), 20.0);
    source_diff(doc, &view, revision)
}
fn state_render(state: &mut MarkdownState, doc: &Arc<TextDocument>, revision: u64) {
    state.execute(MarkdownCommand::SetMode(PresentationMode::Rendered), Some(doc.clone()), Some(state_diff(doc, revision)));
}
fn state_settle(state: &mut MarkdownState) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while state.loading() {
        state.poll();
        assert!(std::time::Instant::now() < until, "Markdown worker did not settle");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
#[test]
fn markdown_state_starts_source_and_invalidate_retains_preference_only() {
    let mut state = MarkdownState::new();
    assert_eq!(state.mode(), PresentationMode::Source);
    assert!(state.projection().is_none());
    assert!(!state.loading());
    let doc = state_doc("test.md", "# old");
    state_render(&mut state, &doc, 1);
    state.invalidate();
    assert!(!state.loading());
    assert!(state.projection().is_none());
    assert_eq!(state.mode(), PresentationMode::Source);
    assert_eq!(state.preferred_mode(), PresentationMode::Rendered);
    assert!(!state.poll());
}
#[test]
fn markdown_state_replacement_cannot_install_abandoned_document() {
    let old = state_doc("old.md", "# old");
    let new = state_doc("new.md", "# new");
    let mut state = MarkdownState::new();
    state_render(&mut state, &old, 1);
    state_render(&mut state, &new, 2);
    assert!(state.projection().is_none());
    state_settle(&mut state);
    assert_eq!(state.projection().unwrap().revision, 2);
    assert_eq!(&*state.projection().unwrap().pair.sides[0].source, "# new");
}
#[test]
fn markdown_state_reuses_ready_pair_for_toggles_options_and_refinement() {
    let doc = state_doc("test.md", "**old**");
    let mut state = MarkdownState::new();
    state_render(&mut state, &doc, 1);
    state_settle(&mut state);
    let ready = state.projection().unwrap().clone();
    state_render(&mut state, &doc, 1);
    assert!(!state.loading());
    state.execute(MarkdownCommand::SetMode(PresentationMode::Source), Some(doc.clone()), None);
    assert_eq!(state.mode(), PresentationMode::Source);
    state_render(&mut state, &doc, 1);
    assert!(!state.loading());
    assert!(Arc::ptr_eq(&ready, state.projection().unwrap()));
    state_render(&mut state, &doc, 2);
    assert!(state.loading());
    assert!(Arc::ptr_eq(&ready, state.projection().unwrap()));
    state_settle(&mut state);
    assert!(Arc::ptr_eq(&ready.pair, &state.projection().unwrap().pair));
    assert_eq!(state.projection().unwrap().revision, 2);
}
#[test]
fn markdown_state_limit_failure_falls_back_to_source_with_notice() {
    let doc = state_doc("test.md", &"x".repeat(Limits::DEFAULT.bytes_per_side + 1));
    let mut state = MarkdownState::new();
    state_render(&mut state, &doc, 1);
    state_settle(&mut state);
    assert_eq!(state.mode(), PresentationMode::Source);
    assert!(state.notice().is_some());
    assert!(state.projection().is_none());
}
#[test]
fn markdown_state_preference_survives_markdown_text_markdown_sequence() {
    let md = state_doc("test.md", "# heading");
    let text = state_doc("test.txt", "plain text");
    let mut state = MarkdownState::new();
    state_render(&mut state, &md, 1);
    state_settle(&mut state);
    state_render(&mut state, &text, 2);
    assert_eq!(state.mode(), PresentationMode::Source);
    assert!(state.projection().is_none());
    assert!(!state.loading());
    assert_eq!(state.preferred_mode(), PresentationMode::Rendered);
    state_render(&mut state, &md, 3);
    state_settle(&mut state);
    assert_eq!(state.mode(), PresentationMode::Rendered);
}

#[test]
fn markdown_state_comparison_owner_is_lazy_and_reprojects_refinements() {
    use crate::comparison_session::ComparisonSession;
    let doc = state_doc("test.md", "**old**");
    let mut owner = ComparisonSession::new(doc.view(&[], 20.0), lenz_engine::Options::default());
    owner.markdown_refresh(Some(&doc));
    owner.markdown_command(MarkdownCommand::SetMode(PresentationMode::Source), Some(doc.clone()));
    assert!(!owner.markdown_loading());
    assert!(owner.markdown_projection().is_none());
    owner.markdown_command(MarkdownCommand::SetMode(PresentationMode::Rendered), Some(doc.clone()));
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while owner.markdown_loading() { owner.markdown_poll(); assert!(std::time::Instant::now() < until); std::thread::yield_now(); }
    let first = owner.markdown_projection().unwrap().clone();
    owner.markdown_refresh(Some(&doc));
    assert!(Arc::ptr_eq(&first, owner.markdown_projection().unwrap()));
    while owner.markdown_loading() { owner.markdown_poll(); assert!(std::time::Instant::now() < until); std::thread::yield_now(); }
    let next = owner.markdown_projection().unwrap();
    assert!(next.revision > first.revision);
    assert!(Arc::ptr_eq(&first.pair, &next.pair));
    assert_eq!(owner.presentation_disabled(PresentationMode::Rendered, None), Some("Comparison loading"));
    assert_eq!(owner.presentation_disabled(PresentationMode::Rendered, Some(&state_doc("a.txt", "plain"))), Some("Not a Markdown comparison"));
}

#[test]
fn markdown_state_session_owner_polls_and_restores_preference_after_file_switches() {
    use crate::{changeset::{ChangeSet, Spec}, commands::Command, session::Session, view::ScrollMode};
    let dir = tempfile::tempdir().unwrap();
    for side in ["left", "right"] {
        std::fs::create_dir(dir.path().join(side)).unwrap();
        for name in ["a.md", "b.txt", "c.md"] { std::fs::write(dir.path().join(side).join(name), format!("# {side}\n")).unwrap(); }
    }
    let spec = Spec::Plain { left: dir.path().join("left"), right: dir.path().join("right") };
    let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
    let mut session = Session::from_set(set, spec, lenz_engine::Options::default(), ScrollMode::Shared);
    let settle = |session: &mut Session| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while session.busy() { session.poll(); assert!(std::time::Instant::now() < until); std::thread::sleep(std::time::Duration::from_millis(1)); }
    };
    settle(&mut session);
    assert_eq!(session.markdown_mode(), PresentationMode::Source);
    session.execute(Command::SetPresentation(PresentationMode::Rendered));
    assert!(session.busy());
    settle(&mut session);
    let pair = Arc::downgrade(&session.markdown_projection().unwrap().pair);
    let revision = session.markdown_projection().unwrap().revision;
    session.execute(Command::ToggleCase);
    assert_eq!(session.markdown_mode(), PresentationMode::Rendered);
    settle(&mut session);
    assert!(session.markdown_projection().unwrap().revision > revision);
    assert!(Arc::ptr_eq(&pair.upgrade().unwrap(), &session.markdown_projection().unwrap().pair));
    session.open_file(1);
    assert!(session.markdown_projection().is_none());
    settle(&mut session);
    assert_eq!(session.markdown_mode(), PresentationMode::Source);
    assert!(pair.upgrade().is_none());
    session.open_file(2);
    settle(&mut session);
    assert_eq!(session.markdown_mode(), PresentationMode::Rendered);
    let pair = Arc::downgrade(&session.markdown_projection().unwrap().pair);
    drop(session);
    assert!(pair.upgrade().is_none());
}

#[test]
fn markdown_state_session_presentation_availability_matches_pending_dispatch() {
    use crate::{changeset::{ChangeSet, Spec}, commands::Command, session::Session, view::ScrollMode};
    let dir = tempfile::tempdir().unwrap();
    for extension in ["md", "txt"] {
        let left = dir.path().join(format!("left.{extension}"));
        let right = dir.path().join(format!("right.{extension}"));
        std::fs::write(&left, "# old\n").unwrap();
        std::fs::write(&right, "# new\n").unwrap();
        let spec = Spec::Plain { left, right };
        let set = Arc::new(ChangeSet::load(&spec, dir.path()).unwrap());
        let mut session = Session::from_set(set, spec, lenz_engine::Options::default(), ScrollMode::Shared);
        let rendered = Command::SetPresentation(PresentationMode::Rendered);
        let source = Command::SetPresentation(PresentationMode::Source);
        let settle = |session: &mut Session| {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while session.busy() { session.poll(); assert!(std::time::Instant::now() < until); std::thread::sleep(std::time::Duration::from_millis(1)); }
        };
        assert!(session.document().is_none());
        let unavailable = session.disabled(rendered).unwrap();
        session.execute(rendered);
        let no_doc_notice = session.notice().map(str::to_owned);
        let no_doc_unavailable = unavailable;
        assert!(!session.markdown_loading());
        assert_eq!(session.disabled(source), None);
        session.execute(source);
        assert_eq!(session.markdown_mode(), PresentationMode::Source);
        settle(&mut session);
        if extension == "txt" {
            let unavailable = session.disabled(rendered).unwrap();
            session.execute(rendered);
            assert_eq!(session.notice(), Some(unavailable));
            assert!(!session.markdown_loading());
            assert_eq!(session.markdown_mode(), PresentationMode::Source);
            continue;
        }
        session.execute(rendered);
        settle(&mut session);
        let ready = session.markdown_projection().unwrap().clone();
        session.execute(Command::ToggleCase);
        assert!(session.loading(), "real recompare is pending before polling its result");
        assert_eq!(session.disabled(Command::ToggleCase), Some("comparison loading"));
        let options = session.options();
        session.execute(Command::ToggleCase);
        assert_eq!(session.options(), options, "other loading guards remain unchanged");
        assert_eq!(session.disabled(source), None);
        session.execute(source);
        assert_eq!(session.markdown_mode(), PresentationMode::Source);
        assert_eq!(session.disabled(rendered), None);
        session.execute(rendered);
        assert_eq!(session.notice(), None);
        assert_eq!(session.markdown_mode(), PresentationMode::Rendered);
        assert!(Arc::ptr_eq(&ready, session.markdown_projection().unwrap()));
        settle(&mut session);
        assert_eq!(session.markdown_mode(), PresentationMode::Rendered);
        assert_eq!(no_doc_notice.as_deref(), Some(no_doc_unavailable));
    }
}

fn action_projection(source: &str) -> Projection {
    let doc = TextDocument::new(source, source, Path::new("a.md"), Path::new("b.md"));
    project(
        Arc::new(
            parse_pair(
                [doc.text_arc(Side::Left), doc.text_arc(Side::Right)],
                Limits::DEFAULT,
                &|| true,
            )
            .unwrap(),
        ),
        source_diff(&doc, &doc.view(&[], 20.0), 1),
        &|| true,
    )
    .unwrap()
}

#[test]
fn markdown_copy_keeps_visible_text_and_original_markup_distinct() {
    let p = action_projection("**bold** &amp; 漢字\r\n");
    let text = &p.pair.sides[1].text;
    let start = text.find("bold").unwrap();
    let selection = RenderSelection {
        side: Side::Right,
        anchor: start + 4,
        head: start,
    };
    assert_eq!(p.copy_visible(selection).unwrap(), "bold");
    assert_eq!(p.copy_source(selection).unwrap(), "**bold** &amp; 漢字\r\n");
    let at = text.find('&').unwrap();
    let mapped = p
        .source_selection(RenderSelection {
            side: Side::Right,
            anchor: at,
            head: at + 1,
        })
        .unwrap();
    assert_eq!((mapped.anchor.col, mapped.head.col), (9, 14));
    let at = text.find('漢').unwrap();
    assert_eq!(
        p.copy_visible(RenderSelection {
            side: Side::Right,
            anchor: at + 1,
            head: at + 3
        }),
        Err(MarkdownError::InvalidMap)
    );
    assert_eq!(
        p.copy_visible(RenderSelection {
            side: Side::Right,
            anchor: 0,
            head: usize::MAX
        }),
        Err(MarkdownError::InvalidMap)
    );
}

#[test]
fn markdown_copy_synthetic_and_overlapping_image_blocks_are_safe() {
    let source = "before ![alt](image.png) after\n";
    let p = action_projection(source);
    let doc = &p.pair.sides[1];
    assert_eq!(
        p.copy_source(RenderSelection {
            side: Side::Right,
            anchor: 0,
            head: doc.text.len()
        })
        .unwrap(),
        source
    );
    let segment = doc
        .segments
        .iter()
        .find(|s| matches!(s.origin, Origin::Synthetic) && !s.text.is_empty())
        .unwrap();
    let selection = RenderSelection {
        side: Side::Right,
        anchor: segment.text.start,
        head: segment.text.end,
    };
    assert!(p.source_selection(selection).is_none());
    assert_eq!(p.copy_source(selection).unwrap(), "");
    assert_eq!(
        p.copy_visible(selection).unwrap(),
        &doc.text[segment.text.clone()]
    );
}

#[test]
fn markdown_copy_search_hidden_and_normalized_origins_keep_source_locations() {
    let source = "**bold** &amp;\r\n\r\n[ref]: https://hidden.example\r\n";
    let p = action_projection(source);
    let doc = &p.pair.sides[1];
    let at = doc.text.find('&').unwrap();
    assert_eq!(p.find_source(Side::Right, 9..14), vec![at..at + 1]);
    let at = source.find("hidden").unwrap();
    let hidden = p.find_source(Side::Right, at..at + 6);
    assert!(!hidden.is_empty());
    assert!(hidden.iter().all(|r| r.is_empty()));
    assert!(p.find_source(Side::Right, 0..usize::MAX).is_empty());
}

#[test]
fn markdown_state_checked_selection_and_show_source_preserve_source_view() {
    use crate::selection::{Pos, Selection};
    let doc = Arc::new(TextDocument::new(
        "# one\r\n\r\n**two**\r\n",
        "# one\r\n\r\n**two**\r\n",
        Path::new("a.md"),
        Path::new("b.md"),
    ));
    let mut owner =
        crate::comparison_session::ComparisonSession::new(doc.view(&[], 1.), Default::default());
    owner.markdown_command(
        MarkdownCommand::SetMode(PresentationMode::Rendered),
        Some(doc.clone()),
    );
    assert_eq!(
        owner.markdown_edit_disabled(),
        Some("Switch to Source to edit")
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while owner.markdown_loading() {
        owner.markdown_poll();
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    let at = owner.markdown_projection().unwrap().pair.sides[1]
        .text
        .find("two")
        .unwrap();
    let selection = RenderSelection {
        side: Side::Right,
        anchor: at,
        head: at + 3,
    };
    owner.markdown_command(MarkdownCommand::Select(selection), Some(doc.clone()));
    assert_eq!(owner.markdown_selection(), Some(selection));
    owner.markdown_command(
        MarkdownCommand::Select(RenderSelection {
            head: usize::MAX,
            ..selection
        }),
        Some(doc.clone()),
    );
    assert_eq!(owner.markdown_selection(), None);
    owner.markdown_command(MarkdownCommand::Select(selection), Some(doc.clone()));
    owner.markdown_command(MarkdownCommand::ClearSelection, Some(doc.clone()));
    assert_eq!(owner.markdown_selection(), None);
    let original = Selection::caret(Side::Right, Pos::new(0, 1));
    assert_eq!(
        owner.move_source_caret(&doc, original, crate::selection::CaretCommand::Right, true),
        original
    );
    owner.markdown_command(
        MarkdownCommand::ShowSource(SourceAnchor {
            side: Side::Right,
            byte: 12,
        }),
        Some(doc.clone()),
    );
    assert_eq!(owner.markdown_mode(), PresentationMode::Source);
    assert_eq!(owner.markdown_edit_disabled(), None);
    assert_eq!(
        owner.markdown_projection().unwrap().pair.sides[1].source_pos(12),
        Some(Pos::new(2, 3))
    );
    assert_eq!(owner.view().anchor_line_on(Side::Right), 2.);
    assert_ne!(
        owner.move_source_caret(&doc, original, crate::selection::CaretCommand::Right, true),
        original
    );
}

#[test]
fn markdown_navigation_keeps_source_hunk_ids_hidden_find_order_and_line_anchors() {
    use crate::{commands::Command, selection::Pos};
    let left = "# A\r\n\r\n**same**\r\n\r\n[ref]: hidden-one\r\n";
    let right = "# B\r\n\r\n_same_\r\n\r\n[ref]: hidden-two\r\n";
    let doc = Arc::new(TextDocument::new(
        left,
        right,
        Path::new("a.md"),
        Path::new("b.md"),
    ));
    let mut owner = crate::comparison_session::ComparisonSession::new(
        doc.view(&doc.line_diff(&Default::default()), 2.),
        Default::default(),
    );
    owner.markdown_command(
        MarkdownCommand::SetMode(PresentationMode::Rendered),
        Some(doc.clone()),
    );
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while owner.markdown_loading() {
        owner.markdown_poll();
        assert!(std::time::Instant::now() < until);
        std::thread::yield_now();
    }
    owner.command(Command::NextChange, &[], false, false);
    let selected = owner.view().current_change().unwrap();
    assert_eq!(
        owner.markdown_anchor(Side::Right),
        owner
            .markdown_projection()
            .unwrap()
            .location(selected, Side::Right)
    );
    owner.markdown_command(
        MarkdownCommand::SetMode(PresentationMode::Source),
        Some(doc.clone()),
    );
    owner.markdown_command(
        MarkdownCommand::SetMode(PresentationMode::Rendered),
        Some(doc.clone()),
    );
    assert_eq!(owner.view().current_change(), Some(selected));
    assert_eq!(
        owner.search(&doc, "hidden".into(), Side::Right, Pos::new(0, 0)),
        Some((4, 7..13))
    );
    let (anchor, range, mapped) = owner.markdown_find_hit().unwrap();
    assert_eq!(&right[range], "hidden");
    assert!(mapped.iter().all(|r| r.is_empty()));
    assert_eq!(owner.markdown_anchor(Side::Right), Some(anchor));
    owner.go_to_line(Side::Right, 3);
    assert_eq!(
        owner.markdown_anchor(Side::Right),
        Some(SourceAnchor {
            side: Side::Right,
            byte: 7
        })
    );
    owner.markdown_command(MarkdownCommand::ShowSource(anchor), Some(doc));
    assert_eq!(owner.markdown_mode(), PresentationMode::Source);
    assert_eq!(owner.view().anchor_line_on(Side::Right), 4.);

    let doc = TextDocument::new("", "new\n", Path::new("a.md"), Path::new("b.md"));
    let p = project(
        Arc::new(
            parse_pair(
                [doc.text_arc(Side::Left), doc.text_arc(Side::Right)],
                Limits::DEFAULT,
                &|| true,
            )
            .unwrap(),
        ),
        source_diff(&doc, &doc.view(&doc.line_diff(&Default::default()), 2.), 1),
        &|| true,
    )
    .unwrap();
    assert_eq!(
        p.location(0, Side::Left),
        Some(SourceAnchor {
            side: Side::Left,
            byte: 0
        })
    );
    assert_eq!(
        p.copy_visible(RenderSelection {
            side: Side::Left,
            anchor: 0,
            head: 0
        })
        .unwrap(),
        ""
    );
}

#[test]
fn markdown_unsupported_math_nested_delimiters_stays_literal() {
    let source = "$**bold**$ and $$*math*$$\n";
    let doc = parse(source);
    assert!(doc.text.contains("$**bold**$"), "{}", doc.text);
    assert!(doc.text.contains("$$*math*$$"), "{}", doc.text);
    assert!(doc.blocks.iter().flat_map(|b| &b.runs).all(|r| !r.style.strong && !r.style.emphasis));
}

#[test]
fn markdown_inherited_preference_is_lazy_and_source_default_is_unallocated() {
    use crate::{comparison_session::ComparisonSession, view::View};
    let mut previous=ComparisonSession::new(View::new(0,0,&[],20.),Default::default());
    let mut next=ComparisonSession::new(View::new(0,0,&[],20.),Default::default());
    next.markdown_inherit_presentation(&previous);
    assert_eq!(next.markdown_mode(),PresentationMode::Source);
    assert!(!next.markdown_loading());assert!(next.markdown_projection().is_none());
    let doc=TextDocument::new("# Before\n","# After\n",Path::new("left.md"),Path::new("right.md"));
    previous.markdown_command_document(MarkdownCommand::SetMode(PresentationMode::Rendered),Some(&doc));
    next.markdown_inherit_presentation(&previous);
    assert_eq!(next.markdown_preferred_mode(),PresentationMode::Rendered);
    assert!(!next.markdown_loading());assert!(next.markdown_projection().is_none());
    assert_eq!(next.presentation_disabled(PresentationMode::Rendered,None),Some("Comparison loading"));
    next.markdown_refresh(Some(&doc));
    assert!(next.markdown_loading());
}

#[test]
fn markdown_inspection_maps_direct_utf8_and_rejects_normalized_synthetic_stale_positions() {
    use crate::{comparison_session::ComparisonSession, selection::Pos, view::View};
    let source="# 漢字\n\nentity &amp;\n\n- item\n";
    let doc=TextDocument::new(source,source,Path::new("left.md"),Path::new("right.md"));
    let mut session=ComparisonSession::new(View::new(5,5,&[],20.),Default::default());
    assert_eq!(session.markdown_inspection_position(Side::Right),Ok(None));
    session.markdown_command_document(MarkdownCommand::SetMode(PresentationMode::Rendered),Some(&doc));
    let until=std::time::Instant::now()+std::time::Duration::from_secs(5);
    while session.markdown_loading(){session.markdown_poll();assert!(std::time::Instant::now()<until);std::thread::yield_now();}
    let p=session.markdown_projection().unwrap().clone();let text=&p.pair.sides[1].text;
    for at in [text.find('漢').unwrap(), text.find('字').unwrap()] {
        session.markdown_command_document(MarkdownCommand::Select(RenderSelection{side:Side::Right,anchor:at,head:at}),Some(&doc));
        assert_eq!(session.markdown_inspection_position(Side::Right),Ok(Some(Pos::new(0,2+at))));
    }
    for at in [text.find('&').unwrap(),text.find('•').unwrap()] {
        session.markdown_command_document(MarkdownCommand::Select(RenderSelection{side:Side::Right,anchor:at,head:at}),Some(&doc));
        assert_eq!(session.markdown_inspection_position(Side::Right),Err("Show in source to inspect this position"));
    }
    session.markdown_command_document(MarkdownCommand::SetAnchor(SourceAnchor{side:Side::Left,byte:2}),Some(&doc));
    assert_eq!(session.markdown_inspection_position(Side::Left),Ok(Some(Pos::new(0,2))));
    session.markdown_invalidate();assert_eq!(session.markdown_inspection_position(Side::Right),Ok(None));
}

#[test]
fn markdown_final_linked_image_has_no_empty_leaf_and_empty_copy_is_empty() {
    let doc=parse("[![alt](image)](target)");
    assert_eq!(doc.blocks.len(),1);
    assert!(!doc.text.starts_with('\n'));
    assert_eq!(doc.blocks[0].source,0.."[![alt](image)](target)".len());
    assert!(matches!(&doc.blocks[0].kind,BlockKind::Image{destination,..} if destination=="image"));
    assert_eq!(&*doc.links[0].destination,"target");
    let (image,_)=projection_fixture("[![alt](image)](target)","[![alt](image)](target)",Default::default());
    assert_eq!(image.copy_source(RenderSelection{side:Side::Right,anchor:0,head:image.pair.sides[1].text.len()}).unwrap(),"[![alt](image)](target)");
    let (p,_)=projection_fixture("words", "words",Default::default());
    assert_eq!(p.copy_source(RenderSelection{side:Side::Right,anchor:2,head:2}).unwrap(),"");
}

#[test]
fn markdown_final_source_points_have_forward_affinity_and_validate_unicode() {
    let source="α界 first\nsecond &amp; tail\n\n**last**";
    let (p,_)=projection_fixture(source,source,Default::default());
    let doc=&p.pair.sides[1];
    for needle in ["α", "界", "second", "tail", "last"] {
        let source_byte=source.find(needle).unwrap();
        assert_eq!(p.source_point(SourceAnchor{side:Side::Right,byte:source_byte}),doc.text.find(needle));
    }
    assert_eq!(p.source_point(SourceAnchor{side:Side::Right,byte:1}),None);
    assert_eq!(p.source_point(SourceAnchor{side:Side::Right,byte:source.len()+1}),None);
    assert_eq!(p.source_point(SourceAnchor{side:Side::Right,byte:source.len()}),Some(doc.text.len()));
    let entity=source.find("&amp;").unwrap();
    assert_eq!(p.source_point(SourceAnchor{side:Side::Right,byte:entity+2}),doc.text.find('&'));
    let last=source.find("last").unwrap();
    assert_eq!(p.source_point(SourceAnchor{side:Side::Right,byte:last-1}),doc.text.find("last"));
}

fn styled(doc: &MarkdownDocument, pick: impl Fn(&InlineStyle) -> bool) -> Vec<String> {
    doc.blocks
        .iter()
        .flat_map(|b| b.runs.iter())
        .filter(|r| pick(&r.style))
        .map(|r| doc.text[r.text.clone()].to_string())
        .collect()
}

#[test]
fn highlights_hide_their_markers_and_keep_source() {
    let doc = parse("Read ==the rules== now.\n");
    assert_eq!(styled(&doc, |s| s.highlight), ["the rules"]);
    assert!(doc.text.contains("Read the rules now."));
    assert!(
        doc.segments
            .iter()
            .filter(|s| s.text.is_empty())
            .any(|s| matches!(&s.origin, Origin::Normalized(r) if &doc.source[r.clone()] == "=="))
    );
}

#[test]
fn equals_signs_that_are_not_highlights_stay_literal() {
    for source in ["if a == b then\n", "x ==y\n", "`==code==`\n", "==unclosed\n", "a==b==c\n"] {
        let doc = parse(source);
        assert!(styled(&doc, |s| s.highlight).is_empty(), "{source:?}");
    }
}

#[test]
fn tags_are_words_after_a_hash() {
    let doc = parse("Filed under #accounting and #team/ops, not C# or #1 or `#code` or [#link](x.md).\n");
    assert_eq!(styled(&doc, |s| s.tag), ["#accounting", "#team/ops"]);
}

#[test]
fn links_with_a_scheme_are_external() {
    let doc = parse("[site](https://example.com) and [doc](notes/a.md)\n");
    assert_eq!(styled(&doc, |s| s.external), ["site"]);
    assert_eq!(styled(&doc, |s| s.link), ["site", "doc"]);
}

#[test]
fn a_quote_starting_with_a_type_is_a_callout_with_a_title() {
    let doc = parse("> [!warning]- Mind the size\n> Requests over 1 MiB are refused.\n\nAfter.\n");
    let callouts: Vec<_> = doc.blocks.iter().filter_map(|b| b.callout).collect();
    assert!(!callouts.is_empty());
    assert!(callouts.iter().all(|c| c.kind == CalloutKind::Warning && c.folded == Some(true)));
    assert_eq!(styled(&doc, |s| s.callout_title), ["Mind the size"]);
    assert!(doc.text.contains("Mind the size\nRequests over 1 MiB"));
    assert!(!doc.text.contains("[!warning]"));
    assert!(doc.blocks.last().unwrap().callout.is_none(), "after the quote, no callout");
}

#[test]
fn callout_kinds_have_aliases_and_a_note_fallback() {
    assert_eq!(CalloutKind::parse("TLDR"), CalloutKind::Abstract);
    assert_eq!(CalloutKind::parse("error"), CalloutKind::Danger);
    assert_eq!(CalloutKind::parse("whatever"), CalloutKind::Note);
    let doc = parse("> [!tip]\n> body\n");
    assert!(styled(&doc, |s| s.callout_title).is_empty(), "no title: the adapter shows the kind's label");
    assert_eq!(doc.blocks[0].callout.map(|c| c.folded), Some(None));
    assert!(doc.text.starts_with("body"));
}

#[test]
fn quotes_that_only_look_like_callouts_stay_quotes() {
    for source in ["> [!not a callout\n> text\n", "> text [!note]\n", "> `[!note]` code\n"] {
        let doc = parse(source);
        assert!(doc.blocks.iter().all(|b| b.callout.is_none()), "{source:?}");
    }
    let nested = parse("> > [!note] nested\n");
    assert!(nested.blocks.iter().any(|b| b.quote_depth == 2 && b.callout.is_some()), "a nested quote may be a callout");
    let outer = parse("> outer\n>\n> > [!note] inner\n");
    assert!(outer.blocks[0].callout.is_none(), "the outer quote is not the callout");
}

#[test]
fn front_matter_becomes_properties_rows() {
    let doc = parse("---\nstatus: draft\ntags:\n  - service\n  - accounting\n---\n# Ledger\n");
    let block = &doc.blocks[0];
    assert!(matches!(block.kind, BlockKind::Properties));
    let rows = properties(&doc.text[block.text.clone()]).unwrap();
    assert_eq!(
        rows,
        [
            ("status".to_string(), vec!["draft".to_string()]),
            ("tags".to_string(), vec!["service".to_string(), "accounting".to_string()])
        ]
    );
    assert_eq!(properties("tags: [a, \"b\"]\n").unwrap()[0].1, ["a", "b"]);
    assert!(matches!(doc.blocks[1].kind, BlockKind::Heading(1)));
    assert!(styled(&doc, |s| s.tag).is_empty(), "no inline extras in front matter");
}

#[test]
fn only_simple_front_matter_at_the_start_is_properties() {
    assert!(properties("server:\n  host: a\n  port: 1\n").is_none(), "nested maps stay literal");
    assert!(properties("text: |\n  multi\n  line\n").is_none(), "block strings stay literal");
    let later = parse("# Title\n\n---\nstatus: draft\n---\n");
    assert!(!later.blocks.iter().any(|b| matches!(b.kind, BlockKind::Properties)), "a rule later in the file is not front matter");
    for source in ["Intro\n\n---\n\nPart two\n\n---\n\nEnd\n", "Intro\n\n---\nkey: value\n---\n\nEnd\n"] {
        let rules = parse(source);
        assert!(rules.blocks.iter().all(|b| !matches!(b.kind, BlockKind::Literal | BlockKind::Properties)), "{source:?}");
        assert!(rules.blocks.iter().any(|b| matches!(b.kind, BlockKind::Rule)), "{source:?}");
    }
}

#[test]
fn wiki_links_show_their_alias_and_hide_the_brackets() {
    let doc = parse("See [[Accounts]] and [[docs/rules|the rules]] or [[Ledger#Running]].\n");
    assert_eq!(styled(&doc, |s| s.wikilink), ["Accounts", "the rules", "Ledger#Running"]);
    assert!(doc.text.contains("See Accounts and the rules or Ledger#Running."));
    assert!(!doc.text.contains("[["));
    assert!(styled(&doc, |s| s.external).is_empty());
    let code = parse("`[[not a link]]`\n");
    assert!(styled(&code, |s| s.wikilink).is_empty());
}

#[test]
fn footnotes_number_references_and_style_definitions_in_place() {
    let doc = parse("Text[^rules] and more[^b].\n\n[^b]: Second.\n[^rules]: First.\n");
    assert_eq!(styled(&doc, |s| s.footnote), ["1", "2"]);
    let defs: Vec<_> = doc
        .blocks
        .iter()
        .filter_map(|b| match &b.kind {
            BlockKind::Footnote { label } => Some((label.as_str(), &doc.text[b.text.clone()])),
            _ => None,
        })
        .collect();
    assert_eq!(defs, [("2", "Second.\n"), ("1", "First.\n")], "source order, labelled with the reference number");
    assert!(doc.text.starts_with("Text1 and more2."));
    let unknown = parse("No definition[^x].\n");
    assert!(unknown.text.contains("[^x]"), "an undefined reference stays literal");
}

#[test]
fn local_images_stay_inside_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join("docs/img")).unwrap();
    std::fs::write(root.join("docs/img/a.png"), b"png").unwrap();
    std::fs::write(root.join("docs/img/a.txt"), b"txt").unwrap();
    std::fs::write(dir.path().join("secret.png"), b"png").unwrap();
    std::os::unix::fs::symlink(dir.path().join("secret.png"), root.join("docs/out.png")).unwrap();
    let md = root.join("docs/README.md");
    assert_eq!(
        local_image(&root, &md, "img/a.png"),
        Some(root.join("docs/img/a.png").canonicalize().unwrap())
    );
    assert_eq!(
        local_image(&root, &md, "img/a.png#x"),
        Some(root.join("docs/img/a.png").canonicalize().unwrap())
    );
    for refused in [
        "../../secret.png",
        "/etc/hosts.png",
        "https://x/a.png",
        "data:image/png;base64,AA",
        "out.png",
        "img/missing.png",
        "img/a.txt",
        "",
    ] {
        assert_eq!(local_image(&root, &md, refused), None, "{refused}");
    }
}

#[test]
fn front_matter_lists_may_be_unindented() {
    assert_eq!(
        properties("tags:\n- service\n- accounting\nstatus: draft\n").unwrap(),
        [
            ("tags".to_string(), vec!["service".to_string(), "accounting".to_string()]),
            ("status".to_string(), vec!["draft".to_string()])
        ]
    );
    assert!(properties("- orphan\n").is_none(), "an item needs a key above it");
}

fn inline(left: &str, right: &str) -> (Projection, Vec<String>) {
    let (projection, _) = projection_fixture(left, right, lenz_engine::Options::default());
    let pair = &projection.pair;
    let shown = projection
        .inline
        .iter()
        .map(|b| {
            b.spans
                .iter()
                .map(|s| {
                    let t = &pair.sides[s.side as usize].text[s.text.clone()];
                    match s.change {
                        Some(Kind::Inserted) => format!("+[{t}]"),
                        Some(Kind::Deleted) => format!("-[{t}]"),
                        _ => t.to_string(),
                    }
                })
                .collect::<String>()
        })
        .collect();
    (projection, shown)
}

#[test]
fn inline_interleaves_removed_and_inserted_words_in_place() {
    let (p, shown) = inline("# T\n\nA small service.\n", "# T\n\nA small, fast service.\n");
    assert_eq!(shown[0], "T");
    assert!(shown[1].starts_with("A small") && shown[1].contains("+[") && shown[1].ends_with("service."), "{shown:?}");
    assert_eq!(p.inline[1].change, Some(Kind::Modified));
    assert_eq!(p.inline[0].change, None);
}

#[test]
fn inline_marks_words_whose_styling_changed() {
    let (p, shown) = inline("Keep *this* word.\n", "Keep **this** word.\n");
    assert_eq!(shown[0], "Keep -[this]+[this] word.");
    assert_eq!(p.inline[0].change, Some(Kind::Modified));
    // a link target alone keeps the block's marker only
    let (p, shown) = inline("[a](x)\n", "[a](y)\n");
    assert_eq!((shown[0].as_str(), p.inline[0].change), ("a", None));
}

#[test]
fn inline_shows_added_and_removed_blocks_whole() {
    let (p, shown) = inline("One.\n\nGone paragraph.\n", "One.\n\nNew paragraph here.\n\nAnd more.\n");
    assert!(p.inline.iter().any(|b| b.change == Some(Kind::Inserted) && b.side == Side::Right));
    assert!(shown.iter().all(|s| !s.is_empty()));
    let (p, _) = inline("", "# Added file\n\ntext\n");
    assert!(p.inline.iter().all(|b| b.change == Some(Kind::Inserted)));
    let (p, _) = inline("# Deleted file\n\ntext\n", "");
    assert!(p.inline.iter().all(|b| b.change == Some(Kind::Deleted) && b.side == Side::Left));
}

#[test]
fn a_rewritten_block_shows_old_then_new_instead_of_word_salad() {
    let (p, shown) = inline("The quick brown fox jumps.\n", "Completely different words entirely here.\n");
    assert_eq!(p.inline.len(), 2, "{shown:?}");
    assert_eq!((p.inline[0].side, p.inline[0].change), (Side::Left, Some(Kind::Deleted)));
    assert_eq!((p.inline[1].side, p.inline[1].change), (Side::Right, Some(Kind::Inserted)));
}

#[test]
fn table_rows_and_code_change_as_whole_blocks() {
    let (p, _) = inline("| a | b |\n| - | - |\n| 1 | 2 |\n", "| a | b |\n| - | - |\n| 1 | 3 |\n");
    assert!(p.inline.iter().any(|b| b.change == Some(Kind::Deleted) && b.spans.len() == 1));
    assert!(p.inline.iter().all(|b| b.change != Some(Kind::Modified)), "table rows are never mixed word by word");
}

#[test]
fn inline_spans_stay_inside_their_blocks_and_follow_source_order() {
    let (p, _) = inline("# A\n\nx y z\n\n- one\n- two\n", "# A\n\nx q z\n\n- one\n- two\n- three\n");
    for b in &p.inline {
        assert!(b.block < p.pair.sides[b.side as usize].blocks.len());
        for s in &b.spans {
            let blocks = &p.pair.sides[s.side as usize].blocks;
            assert!(blocks.iter().any(|d| d.text.start <= s.text.start && s.text.end <= d.text.end), "a span stays inside one block");
        }
    }
    assert!(p.inline.windows(2).all(|w| w[0].group <= w[1].group));
}

#[test]
fn a_markup_only_change_is_not_struck() {
    let (p, shown) = inline("Some *text* here.\n", "Some _text_ here.\n");
    assert_eq!(shown, ["Some text here."]);
    assert!(p.inline.iter().all(|b| b.spans.iter().all(|s| s.change.is_none())));
}

#[test]
fn many_footnote_references_in_one_paragraph_do_not_overflow_the_stack() {
    let source = format!("{}\n\n[^a]: note\n", "[^a]".repeat(30_000));
    assert!(parse_pair([source.into(), "".into()], Limits::DEFAULT, &|| true).is_ok());
}

#[test]
fn side_by_side_marks_an_added_section_by_its_bar_alone() {
    let old = "# Title\n\nThe service keeps accounts in balance.\n\n## End\n\nBye.\n";
    let new = "# Title\n\nThe service keeps every account in balance.\n\n## Importing\n\nOpening balances come from a CSV file with one row per account.\n\n- Dates use ISO 8601\n- Amounts are in cents\n\n## End\n\nBye.\n";
    let (projection, diff) = projection_fixture(old, new, lenz_engine::Options::default());
    assert_eq!(diff.hunks.len(), 1, "the edit and the new section are one source change");
    let doc = &projection.pair.sides[1];
    let block = |text: &str| doc.blocks.iter().position(|b| doc.text[b.text.clone()].contains(text)).unwrap();
    assert_eq!(projection.whole_change(Side::Right, block("every account")), None, "the edited paragraph keeps its word outlines");
    for added in ["Importing", "CSV file", "ISO 8601", "in cents"] {
        assert_eq!(projection.whole_change(Side::Right, block(added)), Some(Kind::Inserted), "{added}: added whole");
    }
    assert!(projection.whole[0].is_empty(), "nothing was removed");
}

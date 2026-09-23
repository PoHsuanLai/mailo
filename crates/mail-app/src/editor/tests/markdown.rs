//! Markdown typed as you go: each shortcut, and what it leaves behind.

use super::*;

#[test]
fn markdown_shortcut_table() {
    struct Case {
        name: &'static str,
        typed: &'static str,
        kind: &'static str,
        text: &'static str,
        mark: Option<Mark>,
    }
    let cases = [
        Case {
            name: "h1",
            typed: "# ",
            kind: "h1",
            text: "",
            mark: None,
        },
        Case {
            name: "h2",
            typed: "## ",
            kind: "h2",
            text: "",
            mark: None,
        },
        Case {
            name: "h3",
            typed: "### ",
            kind: "h3",
            text: "",
            mark: None,
        },
        Case {
            name: "bullet dash",
            typed: "- ",
            kind: "ul",
            text: "",
            mark: None,
        },
        Case {
            name: "bullet star",
            typed: "* ",
            kind: "ul",
            text: "",
            mark: None,
        },
        Case {
            name: "numbered",
            typed: "1. ",
            kind: "ol",
            text: "",
            mark: None,
        },
        Case {
            name: "numbered paren",
            typed: "1) ",
            kind: "ol",
            text: "",
            mark: None,
        },
        Case {
            name: "todo empty",
            typed: "[] ",
            kind: "todo",
            text: "",
            mark: None,
        },
        Case {
            name: "todo spaced",
            typed: "[ ] ",
            kind: "todo",
            text: "",
            mark: None,
        },
        Case {
            name: "quote",
            typed: "> ",
            kind: "quote",
            text: "",
            mark: None,
        },
        Case {
            name: "code fence",
            typed: "```",
            kind: "code",
            text: "",
            mark: None,
        },
        Case {
            name: "divider",
            typed: "---",
            kind: "hr",
            text: "#",
            mark: None,
        },
        Case {
            name: "bold",
            typed: "say **hi**",
            kind: "p",
            text: "say hi",
            mark: Some(Mark::Bold),
        },
        Case {
            name: "italic star",
            typed: "say *hi*",
            kind: "p",
            text: "say hi",
            mark: Some(Mark::Italic),
        },
        Case {
            name: "italic underscore",
            typed: "say _hi_",
            kind: "p",
            text: "say hi",
            mark: Some(Mark::Italic),
        },
        Case {
            name: "code span",
            typed: "say `hi`",
            kind: "p",
            text: "say hi",
            mark: Some(Mark::Code),
        },
        Case {
            name: "strike",
            typed: "say ~~hi~~",
            kind: "p",
            text: "say hi",
            mark: Some(Mark::Strike),
        },
    ];
    for case in cases {
        let mut doc = Doc::from_text(case.typed);
        let at = Pos::new(0, grapheme_len(case.typed));
        let shortcut = shortcuts(&doc, at).unwrap_or_else(|| panic!("{}: no shortcut", case.name));
        let before = doc.clone();
        let inverses = apply_all(&mut doc, shortcut.ops.clone()).unwrap();
        assert_eq!(kinds(&doc), case.kind, "{}", case.name);
        assert_eq!(body(&doc), case.text, "{}", case.name);
        if let Some(mark) = case.mark {
            let Node::Para { runs, .. } = &doc.nodes[0] else {
                panic!("{}", case.name);
            };
            let marked = runs
                .iter()
                .find(|run| run.marks.has(mark))
                .unwrap_or_else(|| panic!("{}: mark missing", case.name));
            assert_eq!(marked.text, "hi", "{}", case.name);
        }
        // The shortcut is one undo step: its inverses, reversed, restore the markers.
        let mut undo = inverses;
        undo.reverse();
        apply_ops(&mut doc, undo);
        assert_eq!(doc, before, "{}: undo", case.name);
    }

    let doc = Doc::from_text("already # ");
    assert!(shortcuts(&doc, Pos::new(0, grapheme_len("already # "))).is_none());
    let doc = Doc::from_text("**hi");
    assert!(shortcuts(&doc, Pos::new(0, grapheme_len("**hi"))).is_none());
}

#[test]
fn shortcuts_that_do_not_fire() {
    let cases: &[(&str, Node, usize)] = &[
        (
            "marker after text",
            plain(ParaKind::Paragraph, "already # "),
            10,
        ),
        ("unclosed bold", plain(ParaKind::Paragraph, "**hi"), 4),
        ("empty bold", plain(ParaKind::Paragraph, "****"), 4),
        (
            "italic opening on a space",
            plain(ParaKind::Paragraph, "a * b*"),
            6,
        ),
        (
            "underscore inside a word",
            plain(ParaKind::Paragraph, "snake_case_"),
            11,
        ),
        (
            "line marker in a list item",
            plain(ParaKind::Bullet, "# "),
            2,
        ),
        ("anything in code", plain(ParaKind::Code, "**hi**"), 6),
        (
            "divider with text after",
            plain(ParaKind::Paragraph, "---x"),
            3,
        ),
    ];
    for (name, node, at) in cases {
        let doc = doc_of(vec![node.clone()]);
        assert!(shortcuts(&doc, Pos::new(0, *at)).is_none(), "{name}");
    }
}

#[test]
fn a_marker_typed_before_existing_text_keeps_the_text() {
    let doc = Doc::from_text("# Title");
    let shortcut = shortcuts(&doc, Pos::new(0, 2)).expect("a heading marker at the start");
    let mut doc = doc;
    apply_ops(&mut doc, shortcut.ops);
    assert_eq!((kinds(&doc).as_str(), body(&doc).as_str()), ("h1", "Title"));
}

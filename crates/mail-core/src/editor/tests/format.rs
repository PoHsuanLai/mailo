//! Size, colour, alignment, indent, nested lists and quote levels: the edits, the HTML and the
//! plain text each is sent as, and the HTML read back as the same document.

use super::*;

fn sized(text: &str, size: Option<TextSize>, color: Option<TextColor>) -> Run {
    let mut marks = Marks::new();
    marks.size = size;
    marks.color = color;
    Run::new(text, marks)
}

fn aligned(kind: ParaKind, text: &str, align: Align, depth: u8) -> Node {
    Node::laid(
        kind,
        vec![Run::new(text, Marks::new())],
        Layout { align, depth },
    )
}

fn whole(doc: &Doc) -> Range {
    let last = doc.nodes.len() - 1;
    Range {
        start: Pos::new(0, 0),
        end: Pos::new(last, node_len(&doc.nodes[last])),
    }
}

fn collapsed(node: usize, offset: usize) -> Range {
    let at = Pos::new(node, offset);
    Range { start: at, end: at }
}

/// Each edit, applied, gives the document asked for, and its inverse gives the original back.
#[test]
fn every_format_is_an_edit_with_an_inverse() {
    // (what, before, the edits, what each paragraph is after as `levels`, and as text runs)
    type Case = (&'static str, Doc, fn(&Doc) -> Vec<Op>, &'static str);
    let cases: Vec<Case> = vec![
        (
            "indent a paragraph and a heading",
            doc_of(vec![
                plain(ParaKind::Paragraph, "a"),
                plain(ParaKind::Heading(Level::Two), "b"),
            ]),
            |doc| indent(doc, whole(doc), Nest::In),
            "p1|h21",
        ),
        (
            "outdent stops at the margin",
            doc_of(vec![
                deep(ParaKind::Paragraph, "a", 1),
                plain(ParaKind::Paragraph, "b"),
            ]),
            |doc| indent(doc, whole(doc), Nest::Out),
            "p0|p0",
        ),
        (
            "indent leaves quotes and code alone",
            doc_of(vec![
                plain(ParaKind::Quote, "a"),
                plain(ParaKind::Code, "b"),
            ]),
            |doc| indent(doc, whole(doc), Nest::In),
            "quote0|code0",
        ),
        (
            "indent stops at the deepest",
            doc_of(vec![deep(ParaKind::Paragraph, "a", Layout::MAX_DEPTH)]),
            |doc| indent(doc, whole(doc), Nest::In),
            "p6",
        ),
        (
            "quote a paragraph, then deeper",
            doc_of(vec![
                plain(ParaKind::Paragraph, "a"),
                plain(ParaKind::Quote, "b"),
            ]),
            |doc| quote(doc, whole(doc), Nest::In),
            "quote0|quote1",
        ),
        (
            "centre two paragraphs",
            doc_of(vec![
                plain(ParaKind::Paragraph, "a"),
                plain(ParaKind::Bullet, "b"),
            ]),
            |doc| {
                vec![Op::SetAlign {
                    range: whole(doc),
                    align: Align::Center,
                }]
            },
            "p0|ul0",
        ),
        (
            "a depth is held at the deepest",
            doc_of(vec![plain(ParaKind::Bullet, "a")]),
            |_| {
                vec![Op::SetDepth {
                    range: collapsed(0, 0),
                    depth: 40,
                }]
            },
            "ul6",
        ),
        (
            "size part of a run",
            doc_of(vec![plain(ParaKind::Paragraph, "abc")]),
            |_| {
                vec![Op::SetSize {
                    range: Range {
                        start: Pos::new(0, 1),
                        end: Pos::new(0, 2),
                    },
                    size: Some(TextSize::Huge),
                }]
            },
            "p0",
        ),
        (
            "colour across two paragraphs, then clear one",
            doc_of(vec![
                plain(ParaKind::Paragraph, "ab"),
                plain(ParaKind::Paragraph, "cd"),
            ]),
            |doc| {
                vec![
                    Op::SetColor {
                        range: whole(doc),
                        color: Some(TextColor::Blue),
                    },
                    Op::SetColor {
                        range: collapsed(0, 0),
                        color: None,
                    },
                    Op::SetColor {
                        range: Range {
                            start: Pos::new(1, 0),
                            end: Pos::new(1, 1),
                        },
                        color: None,
                    },
                ]
            },
            "p0|p0",
        ),
    ];
    for (name, before, edits, after) in cases {
        let mut doc = before.clone();
        let ops = edits(&doc);
        let inverses = apply_all(&mut doc, ops).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(levels(&doc), after, "{name}");
        for inverse in inverses.into_iter().rev() {
            apply(&mut doc, inverse).unwrap_or_else(|err| panic!("{name}: {err}"));
        }
        assert_eq!(doc, before, "{name}: the inverses did not restore it");
    }
}

#[test]
fn a_size_or_colour_paints_only_its_range() {
    let mut doc = doc_of(vec![plain(ParaKind::Paragraph, "abc")]);
    apply_ops(
        &mut doc,
        vec![
            Op::SetSize {
                range: Range {
                    start: Pos::new(0, 1),
                    end: Pos::new(0, 3),
                },
                size: Some(TextSize::Large),
            },
            Op::SetColor {
                range: Range {
                    start: Pos::new(0, 2),
                    end: Pos::new(0, 3),
                },
                color: Some(TextColor::Red),
            },
        ],
    );
    let Node::Para { runs, .. } = &doc.nodes[0] else {
        panic!("a paragraph");
    };
    assert_eq!(
        runs,
        &vec![
            sized("a", None, None),
            sized("b", Some(TextSize::Large), None),
            sized("c", Some(TextSize::Large), Some(TextColor::Red)),
        ]
    );
    assert_eq!(size_at(&doc, collapsed(0, 0)), None);
    assert_eq!(
        size_at(
            &doc,
            Range {
                start: Pos::new(0, 1),
                end: Pos::new(0, 3)
            }
        ),
        Some(TextSize::Large)
    );
    assert_eq!(color_at(&doc, collapsed(0, 3)), Some(TextColor::Red));
}

#[test]
fn a_size_steps_through_the_four() {
    // (from, which way, to)
    let cases = [
        (None, Nest::In, Some(TextSize::Large)),
        (Some(TextSize::Large), Nest::In, Some(TextSize::Huge)),
        (Some(TextSize::Huge), Nest::In, Some(TextSize::Huge)),
        (None, Nest::Out, Some(TextSize::Small)),
        (Some(TextSize::Small), Nest::Out, Some(TextSize::Small)),
        (Some(TextSize::Large), Nest::Out, None),
    ];
    for (from, nest, to) in cases {
        assert_eq!(step_size(from, nest), to, "{from:?} {nest:?}");
    }
}

#[test]
fn html_says_each_format_and_nothing_more() {
    // (what, the document, the HTML)
    let cases: Vec<(&str, Doc, &str)> = vec![
        (
            "sizes and colours, inside a link and around a mark",
            doc_of(vec![Node::para(
                ParaKind::Paragraph,
                vec![
                    sized("small ", Some(TextSize::Small), None),
                    Run::new("normal ", Marks::new()),
                    sized("big", Some(TextSize::Huge), Some(TextColor::Green)),
                    Run::new(" ", Marks::new()),
                    {
                        let mut marks = marks_of(&[Mark::Bold]);
                        marks.link = Some(url());
                        marks.color = Some(TextColor::Red);
                        Run::new("go", marks)
                    },
                ],
            )]),
            "<p><span style=\"font-size:small\">small </span>normal \
             <span style=\"font-size:x-large;color:#188038\">big</span> \
             <a href=\"https://example.com/a\"><span style=\"color:#d93025\">\
             <strong>go</strong></span></a></p>",
        ),
        (
            "alignment and indent on a paragraph and a heading",
            doc_of(vec![
                aligned(ParaKind::Paragraph, "a", Align::Center, 0),
                aligned(ParaKind::Paragraph, "b", Align::Left, 2),
                aligned(ParaKind::Heading(Level::One), "c", Align::Right, 1),
                aligned(ParaKind::Code, "d", Align::Center, 3),
            ]),
            "<p style=\"text-align:center\">a</p>\
             <p style=\"margin-left:80px\">b</p>\
             <h1 style=\"margin-left:40px;text-align:right\">c</h1>\
             <pre>d</pre>",
        ),
        (
            "a nested list, in and out, with a numbered list inside a bullet",
            doc_of(vec![
                deep(ParaKind::Bullet, "a", 0),
                deep(ParaKind::Numbered, "b", 1),
                deep(ParaKind::Numbered, "c", 1),
                deep(ParaKind::Bullet, "d", 2),
                aligned(ParaKind::Bullet, "e", Align::Center, 0),
            ]),
            "<ul><li><p>a</p><ol><li><p>b</p></li><li><p>c</p>\
             <ul><li><p>d</p></li></ul></li></ol></li>\
             <li><p style=\"text-align:center\">e</p></li></ul>",
        ),
        (
            "an item cannot skip a level, and the first is at the outer one",
            doc_of(vec![
                deep(ParaKind::Bullet, "a", 2),
                deep(ParaKind::Bullet, "b", 3),
            ]),
            "<ul><li><p>a</p><ul><li><p>b</p></li></ul></li></ul>",
        ),
        (
            "a list of another kind at the outer level is its own list",
            doc_of(vec![
                deep(ParaKind::Bullet, "a", 0),
                deep(ParaKind::Numbered, "b", 0),
            ]),
            "<ul><li><p>a</p></li></ul><ol><li><p>b</p></li></ol>",
        ),
        (
            "nested to-dos",
            doc_of(vec![
                deep(ParaKind::Todo(Check::Open), "a", 0),
                deep(ParaKind::Todo(Check::Done), "b", 1),
            ]),
            "<ul><li><p>☐ a</p><ul><li><p>☑ b</p></li></ul></li></ul>",
        ),
        (
            "quote levels share their quotes",
            doc_of(vec![
                deep(ParaKind::Quote, "a", 0),
                deep(ParaKind::Quote, "b", 2),
                aligned(ParaKind::Quote, "c", Align::Right, 0),
                plain(ParaKind::Paragraph, "d"),
            ]),
            "<blockquote><p>a</p><blockquote><blockquote><p>b</p></blockquote></blockquote>\
             <p style=\"text-align:right\">c</p></blockquote><p>d</p>",
        ),
    ];
    for (name, doc, html) in cases {
        assert_eq!(to_html(&doc), html, "{name}");
    }
}

#[test]
fn plain_text_indents_nests_and_quotes() {
    // (what, the document, the text)
    let cases: Vec<(&str, Doc, &str)> = vec![
        (
            "size, colour and alignment leave the words as they are",
            doc_of(vec![Node::laid(
                ParaKind::Paragraph,
                vec![sized("loud", Some(TextSize::Huge), Some(TextColor::Red))],
                Layout {
                    align: Align::Center,
                    depth: 0,
                },
            )]),
            "loud\n",
        ),
        (
            "an indented paragraph is four spaces a level, kept by the stuffing",
            doc_of(vec![deep(ParaKind::Paragraph, "in", 1)]),
            "     in\n",
        ),
        (
            "a nested list is two spaces a level, each level counting its own",
            doc_of(vec![
                deep(ParaKind::Numbered, "a", 0),
                deep(ParaKind::Numbered, "b", 1),
                deep(ParaKind::Numbered, "c", 1),
                deep(ParaKind::Bullet, "d", 2),
                deep(ParaKind::Numbered, "e", 0),
            ]),
            "1. a\n   1. b\n   2. c\n     - d\n2. e\n",
        ),
        (
            "a quote is one > a level",
            doc_of(vec![
                deep(ParaKind::Quote, "one", 0),
                deep(ParaKind::Quote, "three", 2),
            ]),
            "> one\n\n>>> three\n",
        ),
    ];
    for (name, doc, text) in cases {
        assert_eq!(to_flowed(&doc), text, "{name}");
    }
}

/// What the composer writes, a reopened draft reads back: every format, in every place it can
/// be.
#[test]
fn every_format_survives_the_round_trip() {
    let mut linked = marks_of(&[Mark::Italic]);
    linked.link = Some(url());
    linked.size = Some(TextSize::Large);
    let doc = doc_of(vec![
        Node::laid(
            ParaKind::Paragraph,
            vec![
                sized("Small ", Some(TextSize::Small), None),
                sized("blue ", None, Some(TextColor::Blue)),
                sized("huge purple", Some(TextSize::Huge), Some(TextColor::Purple)),
                Run::new(" and ", Marks::new()),
                Run::new("a large italic link", linked),
            ],
            Layout {
                align: Align::Center,
                depth: 0,
            },
        ),
        aligned(ParaKind::Paragraph, "indented right", Align::Right, 2),
        aligned(
            ParaKind::Heading(Level::Two),
            "indented heading",
            Align::Left,
            1,
        ),
        deep(ParaKind::Bullet, "one", 0),
        deep(ParaKind::Numbered, "one a", 1),
        aligned(ParaKind::Numbered, "one b", Align::Center, 1),
        deep(ParaKind::Todo(Check::Done), "one b i", 2),
        deep(ParaKind::Bullet, "two", 0),
        deep(ParaKind::Quote, "first level", 0),
        deep(ParaKind::Quote, "third level", 2),
        aligned(ParaKind::Quote, "second level", Align::Right, 1),
        plain(ParaKind::Paragraph, "after"),
    ]);
    let back = round_trip(&doc);
    assert_eq!(canon(&back), canon(&doc), "{}", to_html(&doc));
    assert_eq!(
        levels(&back),
        "p0|p2|h21|ul0|ol1|ol1|done2|ul0|quote0|quote2|quote1|p0"
    );
    assert_eq!(back, doc, "the round trip is exact here");
}

/// A pasted or reopened body keeps the composer's styles and nothing else of the sender's CSS.
#[test]
fn only_the_composer_s_styles_come_back_from_html() {
    let html = "<p style=\"text-align:center;color:#d93025;font-size:13px;position:fixed\">\
                <span style=\"color:#123456;font-size:large\">a</span>\
                <span style=\"color:#9334E6;background:url(https://t.example/p)\">b</span></p>\
                <div style=\"margin-left:40px;text-align:justify\">c</div>";
    let safe = sanitize(html, SanitizePolicy::CURRENT);
    assert!(!safe.as_str().contains("13px"), "{}", safe.as_str());
    assert!(!safe.as_str().contains("t.example"), "{}", safe.as_str());
    assert!(!safe.as_str().contains("#123456"), "{}", safe.as_str());
    let nodes = nodes_from_blocks(&from_html(&safe, &[], RemoteImages::Blocked).blocks);
    let expected = vec![
        Node::laid(
            ParaKind::Paragraph,
            vec![
                sized("a", Some(TextSize::Large), None),
                sized("b", None, Some(TextColor::Purple)),
            ],
            Layout {
                align: Align::Center,
                depth: 0,
            },
        ),
        deep(ParaKind::Paragraph, "c", 1),
    ];
    assert_eq!(nodes, expected);
}

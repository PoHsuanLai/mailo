//! Enter and Backspace at every paragraph kind's edge, and over an object.

use super::*;

#[test]
fn keys_at_every_edge() {
    struct Case {
        name: &'static str,
        nodes: Vec<Node>,
        caret: Caret,
        key: Key,
        body: &'static str,
        kinds: &'static str,
        grip: Grip,
    }
    #[derive(Clone, Copy)]
    enum Key {
        Enter,
        Backspace,
    }
    let ab = |kind| plain(kind, "ab");
    let empty = |kind| plain(kind, "");
    let cases = [
        Case {
            name: "enter paragraph end",
            nodes: vec![ab(ParaKind::Paragraph)],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab|",
            kinds: "p|p",
            grip: Grip::Off,
        },
        Case {
            name: "enter paragraph middle",
            nodes: vec![ab(ParaKind::Paragraph)],
            caret: Caret::at(0, 1),
            key: Key::Enter,
            body: "a|b",
            kinds: "p|p",
            grip: Grip::Off,
        },
        Case {
            name: "enter heading end",
            nodes: vec![ab(ParaKind::Heading(Level::One))],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab|",
            kinds: "h1|p",
            grip: Grip::Off,
        },
        Case {
            name: "enter heading middle",
            nodes: vec![ab(ParaKind::Heading(Level::Two))],
            caret: Caret::at(0, 1),
            key: Key::Enter,
            body: "a|b",
            kinds: "h2|p",
            grip: Grip::Off,
        },
        Case {
            name: "enter empty heading",
            nodes: vec![empty(ParaKind::Heading(Level::Three))],
            caret: Caret::at(0, 0),
            key: Key::Enter,
            body: "",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "enter bullet",
            nodes: vec![ab(ParaKind::Bullet)],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab|",
            kinds: "ul|ul",
            grip: Grip::Off,
        },
        Case {
            name: "enter empty bullet",
            nodes: vec![empty(ParaKind::Bullet)],
            caret: Caret::at(0, 0),
            key: Key::Enter,
            body: "",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "enter numbered",
            nodes: vec![ab(ParaKind::Numbered)],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab|",
            kinds: "ol|ol",
            grip: Grip::Off,
        },
        Case {
            name: "enter empty numbered",
            nodes: vec![empty(ParaKind::Numbered)],
            caret: Caret::at(0, 0),
            key: Key::Enter,
            body: "",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "enter todo",
            nodes: vec![plain(ParaKind::Todo(Check::Done), "ab")],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab|",
            kinds: "done|todo",
            grip: Grip::Off,
        },
        Case {
            name: "enter empty todo",
            nodes: vec![empty(ParaKind::Todo(Check::Open))],
            caret: Caret::at(0, 0),
            key: Key::Enter,
            body: "",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "enter quote end",
            nodes: vec![ab(ParaKind::Quote)],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab|",
            kinds: "quote|p",
            grip: Grip::Off,
        },
        Case {
            name: "enter quote middle",
            nodes: vec![ab(ParaKind::Quote)],
            caret: Caret::at(0, 1),
            key: Key::Enter,
            body: "a|b",
            kinds: "quote|quote",
            grip: Grip::Off,
        },
        Case {
            name: "enter empty quote",
            nodes: vec![empty(ParaKind::Quote)],
            caret: Caret::at(0, 0),
            key: Key::Enter,
            body: "",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "enter code",
            nodes: vec![ab(ParaKind::Code)],
            caret: Caret::at(0, 2),
            key: Key::Enter,
            body: "ab\n",
            kinds: "code",
            grip: Grip::Off,
        },
        Case {
            name: "backspace heading start",
            nodes: vec![ab(ParaKind::Heading(Level::One))],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace bullet start",
            nodes: vec![ab(ParaKind::Bullet)],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace numbered start",
            nodes: vec![ab(ParaKind::Numbered)],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace todo start",
            nodes: vec![ab(ParaKind::Todo(Check::Open))],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace quote start",
            nodes: vec![ab(ParaKind::Quote)],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace code start",
            nodes: vec![ab(ParaKind::Code)],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace paragraph char",
            nodes: vec![ab(ParaKind::Paragraph)],
            caret: Caret::at(0, 2),
            key: Key::Backspace,
            body: "a",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace merge",
            nodes: vec![
                plain(ParaKind::Paragraph, "xy"),
                plain(ParaKind::Paragraph, "ab"),
            ],
            caret: Caret::at(1, 0),
            key: Key::Backspace,
            body: "xyab",
            kinds: "p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace arms object",
            nodes: vec![
                Node::Object(Object::Divider),
                plain(ParaKind::Paragraph, "ab"),
            ],
            caret: Caret::at(1, 0),
            key: Key::Backspace,
            body: "#|ab",
            kinds: "hr|p",
            grip: Grip::Armed(0),
        },
        Case {
            name: "backspace armed after an object's own position",
            nodes: vec![
                plain(ParaKind::Paragraph, "ab"),
                Node::Object(Object::Divider),
            ],
            caret: Caret::at(1, 1),
            key: Key::Backspace,
            body: "ab|#",
            kinds: "p|hr",
            grip: Grip::Armed(1),
        },
        Case {
            name: "enter before an object",
            nodes: vec![Node::Object(Object::Divider)],
            caret: Caret::at(0, 0),
            key: Key::Enter,
            body: "|#",
            kinds: "p|hr",
            grip: Grip::Off,
        },
        Case {
            name: "enter after an object",
            nodes: vec![Node::Object(Object::Divider)],
            caret: Caret::at(0, 1),
            key: Key::Enter,
            body: "#|",
            kinds: "hr|p",
            grip: Grip::Off,
        },
        Case {
            name: "backspace merges into a heading, which keeps its kind",
            nodes: vec![
                plain(ParaKind::Heading(Level::Two), "xy"),
                plain(ParaKind::Paragraph, "ab"),
            ],
            caret: Caret::at(1, 0),
            key: Key::Backspace,
            body: "xyab",
            kinds: "h2",
            grip: Grip::Off,
        },
        Case {
            name: "backspace doc start",
            nodes: vec![plain(ParaKind::Paragraph, "ab")],
            caret: Caret::at(0, 0),
            key: Key::Backspace,
            body: "ab",
            kinds: "p",
            grip: Grip::Off,
        },
    ];
    for case in cases {
        let mut doc = doc_of(case.nodes);
        let motion = match case.key {
            Key::Enter => {
                enter(&doc, case.caret).unwrap_or_else(|err| panic!("{}: {err}", case.name))
            }
            Key::Backspace => {
                backspace(&doc, case.caret).unwrap_or_else(|err| panic!("{}: {err}", case.name))
            }
        };
        apply_ops(&mut doc, motion.ops);
        assert_eq!(body(&doc), case.body, "{}", case.name);
        assert_eq!(kinds(&doc), case.kinds, "{}", case.name);
        assert_eq!(motion.caret.grip, case.grip, "{}", case.name);
    }

    let mut doc = doc_of(vec![
        Node::Object(Object::Divider),
        plain(ParaKind::Paragraph, "ab"),
    ]);
    let armed = backspace(&doc, Caret::at(1, 0)).unwrap();
    assert!(armed.ops.is_empty());
    let second = backspace(&doc, armed.caret).unwrap();
    apply_ops(&mut doc, second.ops);
    assert_eq!(
        kinds(&doc),
        "p",
        "second backspace deletes the armed object"
    );
    assert_eq!(body(&doc), "ab");
}

#[test]
fn the_second_backspace_on_the_last_object_leaves_a_paragraph() {
    let mut doc = doc_of(vec![Node::Object(Object::Image {
        src: ImageRef::new("cid:1"),
        alt: String::new(),
    })]);
    let armed = backspace(&doc, Caret::at(0, 1)).unwrap();
    assert_eq!(armed.caret.grip, Grip::Armed(0));
    let second = backspace(&doc, armed.caret).unwrap();
    apply_ops(&mut doc, second.ops);
    assert_eq!(kinds(&doc), "p");
    assert_eq!(second.caret, Caret::at(0, 0));
}

#[test]
fn an_empty_item_leaves_the_list_by_enter_in_a_session() {
    let mut session = Session::with(doc_of(vec![plain(ParaKind::Bullet, "milk")]));
    session.caret = Caret::at(0, 4);
    for at in [0, 10] {
        let enter = caret_event("insertParagraph", None, session.caret.pos, false);
        session.handle(&enter, at).unwrap();
    }
    assert_eq!(kinds(&session.doc), "ul|p");
    assert_eq!(body(&session.doc), "milk|");
}

/// Tab and Shift-Tab in a list item nest it under the item before it and bring it back out;
/// outside a list they are not the editor's.
#[test]
fn tab_and_shift_tab_in_a_list() {
    // (what, nodes, caret's node, nest, levels after, whether Tab is the editor's)
    type Case = (&'static str, Vec<Node>, usize, Nest, &'static str, bool);
    let item = |text: &str, depth| deep(ParaKind::Bullet, text, depth);
    let cases: Vec<Case> = vec![
        (
            "tab nests the second item",
            vec![item("a", 0), item("b", 0)],
            1,
            Nest::In,
            "ul0|ul1",
            true,
        ),
        (
            "tab on the first item does nothing, but is still the list's",
            vec![item("a", 0), item("b", 0)],
            0,
            Nest::In,
            "ul0|ul0",
            true,
        ),
        (
            "tab goes no deeper than one under the item before",
            vec![item("a", 0), item("b", 1)],
            1,
            Nest::In,
            "ul0|ul1",
            true,
        ),
        (
            "tab nests a numbered item under a bullet",
            vec![item("a", 0), deep(ParaKind::Numbered, "b", 0)],
            1,
            Nest::In,
            "ul0|ol1",
            true,
        ),
        (
            "tab nests a to-do",
            vec![
                deep(ParaKind::Todo(Check::Open), "a", 0),
                deep(ParaKind::Todo(Check::Open), "b", 0),
            ],
            1,
            Nest::In,
            "todo0|todo1",
            true,
        ),
        (
            "shift-tab brings an item out",
            vec![item("a", 0), item("b", 2)],
            1,
            Nest::Out,
            "ul0|ul1",
            true,
        ),
        (
            "shift-tab at the outer level stays in the list",
            vec![item("a", 0)],
            0,
            Nest::Out,
            "ul0",
            true,
        ),
        (
            "tab at the deepest level stays there",
            vec![item("a", 0), item("b", Layout::MAX_DEPTH)],
            1,
            Nest::In,
            "ul0|ul6",
            true,
        ),
        (
            "tab in a paragraph is not the editor's",
            vec![plain(ParaKind::Paragraph, "a")],
            0,
            Nest::In,
            "p0",
            false,
        ),
        (
            "tab in a quote is not the editor's",
            vec![plain(ParaKind::Quote, "a")],
            0,
            Nest::In,
            "quote0",
            false,
        ),
    ];
    for (name, nodes, node, nest, after, taken) in cases {
        let mut doc = doc_of(nodes);
        let caret = Pos::new(node, 0);
        let ops = tab(
            &doc,
            Range {
                start: caret,
                end: caret,
            },
            nest,
        );
        assert_eq!(ops.is_some(), taken, "{name}");
        apply_ops(&mut doc, ops.unwrap_or_default());
        assert_eq!(levels(&doc), after, "{name}");
    }
}

/// Tab over a selection of items nests each that has an item to nest under, keeping their
/// order; Shift-Tab brings each out.
#[test]
fn tab_over_a_selection_moves_every_item() {
    let mut doc = doc_of(vec![
        deep(ParaKind::Bullet, "a", 0),
        deep(ParaKind::Bullet, "b", 0),
        deep(ParaKind::Bullet, "c", 1),
    ]);
    let all = Range {
        start: Pos::new(0, 0),
        end: Pos::new(2, 1),
    };
    let ops = tab(&doc, all, Nest::In).expect("a list");
    apply_ops(&mut doc, ops);
    assert_eq!(levels(&doc), "ul0|ul1|ul2");
    for _ in 0..3 {
        let ops = tab(&doc, all, Nest::Out).expect("a list");
        apply_ops(&mut doc, ops);
    }
    assert_eq!(levels(&doc), "ul0|ul0|ul0");
}

/// ⌘' quotes a level more and ⌥⌘' a level less; Enter and Backspace in a nested item or quote
/// come out a level before they leave.
#[test]
fn quote_levels_and_nested_enter_and_backspace() {
    // (what, nodes, caret, what is done, levels after)
    #[derive(Clone, Copy)]
    enum Do {
        Quote(Nest),
        Enter,
        Backspace,
    }
    type Case = (&'static str, Vec<Node>, Caret, Do, &'static str);
    let cases: Vec<Case> = vec![
        (
            "a paragraph becomes a quote",
            vec![plain(ParaKind::Paragraph, "a")],
            Caret::at(0, 0),
            Do::Quote(Nest::In),
            "quote0",
        ),
        (
            "an indented paragraph becomes a first-level quote",
            vec![deep(ParaKind::Paragraph, "a", 2)],
            Caret::at(0, 0),
            Do::Quote(Nest::In),
            "quote0",
        ),
        (
            "a quote goes a level deeper",
            vec![deep(ParaKind::Quote, "a", 1)],
            Caret::at(0, 0),
            Do::Quote(Nest::In),
            "quote2",
        ),
        (
            "the deepest quote stays",
            vec![deep(ParaKind::Quote, "a", Layout::MAX_DEPTH)],
            Caret::at(0, 0),
            Do::Quote(Nest::In),
            "quote6",
        ),
        (
            "code is not quoted",
            vec![plain(ParaKind::Code, "a")],
            Caret::at(0, 0),
            Do::Quote(Nest::In),
            "code0",
        ),
        (
            "a deeper quote comes up a level",
            vec![deep(ParaKind::Quote, "a", 2)],
            Caret::at(0, 0),
            Do::Quote(Nest::Out),
            "quote1",
        ),
        (
            "a first-level quote is a paragraph again",
            vec![plain(ParaKind::Quote, "a")],
            Caret::at(0, 0),
            Do::Quote(Nest::Out),
            "p0",
        ),
        (
            "a paragraph has no quote to lose",
            vec![plain(ParaKind::Paragraph, "a")],
            Caret::at(0, 0),
            Do::Quote(Nest::Out),
            "p0",
        ),
        (
            "enter in a nested item keeps its level",
            vec![
                deep(ParaKind::Bullet, "a", 0),
                deep(ParaKind::Bullet, "b", 1),
            ],
            Caret::at(1, 1),
            Do::Enter,
            "ul0|ul1|ul1",
        ),
        (
            "enter on an empty nested item comes out a level",
            vec![
                deep(ParaKind::Bullet, "a", 0),
                deep(ParaKind::Bullet, "", 1),
            ],
            Caret::at(1, 0),
            Do::Enter,
            "ul0|ul0",
        ),
        (
            "enter on an empty outer item leaves the list",
            vec![
                deep(ParaKind::Bullet, "a", 0),
                deep(ParaKind::Bullet, "", 0),
            ],
            Caret::at(1, 0),
            Do::Enter,
            "ul0|p0",
        ),
        (
            "enter on an empty deeper quote comes up a level",
            vec![deep(ParaKind::Quote, "", 1)],
            Caret::at(0, 0),
            Do::Enter,
            "quote0",
        ),
        (
            "enter at the end of a deeper quote leaves every level",
            vec![deep(ParaKind::Quote, "a", 2)],
            Caret::at(0, 1),
            Do::Enter,
            "quote2|p0",
        ),
        (
            "backspace at the start of a nested item comes out a level",
            vec![
                deep(ParaKind::Bullet, "a", 0),
                deep(ParaKind::Bullet, "b", 2),
            ],
            Caret::at(1, 0),
            Do::Backspace,
            "ul0|ul1",
        ),
        (
            "backspace at the start of a deeper quote comes up a level",
            vec![deep(ParaKind::Quote, "a", 1)],
            Caret::at(0, 0),
            Do::Backspace,
            "quote0",
        ),
        (
            "backspace at the start of an indented paragraph outdents it",
            vec![deep(ParaKind::Paragraph, "a", 1)],
            Caret::at(0, 0),
            Do::Backspace,
            "p0",
        ),
    ];
    for (name, nodes, caret, what, after) in cases {
        let mut doc = doc_of(nodes);
        let at = Range {
            start: caret.pos,
            end: caret.pos,
        };
        let ops = match what {
            Do::Quote(nest) => quote(&doc, at, nest),
            Do::Enter => {
                enter(&doc, caret)
                    .unwrap_or_else(|err| panic!("{name}: {err}"))
                    .ops
            }
            Do::Backspace => {
                backspace(&doc, caret)
                    .unwrap_or_else(|err| panic!("{name}: {err}"))
                    .ops
            }
        };
        apply_ops(&mut doc, ops);
        assert_eq!(levels(&doc), after, "{name}");
    }
}

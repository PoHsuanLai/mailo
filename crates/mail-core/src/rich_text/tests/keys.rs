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

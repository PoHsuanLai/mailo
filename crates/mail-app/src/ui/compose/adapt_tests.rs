//! The adapter's tables: every input it reads, and positions both ways.

use dioxus::prelude::{Key, Modifiers};
use ds::{
    Clicks, Composition, EditInput, EditPointer, Extend, KeyInput, Pasted, Point, PointerPhase,
    TextPosition,
};

use super::{
    Asked, Reach, Step, asked, para_at, pointer_selection, pos_of, selected_text, step,
    text_position, word_at,
};
use crate::editor::{Doc, InputEvent, Node, Object, ParaKind, Pos, Range};

fn key(key: Key, modifiers: Modifiers) -> EditInput {
    EditInput::Key(KeyInput { key, modifiers })
}

fn event(input_type: &str, data: Option<&str>, composing: bool) -> Asked {
    Asked::Edit(InputEvent::new(
        input_type,
        data.map(str::to_owned),
        Vec::new(),
        composing,
    ))
}

#[test]
fn every_input_reads_as_the_event_the_glue_would_have_sent() {
    let none = Modifiers::empty();
    let ctrl = Modifiers::CONTROL;
    let shift = Modifiers::SHIFT;
    let mut html = InputEvent::new(
        "insertFromPaste",
        Some("bold".to_owned()),
        Vec::new(),
        false,
    );
    html.html = Some("<b>bold</b>".to_owned());
    let cases: Vec<(EditInput, Asked)> = vec![
        (
            EditInput::Text("a".to_owned()),
            event("insertText", Some("a"), false),
        ),
        (key(Key::Enter, none), event("insertParagraph", None, false)),
        (
            key(Key::Enter, shift),
            event("insertLineBreak", None, false),
        ),
        (key(Key::Enter, ctrl), Asked::Nothing),
        (
            key(Key::Backspace, none),
            event("deleteContentBackward", None, false),
        ),
        (
            key(Key::Backspace, ctrl),
            event("deleteWordBackward", None, false),
        ),
        (
            key(Key::Backspace, Modifiers::ALT),
            event("deleteWordBackward", None, false),
        ),
        (
            key(Key::Delete, none),
            event("deleteContentForward", None, false),
        ),
        (
            key(Key::ArrowLeft, none),
            Asked::Move(Step::Left, Reach::Collapse),
        ),
        (
            key(Key::ArrowRight, shift),
            Asked::Move(Step::Right, Reach::Extend),
        ),
        (
            key(Key::ArrowLeft, ctrl),
            Asked::Move(Step::WordLeft, Reach::Collapse),
        ),
        (
            key(Key::ArrowUp, none),
            Asked::Move(Step::Up, Reach::Collapse),
        ),
        (
            key(Key::ArrowDown, shift),
            Asked::Move(Step::Down, Reach::Extend),
        ),
        (
            key(Key::Home, none),
            Asked::Move(Step::LineStart, Reach::Collapse),
        ),
        (
            key(Key::End, shift),
            Asked::Move(Step::LineEnd, Reach::Extend),
        ),
        (
            key(Key::Home, ctrl),
            Asked::Move(Step::DocStart, Reach::Collapse),
        ),
        (key(Key::Character("a".to_owned()), ctrl), Asked::SelectAll),
        (key(Key::Tab, none), Asked::Nothing),
        (
            EditInput::Composition(Composition::Start),
            event("compositionstart", None, true),
        ),
        (
            EditInput::Composition(Composition::Update {
                text: "ㄓ".to_owned(),
                cursor: None,
            }),
            event("insertCompositionText", Some("ㄓ"), true),
        ),
        (
            EditInput::Composition(Composition::End {
                text: "注".to_owned(),
            }),
            event("compositionend", Some("注"), false),
        ),
        (
            EditInput::Paste(Pasted::Text("plain".to_owned())),
            event("insertFromPaste", Some("plain"), false),
        ),
        (
            EditInput::Paste(Pasted::Html {
                html: "<b>bold</b>".to_owned(),
                text: "bold".to_owned(),
            }),
            Asked::Edit(html),
        ),
        (EditInput::Cut, Asked::Cut),
        (EditInput::Copy, Asked::Copy),
    ];
    for (input, expected) in cases {
        assert_eq!(asked(&input), expected, "{input:?}");
    }
}

/// "注音 a👩‍👩‍👧b", then a divider.
fn doc() -> Doc {
    Doc {
        nodes: vec![
            Node::plain(ParaKind::Paragraph, "注音 a👩‍👩‍👧b"),
            Node::Object(Object::Divider),
            Node::plain(ParaKind::Paragraph, "two words"),
        ],
    }
}

#[test]
fn byte_offsets_and_grapheme_offsets_convert_both_ways() {
    let doc = doc();
    // (grapheme, byte): 注 is 3 bytes, the family 18.
    let pairs = [(0, 0), (1, 3), (2, 6), (3, 7), (4, 8), (5, 26), (6, 27)];
    for (grapheme, byte) in pairs {
        let pos = Pos::new(0, grapheme);
        assert_eq!(text_position(&doc, pos), TextPosition::new("0", byte));
        assert_eq!(
            pos_of(&doc, &TextPosition::new("0", byte)),
            Some(pos),
            "{byte}"
        );
    }
    // Inside a cluster, or past the end, lands on a cluster boundary.
    assert_eq!(
        pos_of(&doc, &TextPosition::new("0", 12)),
        Some(Pos::new(0, 4))
    );
    assert_eq!(
        pos_of(&doc, &TextPosition::new("0", 99)),
        Some(Pos::new(0, 6))
    );
    // An atom is 0 or 1 both ways.
    assert_eq!(
        text_position(&doc, Pos::new(1, 1)),
        TextPosition::new("1", 1)
    );
    assert_eq!(
        pos_of(&doc, &TextPosition::new("1", 1)),
        Some(Pos::new(1, 1))
    );
    assert_eq!(pos_of(&doc, &TextPosition::new("9", 0)), None);
    assert_eq!(pos_of(&doc, &TextPosition::new("x", 0)), None);
}

#[test]
fn steps_cross_paragraphs_objects_and_words() {
    let doc = doc();
    let cases = [
        (Pos::new(0, 0), Step::Left, Pos::new(0, 0)),
        (Pos::new(0, 6), Step::Right, Pos::new(1, 0)),
        (Pos::new(1, 0), Step::Right, Pos::new(1, 1)),
        (Pos::new(1, 1), Step::Right, Pos::new(2, 0)),
        (Pos::new(2, 0), Step::Left, Pos::new(1, 1)),
        (Pos::new(2, 9), Step::Right, Pos::new(2, 9)),
        (Pos::new(2, 6), Step::WordLeft, Pos::new(2, 4)),
        (Pos::new(2, 4), Step::WordLeft, Pos::new(2, 0)),
        (Pos::new(2, 0), Step::WordRight, Pos::new(2, 3)),
        (Pos::new(2, 3), Step::WordRight, Pos::new(2, 9)),
        (Pos::new(2, 5), Step::DocStart, Pos::new(0, 0)),
        (Pos::new(0, 1), Step::DocEnd, Pos::new(2, 9)),
    ];
    for (from, by, to) in cases {
        assert_eq!(step(&doc, from, by), to, "{by:?} from {from:?}");
    }
}

#[test]
fn a_double_click_takes_the_word_and_a_triple_the_paragraph() {
    let doc = doc();
    let range = |node, start, end| Range {
        start: Pos::new(node, start),
        end: Pos::new(node, end),
    };
    assert_eq!(word_at(&doc, Pos::new(2, 1)), range(2, 0, 3));
    assert_eq!(
        word_at(&doc, Pos::new(2, 3)),
        range(2, 0, 3),
        "just after it"
    );
    assert_eq!(word_at(&doc, Pos::new(2, 7)), range(2, 4, 9));
    assert_eq!(para_at(&doc, Pos::new(2, 7)), range(2, 0, 9));
    assert_eq!(para_at(&doc, Pos::new(1, 0)), range(1, 0, 1));
}

#[test]
fn the_selected_text_joins_paragraphs_and_leaves_objects_out() {
    let doc = doc();
    let range = Range {
        start: Pos::new(2, 4),
        end: Pos::new(0, 3),
    };
    assert_eq!(selected_text(&doc, range), "a👩‍👩‍👧b\ntwo ");
}

#[test]
fn a_press_places_shift_extends_a_drag_follows_and_multiple_presses_widen() {
    let doc = doc();
    let anchor = Pos::new(2, 1);
    let pointer = |phase, clicks, extend, node: &str, offset| EditPointer {
        phase,
        at: Point::default(),
        position: Some(TextPosition::new(node, offset)),
        extend,
        clicks: Clicks(clicks),
    };
    // "two words": byte 6 is inside "words".
    let at = Pos::new(2, 6);
    let cases = [
        (
            pointer(PointerPhase::Press, 1, Extend::Fresh, "2", 6),
            Some((at, at)),
        ),
        (
            pointer(PointerPhase::Press, 1, Extend::FromAnchor, "2", 6),
            Some((anchor, at)),
        ),
        (
            pointer(PointerPhase::Drag, 1, Extend::Fresh, "2", 6),
            Some((anchor, at)),
        ),
        (
            pointer(PointerPhase::Press, 2, Extend::Fresh, "2", 6),
            Some((Pos::new(2, 4), Pos::new(2, 9))),
        ),
        (
            pointer(PointerPhase::Press, 3, Extend::Fresh, "2", 6),
            Some((Pos::new(2, 0), Pos::new(2, 9))),
        ),
        (pointer(PointerPhase::Drag, 2, Extend::Fresh, "2", 6), None),
        (
            pointer(PointerPhase::Release, 1, Extend::Fresh, "2", 6),
            None,
        ),
        (pointer(PointerPhase::Press, 1, Extend::Fresh, "7", 0), None),
    ];
    for (pointer, expected) in cases {
        assert_eq!(
            pointer_selection(&doc, anchor, &pointer),
            expected,
            "{pointer:?}"
        );
    }
    let mut nowhere = pointer(PointerPhase::Press, 1, Extend::Fresh, "2", 0);
    nowhere.position = None;
    assert_eq!(pointer_selection(&doc, anchor, &nowhere), None);
}

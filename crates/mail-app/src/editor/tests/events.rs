//! Recorded `beforeinput` sequences, replayed into documents.

use super::*;

fn range_event(input_type: &str, data: Option<&str>, start: Pos, end: Pos) -> InputEvent {
    InputEvent::new(
        input_type,
        data.map(str::to_owned),
        vec![Range { start, end }],
        false,
    )
}

fn type_text(session: &mut Session, text: &str, start_ms: u64, step_ms: u64) {
    for (index, grapheme) in
        unicode_segmentation::UnicodeSegmentation::graphemes(text, true).enumerate()
    {
        let event = caret_event("insertText", Some(grapheme), session.caret.pos, false);
        session
            .handle(&event, start_ms + index as u64 * step_ms)
            .unwrap_or_else(|err| panic!("typing {grapheme:?}: {err}"));
    }
}

#[test]
fn typing_then_enter_then_typing() {
    let mut session = Session::new();
    type_text(&mut session, "Hi", 0, 10);
    session
        .handle(
            &caret_event("insertParagraph", None, session.caret.pos, false),
            30,
        )
        .unwrap();
    type_text(&mut session, "there", 40, 10);
    assert_eq!(body(&session.doc), "Hi|there");
    assert_eq!(session.caret.pos, Pos::new(1, 5));
}

#[test]
fn a_cut_across_paragraphs_joins_what_is_left() {
    let mut doc = doc_of(vec![
        plain(ParaKind::Heading(Level::One), "hello"),
        Node::Object(Object::Divider),
        plain(ParaKind::Paragraph, "world"),
    ]);
    let cut = range_event("deleteByCut", None, Pos::new(0, 2), Pos::new(2, 3));
    let edit = interpret(&doc, Caret::at(0, 0), &cut).unwrap();
    assert_eq!(edit.record, Record::Structural);
    apply_ops(&mut doc, edit.ops);
    assert_eq!(body(&doc), "held", "the object between the ends goes too");
    assert_eq!(kinds(&doc), "h1", "the first paragraph's kind survives");
}

#[test]
fn deleting_everything_leaves_a_paragraph_to_type_in() {
    let mut session = Session::with(doc_of(vec![
        plain(ParaKind::Quote, "one"),
        Node::Object(Object::Divider),
        plain(ParaKind::Paragraph, "two"),
    ]));
    let all = range_event(
        "deleteContentBackward",
        None,
        Pos::new(0, 0),
        Pos::new(2, 3),
    );
    session.handle(&all, 0).unwrap();
    assert_eq!(body(&session.doc), "");
    assert_eq!(kinds(&session.doc), "quote");
    type_text(&mut session, "x", 10, 10);
    assert_eq!(body(&session.doc), "x");
}

#[test]
fn a_selection_replaced_by_typing_and_by_enter() {
    let doc = doc_of(vec![plain(ParaKind::Paragraph, "abcdef")]);
    let mut session = Session::with(doc.clone());
    session
        .handle(
            &range_event("insertText", Some("X"), Pos::new(0, 1), Pos::new(0, 5)),
            0,
        )
        .unwrap();
    assert_eq!(body(&session.doc), "aXf");

    let mut session = Session::with(doc);
    session
        .handle(
            &range_event("insertParagraph", None, Pos::new(0, 2), Pos::new(0, 4)),
            0,
        )
        .unwrap();
    assert_eq!(body(&session.doc), "ab|ef");
    assert_eq!(session.caret.pos, Pos::new(1, 0));
}

#[test]
fn paste_html_goes_through_the_sanitizer_into_nodes() {
    let mut session = Session::with(doc_of(vec![plain(ParaKind::Paragraph, "ab")]));
    let mut paste = range_event("insertFromPaste", None, Pos::new(0, 1), Pos::new(0, 1));
    paste.html = Some(
        "<p>Hello <strong>world</strong></p><script>alert(1)</script>\
         <ul><li>item</li></ul><p onclick=\"x()\">Next</p>"
            .to_owned(),
    );
    session.handle(&paste, 0).unwrap();
    assert_eq!(body(&session.doc), "a|Hello world|item|Next|b");
    assert_eq!(kinds(&session.doc), "p|p|ul|p|p");
    let Node::Para { runs, .. } = &session.doc.nodes[1] else {
        panic!("paragraph");
    };
    assert!(
        runs.iter()
            .any(|run| run.text == "world" && run.marks.has(Mark::Bold))
    );
    assert_eq!(
        session.caret.pos,
        Pos::new(3, 4),
        "the caret ends after the paste"
    );
}

#[test]
fn paste_plain_text() {
    // Several paragraphs: a blank line starts one, a single newline stays inside.
    let mut session = Session::new();
    let paste = caret_event(
        "insertFromPaste",
        Some("one\n\ntwo\nlines"),
        Pos::new(0, 0),
        false,
    );
    session.handle(&paste, 0).unwrap();
    assert_eq!(body(&session.doc), "one|two\nlines");

    // One paragraph is typed into the line, not split around it.
    let mut session = Session::with(Doc::from_text("abcd"));
    let paste = caret_event("insertFromPaste", Some("XY"), Pos::new(0, 2), false);
    session.handle(&paste, 0).unwrap();
    assert_eq!(body(&session.doc), "abXYcd");
    assert_eq!(session.caret.pos, Pos::new(0, 4));
}

/// Each step: the event, its data, and whether the IME is composing.
type Steps<'a> = &'a [(&'a str, Option<&'a str>, bool)];

#[test]
fn composition_is_silent_until_it_ends_then_one_insert() {
    // Zhuyin: ㄋ, ㄋㄧ, ㄋㄧˇ, then the candidate 你, then 好, committed together.
    let zhuyin: Steps = &[
        ("compositionstart", None, true),
        ("insertCompositionText", Some("ㄋ"), true),
        ("insertCompositionText", Some("ㄋㄧ"), true),
        ("insertCompositionText", Some("ㄋㄧˇ"), true),
        ("insertCompositionText", Some("你"), true),
        ("insertCompositionText", Some("你ㄏㄠˇ"), true),
        ("insertCompositionText", Some("你好"), true),
        ("compositionend", Some("你好"), false),
    ];
    // Pinyin: letters, then the candidate.
    let pinyin: Steps = &[
        ("compositionstart", None, true),
        ("insertCompositionText", Some("n"), true),
        ("insertCompositionText", Some("ni"), true),
        ("insertCompositionText", Some("nih"), true),
        ("insertCompositionText", Some("niha"), true),
        ("insertCompositionText", Some("nihao"), true),
        ("insertCompositionText", Some("你好"), true),
        ("compositionend", Some("你好"), false),
    ];
    // Japanese: romaji to kana, then kana to kanji on space.
    let kana: Steps = &[
        ("compositionstart", None, true),
        ("insertCompositionText", Some("k"), true),
        ("insertCompositionText", Some("か"), true),
        ("insertCompositionText", Some("かn"), true),
        ("insertCompositionText", Some("かん"), true),
        ("insertCompositionText", Some("かんj"), true),
        ("insertCompositionText", Some("かんじ"), true),
        ("insertCompositionText", Some("漢字"), true),
        ("compositionend", Some("漢字"), false),
    ];
    for (name, steps, committed) in [
        ("zhuyin", zhuyin, "你好"),
        ("pinyin", pinyin, "你好"),
        ("kana", kana, "漢字"),
    ] {
        assert_composition(name, steps, committed);
    }
}

fn assert_composition(name: &str, steps: Steps<'_>, committed: &str) {
    let mut session = Session::with(Doc::from_text("說："));
    session.caret = Caret::at(0, 2);
    for (index, (input_type, data, composing)) in steps.iter().enumerate() {
        let before = session.doc.clone();
        let event = caret_event(input_type, *data, session.caret.pos, *composing);
        let edit = interpret(&session.doc, session.caret, &event).unwrap();
        if *input_type == "compositionend" {
            match edit.ops.as_slice() {
                [Op::Insert { text, at, .. }] => {
                    assert_eq!(text, committed, "{name}");
                    assert_eq!(*at, Pos::new(0, 2), "{name}");
                }
                other => panic!("{name}: compositionend is one Insert, got {other:?}"),
            }
        } else {
            assert!(
                edit.ops.is_empty(),
                "{name}: {input_type} {data:?} produced {:?}; composition must be silent until it ends",
                edit.ops
            );
        }
        session.handle(&event, index as u64 * 30).unwrap();
        if *composing {
            assert_eq!(
                session.doc, before,
                "{name}: {input_type} changed the document"
            );
        }
    }
    assert_eq!(body(&session.doc), format!("說：{committed}"), "{name}");
    assert_eq!(session.caret.pos, Pos::new(0, 4), "{name}");
    session.log.undo(&mut session.doc).unwrap();
    assert_eq!(
        body(&session.doc),
        "說：",
        "{name}: the composition is one undo step"
    );
}

#[test]
fn backspace_deletes_one_grapheme() {
    // Family emoji: man ZWJ woman ZWJ girl ZWJ boy. One cluster, seven scalars, 25 bytes.
    assert_eq!(grapheme_len(FAMILY), 1, "the fixture must be one cluster");
    assert!(FAMILY.len() > 1, "the fixture must be more than one byte");
    let cases = [
        ("family alone", FAMILY.to_owned(), 1, ""),
        ("family after text", format!("a{FAMILY}"), 2, "a"),
        ("family before text", format!("{FAMILY}b"), 1, "b"),
        ("flag", "x🇹🇼".to_owned(), 2, "x"),
        ("combining accent", "ae\u{301}".to_owned(), 2, "a"),
        ("hangul syllable", "한".to_owned(), 1, ""),
    ];
    for (name, text, caret, left) in cases {
        let mut session = Session::with(Doc::from_text(text));
        session.caret = Caret::at(0, caret);
        let event = caret_event("deleteContentBackward", None, session.caret.pos, false);
        session.handle(&event, 0).unwrap();
        assert_eq!(body(&session.doc), left, "{name}");
        assert_eq!(session.caret.pos, Pos::new(0, caret - 1), "{name}");
    }
}

#[test]
fn delete_forward_and_word_backward() {
    let mut session = Session::with(doc_of(vec![
        plain(ParaKind::Paragraph, "ab"),
        plain(ParaKind::Paragraph, "cd"),
    ]));
    session.caret = Caret::at(0, 2);
    let event = caret_event("deleteContentForward", None, session.caret.pos, false);
    session.handle(&event, 0).unwrap();
    assert_eq!(
        body(&session.doc),
        "abcd",
        "Delete at the end joins the next paragraph"
    );

    let mut session = Session::with(Doc::from_text("one two  "));
    session.caret = Caret::at(0, 9);
    let event = caret_event("deleteWordBackward", None, session.caret.pos, false);
    session.handle(&event, 0).unwrap();
    assert_eq!(body(&session.doc), "one ");
}

#[test]
fn format_toggles_over_the_selection() {
    let mut session = Session::with(Doc::from_text("hello"));
    let bold = range_event("formatBold", None, Pos::new(0, 1), Pos::new(0, 4));
    session.handle(&bold, 0).unwrap();
    let Node::Para { runs, .. } = &session.doc.nodes[0] else {
        panic!("paragraph");
    };
    assert_eq!(runs.len(), 3);
    assert!(runs[1].marks.has(Mark::Bold) && runs[1].text == "ell");
    session.handle(&bold, 10).unwrap();
    assert_eq!(
        session.doc,
        Doc::from_text("hello"),
        "a second press clears it"
    );
}

#[test]
fn history_events_undo_and_redo() {
    let mut session = Session::new();
    type_text(&mut session, "ab", 0, 10);
    session
        .handle(
            &caret_event("historyUndo", None, session.caret.pos, false),
            100,
        )
        .unwrap();
    assert_eq!(body(&session.doc), "");
    assert_eq!(
        session.caret.pos,
        Pos::new(0, 0),
        "the caret is kept inside the document"
    );
    session
        .handle(
            &caret_event("historyRedo", None, session.caret.pos, false),
            200,
        )
        .unwrap();
    assert_eq!(body(&session.doc), "ab");
}

#[test]
fn undo_groups_words_and_paste() {
    let mut session = Session::new();
    type_text(&mut session, "ab cd", 0, 100);
    assert_eq!(body(&session.doc), "ab cd");
    session.log.undo(&mut session.doc).unwrap();
    assert_eq!(body(&session.doc), "ab ", "one undo drops the second word");
    session.log.undo(&mut session.doc).unwrap();
    assert_eq!(body(&session.doc), "", "the next undo drops the first word");

    let mut sentence = Session::new();
    type_text(&mut sentence, "the cat sat", 0, 50);
    assert_eq!(sentence.log.group_count(), 3, "one group per word");

    let mut paused = Session::new();
    type_text(&mut paused, "ab", 0, 100);
    type_text(&mut paused, "c", 100 + PAUSE_MS + 1, 10);
    paused.log.undo(&mut paused.doc).unwrap();
    assert_eq!(
        body(&paused.doc),
        "ab",
        "a pause over 600 ms starts a group"
    );

    let mut pasted = Session::new();
    type_text(&mut pasted, "Hello", 0, 10);
    let paste = caret_event("insertFromPaste", Some("X\n\nY"), pasted.caret.pos, false);
    pasted.handle(&paste, 60).unwrap();
    assert_eq!(body(&pasted.doc), "Hello|X|Y");
    pasted.log.undo(&mut pasted.doc).unwrap();
    assert_eq!(body(&pasted.doc), "Hello", "a paste is one group");
}

#[test]
fn text_typed_after_an_inline_shortcut_is_not_marked() {
    let mut session = Session::new();
    type_text(&mut session, "**hi** there", 0, 10);
    let Node::Para { runs, .. } = &session.doc.nodes[0] else {
        panic!("paragraph");
    };
    assert_eq!(runs_text(runs), "hi there");
    assert_eq!(runs[0].text, "hi");
    assert!(runs[0].marks.has(Mark::Bold));
    assert_eq!(runs[1].text, " there");
    assert!(!runs[1].marks.has(Mark::Bold), "{runs:?}");

    // Undo takes the shortcut back first: the markers return.
    let mut session = Session::new();
    type_text(&mut session, "# ", 0, 10);
    assert_eq!(kinds(&session.doc), "h1");
    session.log.undo(&mut session.doc).unwrap();
    assert_eq!(
        (kinds(&session.doc).as_str(), body(&session.doc).as_str()),
        ("p", "# ")
    );
}

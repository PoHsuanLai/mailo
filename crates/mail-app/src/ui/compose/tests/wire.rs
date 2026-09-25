//! Recorded glue messages, replayed through the page's handler into documents and markup.

use super::super::float::{pick_mention, pick_slash};
use super::super::page::{CcRow, Float};
use super::super::render::{para_renders, reset_para_renders};
use super::*;
use crate::editor::{Level, Mark, ParaKind, to_html};

#[test]
fn the_glue_messages_parse() {
    use super::super::wire::Heard;
    let select = parse(r#"{"t":"select","ranges":[[1,4,0,2]]}"#);
    let Some(Heard::Select(range)) = select else {
        panic!("a select: {select:?}");
    };
    assert_eq!(
        (range.start, range.end),
        (Pos::new(0, 2), Pos::new(1, 4)),
        "ordered"
    );

    let typed = parse(&message(7, "insertText", Some("a"), &[[0, 1, 0, 1]], None));
    let Some(Heard::Input {
        seq,
        event,
        selection,
    }) = typed
    else {
        panic!("an input: {typed:?}");
    };
    assert_eq!(
        (seq, event.input_type.as_str(), selection),
        (7, "insertText", None)
    );

    for junk in ["", "{}", "not json", r#"{"t":"insertText"}"#] {
        assert_eq!(
            parse(junk),
            None,
            "{junk:?} is not a message the glue writes"
        );
    }
}

#[test]
fn typed_markdown_becomes_a_bold_run_and_the_markup_shows_it() {
    let (mut dom, mut page) = body_dom(page_of(""));
    dom.in_runtime(|| type_text(&mut page.write(), "hello **world**"));
    dom.render_immediate(&mut NoOpMutations);

    let read = dom.in_runtime(|| page.peek().clone());
    let Node::Para { runs, .. } = &read.session.doc.nodes[0] else {
        panic!("a paragraph: {:?}", read.session.doc);
    };
    let shape: Vec<(&str, bool)> = runs
        .iter()
        .map(|run| (run.text.as_str(), run.marks.has(Mark::Bold)))
        .collect();
    assert_eq!(shape, [("hello ", false), ("world", true)]);
    assert_eq!(
        to_html(&read.session.doc),
        "<p>hello <strong>world</strong></p>"
    );

    let markup = dioxus_ssr::render(&dom);
    #[cfg(feature = "webview")]
    {
        assert!(
            markup.contains(
                r#"data-n="0"><span class="">hello </span><span class="m-b">world</span>"#
            ),
            "the paragraph is not drawn from the doc:\n{markup}"
        );
        assert!(
            markup.contains(r#"data-seq="15""#),
            "the last message's number is echoed:\n{markup}"
        );
    }
    // On Blitz the paragraph is the surface's node 0, and there is no glue to echo to.
    #[cfg(feature = "native")]
    assert!(
        markup.contains(
            r#"data-edit-node="0"><span class="">hello </span><span class="m-b">world</span>"#
        ),
        "the paragraph is not drawn from the doc:\n{markup}"
    );
}

#[test]
fn a_page_that_is_behind_types_at_rusts_caret() {
    // The glue sends no ranges while the page has not drawn its last message: its positions
    // would be old ones. Rust's own caret carries on.
    let mut page = page_of("");
    feed(
        &mut page,
        &message(
            1,
            "insertText",
            Some("a"),
            &[[0, 0, 0, 0]],
            Some([0, 0, 0, 0]),
        ),
        0,
    );
    feed(
        &mut page,
        &message(2, "insertText", Some("b"), &[], None),
        10,
    );
    feed(
        &mut page,
        &message(3, "insertText", Some("c"), &[], None),
        20,
    );
    assert_eq!(body(&page), "abc");
    assert_eq!(page.wire.seq, 3);
}

#[test]
fn enter_backspace_and_a_cross_paragraph_delete() {
    let mut page = page_of("");
    type_text(&mut page, "one");
    feed(
        &mut page,
        &message(
            4,
            "insertParagraph",
            None,
            &[[0, 3, 0, 3]],
            Some([0, 3, 0, 3]),
        ),
        200,
    );
    type_text(&mut page, "two");
    assert_eq!(body(&page), "one|two");

    // The browser's target range for a Backspace at a paragraph's start spans the break; the
    // collapsed selection is what Rust resolves, and it joins the two.
    feed(
        &mut page,
        &message(
            8,
            "deleteContentBackward",
            None,
            &[[0, 3, 1, 0]],
            Some([1, 0, 1, 0]),
        ),
        500,
    );
    assert_eq!(body(&page), "onetwo");

    // A selection across what was two paragraphs goes in one delete.
    type_text(&mut page, "");
    feed(
        &mut page,
        &message(
            9,
            "insertParagraph",
            None,
            &[[0, 3, 0, 3]],
            Some([0, 3, 0, 3]),
        ),
        600,
    );
    feed(
        &mut page,
        &message(
            10,
            "deleteContentBackward",
            None,
            &[[0, 1, 1, 2]],
            Some([0, 1, 1, 2]),
        ),
        700,
    );
    assert_eq!(body(&page), "oo");
}

#[test]
fn a_composition_leaves_its_paragraph_alone_until_it_ends() {
    let (mut dom, mut page) = body_dom(page_of("ab\n\nsecond"));
    dom.in_runtime(|| page.write().session.caret = crate::editor::Caret::at(0, 2));
    dom.render_immediate(&mut NoOpMutations);
    reset_para_renders();
    let key_before = dom.in_runtime(|| page.peek().node_key(0));

    // Pinyin: the IME writes into the paragraph; the glue forwards only the start and the end.
    // A stray event during the composition must change nothing either.
    let during = [
        message(1, "compositionstart", None, &[], Some([0, 2, 0, 2])),
        message(
            2,
            "insertText",
            Some("n"),
            &[[0, 2, 0, 2]],
            Some([0, 3, 0, 3]),
        ),
        message(3, "deleteContentBackward", None, &[], Some([0, 3, 0, 3])),
    ];
    for raw in &during {
        dom.in_runtime(|| feed(&mut page.write(), raw, 0));
        dom.render_immediate(&mut NoOpMutations);
    }
    let (doc_during, seq) = dom.in_runtime(|| (body(&page.peek()), page.peek().wire.seq));
    assert_eq!(
        doc_during, "ab|second",
        "the document changed mid-composition"
    );
    assert_eq!(seq, 3, "every message is still acknowledged");
    assert_eq!(
        para_renders(0),
        0,
        "Rust redrew the paragraph the IME was writing in"
    );

    dom.in_runtime(|| {
        feed(
            &mut page.write(),
            &message(4, "compositionend", Some("你好"), &[], None),
            0,
        )
    });
    dom.render_immediate(&mut NoOpMutations);
    let (doc_after, caret, key_after) = dom.in_runtime(|| {
        let read = page.peek();
        (body(&read), read.session.caret.pos, read.node_key(0))
    });
    assert_eq!(doc_after, "ab你好|second");
    assert_eq!(caret, Pos::new(0, 4));
    assert_ne!(
        key_after, key_before,
        "the composed paragraph keeps the IME's DOM"
    );
    assert_eq!(
        para_renders(0),
        1,
        "the paragraph is built once, fresh, at compositionend"
    );
    assert_eq!(para_renders(1), 0, "the other paragraph was never touched");
}

#[test]
fn slash_opens_a_menu_that_turns_the_line_into_a_heading() {
    let mut page = page_of("");
    type_text(&mut page, "/hea");
    assert!(
        matches!(page.float, Float::Slash { .. }),
        "{:?}",
        page.float
    );
    assert_eq!(super::super::float::query(&page).as_deref(), Some("hea"));
    let first = super::super::float::slash_items("hea");
    assert_eq!(
        first.first().map(|item| item.name.as_str()),
        Some("Heading 1")
    );

    pick_slash(&mut page, "Heading 1", "Wednesday 23 September 2026");
    assert_eq!(page.float, Float::Closed);
    assert_eq!(body(&page), "", "the typed /hea is gone");
    assert!(matches!(
        page.session.doc.nodes[0],
        Node::Para {
            kind: ParaKind::Heading(Level::One),
            ..
        }
    ));

    // A slash inside a word is a slash.
    let mut page = page_of("");
    type_text(&mut page, "and/or");
    assert_eq!(page.float, Float::Closed);
}

#[test]
fn slash_divider_puts_an_object_in_place_of_the_empty_line() {
    let mut page = page_of("");
    type_text(&mut page, "/div");
    pick_slash(&mut page, "Divider", "");
    assert_eq!(body(&page), "[object]|");
    assert_eq!(
        page.session.caret.pos,
        Pos::new(1, 0),
        "the caret is on the line after it"
    );
}

#[test]
fn a_mention_adds_the_person_to_cc_and_the_chip_flashes() {
    let mut page = page_of("");
    type_text(&mut page, "ask @da");
    assert!(
        matches!(page.float, Float::Mention { .. }),
        "{:?}",
        page.float
    );
    let before = page.cc.len();
    pick_mention(&mut page, "dana@example.test");
    assert_eq!(body(&page), "ask @Dana Whitfield ");
    assert_eq!(page.cc.len(), before + 1);
    assert_eq!(page.cc_row, CcRow::Shown, "the Cc row appears");
    assert_eq!(page.flash.as_deref(), Some("dana@example.test"));

    // Someone already on the message is not added twice.
    type_text(&mut page, "and @da");
    pick_mention(&mut page, "dana@example.test");
    assert_eq!(page.cc.len(), before + 1);
}

//! Lane 2: formatting in the composer as a modern editor (Notion, Bear, Apple Notes) does it,
//! driven only by typing, the keyboard, the selection bubble, the `/` and `@` menus and paste.

use ds_harness::{Driver, Query};

use super::body::{composing, outline, run_is, select_back};
use super::drive::{Drive, Key};
use super::window::{Window, parsed, queued};

/// The bubble's mark segments, in order: Bold, Italic, Underline, Strike, Code.
fn mark_segment(n: usize) -> String {
    format!(".bubble .ds-segmented-segment:nth-child({n})")
}

/// Type the markdown blocks every lane here starts from.
fn type_blocks(window: &mut Window) {
    window.type_text("# Plan\n");
    window.type_text("- a\nb\n\n");
    window.type_text("1. one\ntwo\n\n");
    window.type_text("[ ] call\n\n");
    window.type_text("> wise words\n");
    window.type_text("after");
}

/// What [`type_blocks`] gives.
fn typed_blocks() -> Vec<String> {
    [
        "h2 Plan",
        "ul [a | b]",
        "ol [one | two]",
        "ul.todo [call]",
        "blockquote wise words",
        "p after",
    ]
    .map(str::to_owned)
    .to_vec()
}

#[test]
fn markdown_typed_at_a_line_start_makes_headings_lists_to_dos_and_quotes_and_enter_leaves_them() {
    let mut window = composing();
    window.type_text("# Plan\n");
    assert_eq!(outline(&window.harness), ["h2 Plan", "p"], "after # Plan");
    window.type_text("- a\nb\n");
    assert_eq!(
        outline(&window.harness),
        ["h2 Plan", "ul [a | b | ]"],
        "a third bullet waits after b"
    );
    window.type_text("\n");
    assert_eq!(
        outline(&window.harness),
        ["h2 Plan", "ul [a | b]", "p"],
        "Enter on an empty bullet leaves the list"
    );
    window.type_text("1. one\ntwo\n\n");
    window.type_text("[ ] call\n\n");
    window.type_text("> wise words\n");
    window.type_text("after");
    assert_eq!(outline(&window.harness), typed_blocks());
}

#[test]
fn a_selection_made_with_shift_arrows_or_a_drag_takes_bold_italic_and_underline_from_the_keys() {
    let mut window = composing();
    window.type_text("a bold move");
    select_back(&mut window, 4);
    window.press(&[Key::Ctrl], Key::Char('b'), 1);
    window.until("Ctrl-B bolds it", |h| run_is(h, ".m-b", "move"));
    window.press(&[Key::Ctrl], Key::Char('i'), 1);
    window.until("Ctrl-I italicises it", |h| run_is(h, ".m-b.m-i", "move"));
    window.press(&[Key::Ctrl], Key::Char('u'), 1);
    window.until("Ctrl-U underlines it", |h| {
        run_is(h, ".m-b.m-i.m-u", "move")
    });
    assert_eq!(outline(&window.harness), ["p a bold move"]);

    // A drag selects too: across "bold", then Ctrl-B.
    let (start, end) = word_ends(&window, "bold");
    window.harness.drag(start, end, 6);
    window.harness.advance(super::window::ms(100));
    window.press(&[Key::Ctrl], Key::Char('b'), 1);
    window.until("the dragged word is bold", |h| {
        h.text_of(".c-body .m-b")
            .is_some_and(|run| run.starts_with("bold"))
    });
}

#[test]
#[ignore = "gap(quire): a press on a SegmentedControl segment takes the keyboard from the ds-edit body (click_focus press exempts fields, not ds-edit), so the next Ctrl-I goes nowhere"]
fn a_selection_bolded_from_the_bubble_keeps_the_keyboard_for_ctrl_i() {
    let mut window = composing();
    window.type_text("a bold move");
    select_back(&mut window, 4);
    window.until("the bubble is over the selection", |h| {
        h.count(&mark_segment(1)) == 1
    });
    window.click(&mark_segment(1));
    window.until("the bubble's Bold bolds it", |h| run_is(h, ".m-b", "move"));
    assert!(
        window.harness.is_focused(".c-body"),
        "the bubble took the keyboard from the body"
    );
    window.press(&[Key::Ctrl], Key::Char('i'), 1);
    window.until("Ctrl-I italicises it", |h| run_is(h, ".m-b.m-i", "move"));
}

/// Where a drag over `word` in the body's only paragraph starts and ends: the paragraph's text
/// is laid out in one line, so the word's ends are found by its letters' share of the line.
fn word_ends(window: &Window, word: &str) -> (ds::prelude::Point, ds::prelude::Point) {
    let line = window
        .harness
        .rect(".c-body > p")
        .expect("the paragraph is drawn");
    let text = window.text(".c-body > p");
    let start = text.find(word).expect("the word is in the paragraph");
    let rects: Vec<_> = (1..=window.harness.count(".c-body > p > span"))
        .filter_map(|n| {
            window
                .harness
                .rect(&format!(".c-body > p > span:nth-child({n})"))
        })
        .collect();
    let left = rects.first().map_or(line.origin.x.0, |r| r.origin.x.0);
    let right = rects
        .last()
        .map_or(line.origin.x.0, |r| r.origin.x.0 + r.size.width.0);
    let per = (right - left) / text.chars().count() as f32;
    let y = rects.first().map_or(line.origin.y.0 + 8.0, |r| {
        r.origin.y.0 + r.size.height.0 / 2.0
    });
    let at = |chars: usize| ds::prelude::Point {
        x: ds::prelude::Px(left + per * chars as f32),
        y: ds::prelude::Px(y),
    };
    (at(start), at(start + word.chars().count()))
}

#[test]
fn backticks_stars_and_underscores_close_into_code_bold_and_italic() {
    let mut window = composing();
    window.type_text("run `cargo` now, **really** and _gently_ ok");
    assert!(run_is(&window.harness, ".m-code", "cargo"), "no code run");
    assert!(run_is(&window.harness, ".m-b", "really"), "no bold run");
    assert!(run_is(&window.harness, ".m-i", "gently"), "no italic run");
    assert_eq!(
        outline(&window.harness),
        ["p run cargo now, really and gently ok"]
    );
}

#[test]
fn the_slash_menu_turns_a_line_into_a_quote_and_a_code_block() {
    let mut window = composing();
    window.type_text("/quo");
    window.until("/ opens its menu", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("Quote"))
    });
    window.press(&[], Key::Enter, 1);
    window.until("the menu goes", |h| h.count(".ds-menu") == 0);
    window.type_text("Measure twice\n");
    window.type_text("/code");
    window.until("/ opens its menu again", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("Code"))
    });
    window.press(&[], Key::Enter, 1);
    window.until("the menu goes", |h| h.count(".ds-menu") == 0);
    window.type_text("cargo test");
    assert_eq!(
        outline(&window.harness),
        ["blockquote Measure twice", "pre cargo test"]
    );
}

#[test]
fn an_at_sign_puts_a_person_in_cc_and_a_colon_name_puts_an_emoji_in() {
    let mut window = composing();
    window.type_text("thanks @grac");
    window.until("@ offers Grace", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("Grace"))
    });
    window.press(&[], Key::Enter, 1);
    window.until("Grace is in Cc", |h| {
        h.text_of(".c-props [*|data-row=cc]")
            .is_some_and(|cc| cc.contains("Grace Hopper"))
    });
    window.type_text("for it :smi");
    window.until(":smi offers emoji", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("smiling face"))
    });
    window.press(&[], Key::Enter, 1);
    window.until("the emoji is in", |h| {
        h.text_of(".c-body > p")
            .is_some_and(|text| text.starts_with("thanks @Grace Hopper for it 😊"))
    });
}

#[test]
fn backspace_at_the_start_of_a_heading_or_a_list_item_unstyles_it_before_merging() {
    let mut window = composing();
    window.type_text("intro\n# Title");
    window.press(&[], Key::Home, 1);
    window.press(&[], Key::Backspace, 1);
    assert_eq!(
        outline(&window.harness),
        ["p intro", "p Title"],
        "the heading became a paragraph"
    );
    window.press(&[], Key::Backspace, 1);
    assert_eq!(outline(&window.harness), ["p introTitle"], "then it merged");

    window.press(&[], Key::End, 1);
    window.type_text("\n- item");
    window.press(&[], Key::Home, 1);
    window.press(&[], Key::Backspace, 1);
    assert_eq!(
        outline(&window.harness),
        ["p introTitle", "p item"],
        "the bullet became a paragraph"
    );
    window.press(&[], Key::Backspace, 1);
    assert_eq!(outline(&window.harness), ["p introTitleitem"]);
}

#[test]
fn undo_walks_the_whole_session_back_and_redo_walks_it_forward() {
    let mut window = composing();
    type_blocks(&mut window);
    select_back(&mut window, 5);
    window.press(&[Key::Ctrl], Key::Char('b'), 1);
    assert!(run_is(&window.harness, ".m-b", "after"));
    let done = outline(&window.harness);

    // Undo until the body is empty again, then redo it all, both ways of asking for it.
    for _ in 0..60 {
        if outline(&window.harness) == ["p"] {
            break;
        }
        window.press(&[Key::Ctrl], Key::Char('z'), 1);
    }
    assert_eq!(
        outline(&window.harness),
        ["p"],
        "undo did not reach the start"
    );
    for _ in 0..60 {
        if outline(&window.harness) == done && run_is(&window.harness, ".m-b", "after") {
            break;
        }
        window.press(&[Key::Ctrl, Key::Shift], Key::Char('z'), 1);
    }
    assert_eq!(
        outline(&window.harness),
        done,
        "Ctrl-Shift-Z did not redo it all"
    );
    assert!(
        run_is(&window.harness, ".m-b", "after"),
        "the bold was not redone"
    );

    window.press(&[Key::Ctrl], Key::Char('z'), 1);
    assert_eq!(
        window.harness.count(".c-body .m-b"),
        0,
        "undo left the bold"
    );
    window.press(&[Key::Ctrl], Key::Char('y'), 1);
    assert!(
        run_is(&window.harness, ".m-b", "after"),
        "Ctrl-Y did not redo"
    );
}

/// Clipboard HTML as a word processor in a browser writes it: a wrapper that says "not bold",
/// styled spans for the marks, a nested list and a link.
const DOCS: &str = concat!(
    r#"<meta charset="utf-8"><b style="font-weight:normal;" id="docs-internal-guid-1">"#,
    r#"<h1 dir="ltr"><span style="font-size:20pt;font-weight:400;">Agenda</span></h1>"#,
    r#"<p dir="ltr"><span style="font-weight:400;">Bring </span>"#,
    r#"<span style="font-weight:700;">the numbers</span>"#,
    r#"<span style="font-weight:400;"> and </span>"#,
    r#"<span style="font-style:italic;font-weight:400;">a pen</span>"#,
    r#"<span style="font-weight:400;">, see </span>"#,
    r#"<a href="https://example.test/plan?utm_source=docs"><span style="color:#1155cc;text-decoration:underline;">the plan</span></a></p>"#,
    r#"<ul><li dir="ltr"><p dir="ltr"><span>Budget</span></p></li>"#,
    r#"<ul><li dir="ltr"><p dir="ltr"><span>Travel</span></p></li></ul>"#,
    r#"<li dir="ltr"><p dir="ltr"><span>Hiring</span></p></li></ul></b>"#,
);

#[test]
fn rich_html_pasted_from_a_word_processor_keeps_its_structure_spaces_and_link() {
    let mut window = composing();
    window.harness.paste_html(
        DOCS,
        "Agenda\nBring the numbers and a pen, see the plan\nBudget\nTravel\nHiring",
    );
    window.until("the paste lands", |h| h.count(".c-body > h2") == 1);
    assert_eq!(
        outline(&window.harness),
        [
            "h2 Agenda",
            "p Bring the numbers and a pen, see the plan",
            "ul [Budget | Travel | Hiring]",
        ]
    );
    assert_eq!(
        window.harness.count(".c-body .m-b"),
        0,
        "the not-bold wrapper made something bold"
    );
    assert_eq!(
        window.harness.attr(".c-body .m-a", "href").as_deref(),
        Some("https://example.test/plan"),
        "the link keeps its address, without the tracking parameter"
    );
}

#[test]
fn a_formatted_message_goes_out_as_html_and_as_text_that_reads_naturally() {
    let mut window = composing();
    type_blocks(&mut window);
    window.type_text(" with **bold**, _soft_, `code`");
    window.type_text("\nunder");
    select_back(&mut window, 5);
    window.press(&[Key::Ctrl], Key::Char('u'), 1);
    window.press(&[], Key::End, 1);
    window.click(".c-props [*|data-row=to] .c-pin input");
    window.type_text("ada@example.test\n");
    window.click(".c-title input");
    window.type_text("Plan");
    window.click(".c-foot [*|aria-label=\"Send\"]");
    window.until("the send pill shows", |h| h.count(".ds-send-pill") == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    let mail = parsed(&sent[0].raw);

    let html = mail.html.unwrap_or_default();
    for tag in [
        "<h1>Plan</h1>",
        "<ul><li><p>a</p></li><li><p>b</p></li></ul>",
        "<ol><li><p>one</p></li><li><p>two</p></li></ol>",
        "<blockquote><p>wise words</p></blockquote>",
        "<strong>bold</strong>",
        "<em>soft</em>",
        "<code>code</code>",
        "<u>under</u>",
    ] {
        assert!(html.contains(tag), "no {tag} in the HTML part:\n{html}");
    }
    let text = mail.text.unwrap_or_default();
    for line in [
        "Plan",
        "- a",
        "- b",
        "1. one",
        "2. two",
        "[ ] call",
        "> wise words",
    ] {
        assert!(
            text.lines().any(|l| l.trim_end() == line),
            "no line {line:?} in the text part:\n{text}"
        );
    }
}

#[test]
#[ignore = "gap: the paste sanitizer drops style, so a word processor's font-weight:700 and font-style:italic spans paste as plain text"]
fn styled_spans_pasted_from_a_word_processor_keep_their_bold_and_italic() {
    let mut window = composing();
    window
        .harness
        .paste_html(DOCS, "Agenda\nBring the numbers and a pen, see the plan");
    window.until("the paste lands", |h| h.count(".c-body > h2") == 1);
    assert!(
        run_is(&window.harness, ".m-b", "the numbers"),
        "the styled bold span is not bold"
    );
    assert!(
        run_is(&window.harness, ".m-i", "a pen"),
        "the styled italic span is not italic"
    );
}

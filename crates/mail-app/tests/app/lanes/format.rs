//! Lane 2: formatting in the composer as a modern editor (Notion, Bear, Apple Notes) does it,
//! driven only by typing, the keyboard, the selection bubble, the `/` and `@` menus and paste.

use super::body::{composing, outline, run_is, select_back};
use super::drive::{Drive, Key, PRIMARY};
use super::window::{Window, ms, parsed, queued};

/// The bubble's mark segments, in order: Bold, Italic, Underline, Strike, Code.
fn mark_segment(n: usize) -> String {
    format!(".bubble .ds-segmented-segment:nth-child({n})")
}

/// Press `key` with `held` until `done` holds.
fn press_until(window: &mut Window, held: &[Key], key: Key, done: impl Fn(&Window) -> bool) {
    for _ in 0..80 {
        if done(window) {
            return;
        }
        window.harness.chord(held, key);
        window.harness.advance(ms(16));
    }
    window.harness.advance(ms(100));
    assert!(
        done(window),
        "{key:?} never got there: {:?}",
        outline(&window.harness)
    );
}

/// Markdown typed at a line start makes headings, lists, to-dos and quotes, and Enter on an
/// empty item leaves them; backticks, stars and underscores close into code, bold and italic.
/// Undo walks the whole session back to an empty body and redo walks it forward, both ways of
/// asking for it; then the message goes out as HTML and as text that reads naturally.
#[test]
fn a_message_formatted_by_typing_undoes_and_redoes_whole_and_goes_out_as_html_and_text() {
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
    window.type_text("after `cargo` now, **bold**, _soft_ ok");
    assert!(run_is(&window.harness, ".m-code", "cargo"), "no code run");
    assert!(run_is(&window.harness, ".m-b", "bold"), "no bold run");
    assert!(run_is(&window.harness, ".m-i", "soft"), "no italic run");
    window.type_text("\nunder");
    select_back(&mut window, 5);
    window.press(&[PRIMARY], Key::Char('u'), 1);
    let done = [
        "h2 Plan",
        "ul [a | b]",
        "ol [one | two]",
        "ul.todo [call]",
        "blockquote wise words",
        "p after cargo now, bold, soft ok",
        "p under",
    ];
    assert_eq!(outline(&window.harness), done);
    assert!(run_is(&window.harness, ".m-u", "under"), "no underline");

    // Undo to the empty body, redo it all, then one step each way.
    press_until(&mut window, &[PRIMARY], Key::Char('z'), |w| {
        outline(&w.harness) == ["p"]
    });
    press_until(&mut window, &[PRIMARY, Key::Shift], Key::Char('z'), |w| {
        outline(&w.harness) == done && run_is(&w.harness, ".m-u", "under")
    });
    window.press(&[PRIMARY], Key::Char('z'), 1);
    assert_eq!(
        window.harness.count(".c-body .m-u"),
        0,
        "undo left the underline"
    );
    window.press(&[PRIMARY, Key::Shift], Key::Char('z'), 1);
    assert!(
        run_is(&window.harness, ".m-u", "under"),
        "redo did not redo"
    );

    window.press(&[], Key::End, 1);
    window.click(".c-props [*|data-row=to] .c-pin input");
    window.type_text("ada@example.test\n");
    window.click(".c-title input");
    window.type_text("Plan");
    window.click(&super::look::send());
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
        "<code>cargo</code>",
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

/// A selection made with Shift and the arrows takes Bold from the bubble, which leaves the
/// keyboard with the body for Ctrl-I and Ctrl-U; a drag selects as well. Backspace at the start
/// of a heading or a list item unstyles it before it merges.
#[test]
fn a_selection_takes_marks_from_the_bubble_and_the_keys_and_backspace_unstyles_before_merging() {
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
    window.press(&[PRIMARY], Key::Char('i'), 1);
    window.until("Ctrl-I italicises it", |h| run_is(h, ".m-b.m-i", "move"));
    window.press(&[PRIMARY], Key::Char('u'), 1);
    window.until("Ctrl-U underlines it", |h| {
        run_is(h, ".m-b.m-i.m-u", "move")
    });
    assert_eq!(outline(&window.harness), ["p a bold move"]);

    // A drag selects too: across "bold", then Ctrl-B.
    let (start, end) = word_ends(&window, "bold");
    window.harness.drag(start, end, 6);
    window.harness.advance(ms(100));
    window.press(&[PRIMARY], Key::Char('b'), 1);
    window.until("the dragged word is bold", |h| {
        h.text_of(".c-body .m-b")
            .is_some_and(|run| run.starts_with("bold"))
    });

    window.press(&[], Key::End, 1);
    window.type_text("\n# Title");
    window.press(&[], Key::Home, 1);
    window.press(&[], Key::Backspace, 1);
    assert_eq!(
        outline(&window.harness),
        ["p a bold move", "p Title"],
        "the heading became a paragraph"
    );
    window.press(&[], Key::Backspace, 1);
    assert_eq!(
        outline(&window.harness),
        ["p a bold moveTitle"],
        "then it merged"
    );
    window.press(&[], Key::End, 1);
    window.type_text("\n- item");
    window.press(&[], Key::Home, 1);
    window.press(&[], Key::Backspace, 1);
    assert_eq!(
        outline(&window.harness),
        ["p a bold moveTitle", "p item"],
        "the bullet became a paragraph"
    );
    window.press(&[], Key::Backspace, 1);
    assert_eq!(outline(&window.harness), ["p a bold moveTitleitem"]);
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

/// The `/` menu turns a line into a quote and a code block, and `@` puts a person in Cc.
#[test]
fn the_slash_menu_makes_a_quote_and_a_code_block_and_an_at_sign_puts_a_person_in_cc() {
    let mut window = composing();
    let pick = |window: &mut Window, what: &str| {
        window.until(what, |h| h.count(".ds-menu .ds-menu-item") > 0);
        window.press(&[], Key::Enter, 1);
        window.until("the menu goes", |h| h.count(".ds-menu") == 0);
    };
    window.type_text("thanks @grac");
    pick(&mut window, "@ offers the contact");
    window.type_text("\n/quo");
    pick(&mut window, "/ opens its menu");
    window.type_text("Measure twice\n");
    window.type_text("/code");
    pick(&mut window, "/ opens its menu again");
    window.type_text("cargo test");
    assert_eq!(
        outline(&window.harness)[1..],
        ["blockquote Measure twice", "pre cargo test"]
    );
    window.until("Grace is in Cc", |h| {
        h.text_of(".c-props [*|data-row=cc]")
            .is_some_and(|cc| cc.contains("Grace Hopper"))
    });
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
        1,
        "the not-bold wrapper made more than the bold span bold"
    );
    assert_eq!(
        window.harness.attr(".c-body .m-a", "href").as_deref(),
        Some("https://example.test/plan"),
        "the link keeps its address, without the tracking parameter"
    );
}

#[test]
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

/// What an input method commits is written, and nothing else. A zhuyin composition is drawn at
/// the caret as its preedit and leaves the body untouched until the commit, a key pressed while
/// it composes is the IME's, and typing after the commit is text again; then a kana to kanji
/// conversion after typed text. The IME's candidate window stands at the caret.
#[test]
fn an_input_method_writes_its_commit_and_nothing_else_and_its_window_stands_at_the_caret() {
    let mut window = composing();

    // Zhuyin.
    window.harness.ime_start();
    window.harness.ime_update("ㄓ", "ㄓ".len());
    window.harness.ime_update("ㄓㄨ", "ㄓㄨ".len());
    window.until("the zhuyin preedit is drawn", |h| {
        h.text_of(".c-preedit").as_deref() == Some("ㄓㄨ")
    });
    assert_eq!(
        outline(&window.harness),
        ["p"],
        "the preedit touched the body"
    );
    // A key while the IME composes is the IME's.
    window.harness.key(Key::Char('x'));
    // winit clears the preedit (an empty update), then commits.
    window.harness.ime_commit("注");
    window.until("the zhuyin commit is written", |h| outline(h) == ["p 注"]);
    assert_eq!(
        window.harness.count(".c-preedit"),
        0,
        "the preedit outlived the commit"
    );
    window.type_text("a");
    assert_eq!(
        outline(&window.harness),
        ["p 注a"],
        "typing after the commit"
    );

    // The candidate window follows the caret.
    window.harness.advance(ms(100));
    let caret = window.harness.rect(".c-caret").expect("no caret is drawn");
    let area = window
        .harness
        .ime_cursor_area()
        .expect("the IME has no area");
    assert!(
        (area.origin.x.0 - caret.origin.x.0).abs() <= 1.0
            && (area.origin.y.0 - caret.origin.y.0).abs() <= 1.0,
        "the IME's area {area:?} is not at the caret {caret:?}"
    );

    // Kana to kanji, after typed text.
    window.type_text(" ");
    window.harness.ime_start();
    for preedit in ["に", "にほ", "にほん", "日本"] {
        window.harness.ime_update(preedit, preedit.len());
    }
    window.until("the kana preedit is drawn", |h| {
        h.text_of(".c-preedit").as_deref() == Some("日本")
    });
    assert_eq!(
        outline(&window.harness),
        ["p 注a "],
        "the kana preedit touched the body"
    );
    window.harness.ime_commit("日本");
    window.harness.ime_end();
    window.until("the kanji commit is written", |h| {
        outline(h) == ["p 注a 日本"]
    });
}

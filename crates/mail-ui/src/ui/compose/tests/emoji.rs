//! The `:` menu and the picker's insertion, on the page model: when the menu opens and closes,
//! what it lists, and that a pick is one undo step in both the HTML and the plain text.

use super::super::float::{emoji_items, insert_emoji, pick_emoji};
use super::super::page::Float;
use super::*;
use mail_core::editor::to_flowed;

fn undo(page: &mut Page) {
    let event = InputEvent::new("historyUndo", None, Vec::new(), false);
    page.session
        .handle(&event, 1_000_000)
        .unwrap_or_else(|why| panic!("undo: {why}"));
}

#[test]
fn a_colon_and_a_name_open_the_menu_and_a_pick_is_one_undo_step() {
    let mut page = page_of("");
    type_text(&mut page, "Hi :s");
    assert_eq!(page.float, Float::Closed, "one letter is not a name yet");
    type_text(&mut page, "mi");
    assert_eq!(
        page.float,
        Float::Emoji {
            anchor: Pos::new(0, 3),
            active: 0
        }
    );
    let items = emoji_items(&page);
    assert_eq!(
        items.first().map(|item| item.key.as_str()),
        Some("😊"),
        "the first name that starts with it"
    );
    assert!(items.len() <= super::super::items::EMOJI_ROWS);

    let picked = pick_emoji(&mut page, "😊").map(|emoji| emoji.name);
    assert_eq!(picked, Some("smiling face with smiling eyes"));
    assert_eq!(page.float, Float::Closed);
    assert_eq!(body(&page), "Hi 😊", "the typed :smi is gone");
    assert_eq!(
        page.session.caret.pos,
        Pos::new(0, 4),
        "the caret is after it"
    );
    assert_eq!(
        to_flowed(&page.session.doc).trim_end(),
        "Hi 😊",
        "the plain text has it too"
    );

    undo(&mut page);
    assert_eq!(body(&page), "Hi :smi", "one step back is the name as typed");
}

#[test]
fn the_colon_menu_closes_when_the_name_ends_or_finds_nothing() {
    let cases: &[(&str, &str)] = &[
        (":smi ", "a space ends the name"),
        (":smizzz", "a name that finds nothing"),
        (":)", "a smiley"),
        ("Note:smi", "a colon inside a word"),
        ("10:30", "a time"),
    ];
    for (typed, why) in cases {
        let mut page = page_of("");
        type_text(&mut page, typed);
        assert_eq!(page.float, Float::Closed, "{typed:?}: {why}");
        assert_eq!(body(&page), *typed, "{typed:?} is typed as it is");
    }
    // Back into the name, the menu follows it; the caret leaving it closes it.
    let mut page = page_of("");
    type_text(&mut page, ":cat");
    assert!(matches!(page.float, Float::Emoji { .. }));
    page.session.caret = mail_core::editor::Caret::at(0, 1);
    super::super::float::after_move(&mut page);
    assert_eq!(page.float, Float::Closed, "the caret left the name");
}

#[test]
fn the_picker_puts_an_emoji_at_the_caret_or_over_the_selection_as_one_step() {
    let cat = crate::ui::emoji::find("🐱").unwrap_or_else(|| panic!("🐱 is in the table"));
    let mut page = page_of("");
    type_text(&mut page, "a cat");
    page.session.caret = mail_core::editor::Caret::at(0, 1);
    assert!(insert_emoji(&mut page, cat));
    assert_eq!(body(&page), "a🐱 cat");
    assert_eq!(page.session.caret.pos, Pos::new(0, 2));
    undo(&mut page);
    assert_eq!(body(&page), "a cat");

    page.selection = Some(range([0, 2, 0, 5]));
    assert!(insert_emoji(&mut page, cat));
    assert_eq!(body(&page), "a 🐱", "the selection is replaced");
    assert_eq!(page.selection, None);
    undo(&mut page);
    assert_eq!(body(&page), "a cat", "and comes back in one step");
}

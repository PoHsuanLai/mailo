//! Lane 1: find a person in the search panel, write to them, attach two files (one through the
//! Attach button's dialog, one dropped from a file manager), send, take it back, send again.

use ds::file_drop::drag::{DropAcceptance, FileDragInput, Offer};
use ds_harness::Query;
use mail_store::Store;
use std::path::PathBuf;

use super::drive::{Drive, Key};
use super::window::{Window, account, panel_row, panel_settled, parsed, queued};

/// The search panel's field.
const FIELD: &str = ".spotlight input";
/// The composer's To field.
const TO: &str = ".c-props [*|data-row=to] .c-pin input";
/// The attached files' chips.
const ATTACHED: &str = ".c-props [*|data-row=attached] .ds-chip";
/// The send pill's Undo.
const UNDO: &str = ".send-at .ds-send-pill-undo";
/// The composer's Send button.
const SEND: &str = ".c-foot [*|aria-label=\"Send\"]";

/// Drag `paths` in from a file manager and let go over `selector`.
fn drop_on(window: &mut Window, selector: &str, paths: Vec<PathBuf>) -> DropAcceptance {
    let at = window.centre(selector);
    let harness = &mut window.harness;
    harness.file_drag(FileDragInput::Entered { point: Some(at) });
    harness.file_drag(FileDragInput::Offered(Offer::Files(paths)));
    let told = harness.file_drag(FileDragInput::Moved { point: at });
    harness.file_drag(FileDragInput::Dropped);
    told
}

/// Find Ada in the search panel, then write to her with two files attached: the composer is
/// left open, ready to send.
fn write_to_ada_with_two_files() -> Window {
    let mut window = Window::open(|_| {});

    // ⌘K: the panel comes up with the keyboard in its field.
    window.press(&[Key::Ctrl], Key::Char('k'), 1);
    window.until("⌘K puts the keyboard in the panel's field", |h| {
        h.is_focused(FIELD)
    });

    // Part of a name: the person is offered among People.
    window.type_text("lovel");
    window.until("the panel offers Ada among its people", |h| {
        panel_settled(h) && panel_row(h, "ada@example.test").is_some()
    });
    let people = window.text(".spotlight-rows");
    assert!(people.contains("Ada Lovelace"), "{people}");

    // Picking her shows her mail: the search becomes hers and the panel goes.
    let n = panel_row(&window.harness, "ada@example.test").unwrap_or_default();
    window.click(&format!(".spotlight-rows .ds-menu > :nth-child({n})"));
    window.until("the panel goes", |h| h.count(".spotlight input") == 0);
    window.until("the list shows only Ada's mail", |h| {
        h.count(".list .ds-thread") == 1
    });
    assert_eq!(window.subjects(), ["Flight to the conference"]);

    // A new message. The panel's person row reads her mail and offers no "write to", so she
    // is found again where a composer finds people: its To field.
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    window.click(TO);
    window.type_text("lovel");
    window.until("To offers Ada", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("Ada Lovelace"))
    });
    window.press(&[], Key::Enter, 1);
    window.until("Ada is in To", |h| {
        h.count(".c-props [*|aria-label=\"Remove Ada Lovelace\"]") == 1
    });

    window.click(".c-title input");
    window.type_text("The slides for Friday");
    window.click(".c-body");
    window.type_text("Both files are attached.");

    // Attach, through the dialog.
    let slides = window.file("slides.pdf", b"%PDF-1.4 slides");
    let notes = window.file("notes.txt", b"Speaker notes.");
    window.answer(vec![slides]);
    window.click(".cpage [*|aria-label=\"Attach\"]");
    window.until("the picked file is attached", |h| h.count(ATTACHED) == 1);
    assert_eq!(
        window.asked(),
        [mail_app::ui::native::DialogAsk::Attachments]
    );

    // And a second, dropped on the page from a file manager.
    let told = drop_on(&mut window, ".c-body", vec![notes]);
    assert_eq!(told, DropAcceptance::Copy, "the composer took the drop");
    window.until("the dropped file is attached", |h| h.count(ATTACHED) == 2);
    let chips = window.text(".c-props [*|data-row=attached]");
    assert!(
        chips.contains("slides.pdf") && chips.contains("notes.txt"),
        "{chips}"
    );
    window
}

/// What Send queued: one message to Ada, with the text and both files.
fn assert_sent_to_ada(window: &Window) {
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    assert_eq!(sent[0].rcpt_to, ["ada@example.test"]);
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.subject, "The slides for Friday");
    assert!(
        mail.text
            .as_deref()
            .is_some_and(|text| text.contains("Both files are attached.")),
        "{:?}",
        mail.text
    );
    let names: Vec<&str> = mail.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["slides.pdf", "notes.txt"]);
    assert_eq!(mail.attachments[0].bytes, b"%PDF-1.4 slides");
    assert_eq!(mail.attachments[1].bytes, b"Speaker notes.");
    let draft = window.store.draft(sent[0].draft).expect("the sent draft");
    assert_eq!(draft.account, account());
}

#[test]
fn a_person_found_in_search_is_written_to_with_two_files_and_sent() {
    let mut window = write_to_ada_with_two_files();
    window.click(SEND);
    window.until("the pill offers Undo", |h| h.count(UNDO) == 1);
    assert_sent_to_ada(&window);
}

#[test]
#[ignore = "gap(quire): Blitz's hit test misses the send pill's Undo, drawn above its zero-height .send-at box; the click lands on .c-foot"]
fn a_send_is_taken_back_from_the_pill_with_its_files_and_sent_again() {
    let mut window = write_to_ada_with_two_files();

    // Send, then Undo from the pill: the draft comes back as it was, and nothing is queued.
    window.click(SEND);
    window.until("the pill offers Undo", |h| h.count(UNDO) == 1);
    assert_eq!(queued(&window.store).len(), 1, "Send queued nothing");
    window.click(UNDO);
    window.until("the draft comes back", |h| {
        h.count(".cpage .c-body") == 1 && h.count(UNDO) == 0
    });
    assert!(queued(&window.store).is_empty(), "Undo left it queued");
    window.until("the attachments came back with it", |h| {
        h.count(ATTACHED) == 2
    });
    assert!(
        window.text(".c-body").contains("Both files are attached."),
        "the body did not come back: {}",
        window.text(".c-body")
    );

    // Send again: one message, to Ada, with the text and both files.
    window.click(SEND);
    window.until("the pill offers Undo again", |h| h.count(UNDO) == 1);
    assert_sent_to_ada(&window);
}

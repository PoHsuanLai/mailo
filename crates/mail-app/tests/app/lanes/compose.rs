//! Lane 1: writing. Find a person in the search panel, write to them, attach two files (one
//! through the Attach button's dialog, one dragged in from a file manager, which lights the
//! composer and is refused anywhere else, and refused as a folder), copy the body, and send.
//! A new message opens in the reader column every way it is asked for and a reply under its
//! thread; what is not an address marks To and keeps Send put; Esc parks a written draft, which
//! ⌘K's Recent brings back, and a draft in Drafts opens in place and is deleted from its menu. A
//! `mailto:` link opens the window on its composer and sends nothing.

use ds::base::press::PointerButton;
use ds::file_drop::drag::{DropAcceptance, FileDragInput, Offer};
use ds_harness::Harness;
use mail_app::ui::copy;
use mail_core::Store;
use mail_domain::SendState;
use std::path::PathBuf;

use super::drive::{Drive, Key, PRIMARY};
use super::look::{composer_sizes_hold, send};
use super::row_menu::press_menu_item;
use super::window::{INBOX, Window, account, panel_row, panel_settled, parsed, queued};

/// The search panel's field.
const FIELD: &str = ".ds-search-card input";
/// The composer's To field.
const TO: &str = ".c-props [*|data-row=to] .c-pin input";
/// The attached files' chips.
const ATTACHED: &str = ".c-attached .ds-chip";
/// The send pill's Undo.
const UNDO: &str = ".send-at .ds-send-pill-undo";

/// Drag `paths` in from a file manager and let go at `at`: what the window told the platform
/// just before the release.
fn drop_at(harness: &mut Harness, at: ds::prelude::Point, paths: Vec<PathBuf>) -> DropAcceptance {
    harness.file_drag(FileDragInput::Entered { point: Some(at) });
    harness.file_drag(FileDragInput::Offered(Offer::Files(paths)));
    let told = harness.file_drag(FileDragInput::Moved { point: at });
    harness.file_drag(FileDragInput::Dropped {
        modifiers: dioxus::prelude::Modifiers::empty(),
    });
    told
}

/// The composer's drop light: `accepts` while files are dragged over the window, `target` over
/// the composer itself.
fn light(window: &Window) -> Option<String> {
    window.harness.attr(".cpage", "data-drop")
}

#[test]
fn a_person_found_in_search_is_written_to_with_two_files_and_sent() {
    let mut window = Window::open(|_| {});

    // ⌘K, part of a name: the person is offered among People, and picking her shows her mail.
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K puts the keyboard in the panel", |h| {
        h.is_focused(FIELD)
    });
    window.type_text("lovel");
    window.until("the panel offers Ada among its people", |h| {
        panel_settled(h) && panel_row(h, "ada@example.test").is_some()
    });
    let n = panel_row(&window.harness, "ada@example.test").unwrap_or_default();
    window.click(&format!(".ds-search-card-rows .ds-menu > :nth-child({n})"));
    window.until("the panel goes", |h| h.count(FIELD) == 0);
    window.until("the list shows only Ada's mail", |h| {
        h.count(".list .ds-thread") == 1
    });
    assert_eq!(window.subjects(), [INBOX[0].1]);

    // A new message, and she is found again in its To field.
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    window.click(TO);
    window.type_text("lovel");
    window.until("To offers Ada", |h| h.count(".ds-menu .ds-menu-item") > 0);
    window.press(&[], Key::Enter, 1);
    window.until("Ada is in To", |h| {
        h.count(".c-props [*|data-row=to] .ds-chip") == 1
    });
    window.click(".c-title input");
    window.type_text("The slides for Friday");
    window.click(".c-body");
    window.type_text("Both files are attached.");

    // The body's text copies with the window's primary key.
    window.press(&[PRIMARY], Key::Char('a'), 1);
    window.press(&[PRIMARY], Key::Char('c'), 1);
    assert_eq!(
        window.harness.clipboard_text().as_deref(),
        Some("Both files are attached.")
    );
    window.press(&[], Key::End, 1);

    // Attach, through the dialog.
    let slides = window.file("slides.pdf", b"%PDF-1.4 slides");
    let notes = window.file("notes.txt", b"Speaker notes.");
    window.answer(vec![slides]);
    window.click(&format!(
        ".cpage [*|aria-label=\"{}\"]",
        copy::controls::ATTACH
    ));
    window.until("the picked file is attached", |h| h.count(ATTACHED) == 1);
    assert_eq!(
        window.asked(),
        [mail_app::ui::native::DialogAsk::Attachments]
    );

    // Files dragged over the window light the composer, over it target it, and the light goes
    // with the drag; the list takes no files.
    assert_eq!(light(&window), None, "lit before a drag");
    let list = window.centre(".list .ds-thread");
    let h = &mut window.harness;
    h.file_drag(FileDragInput::Entered { point: Some(list) });
    let over_list = h.file_drag(FileDragInput::Offered(Offer::Files(vec![notes.clone()])));
    assert_eq!(over_list, DropAcceptance::Refuse, "the list takes files");
    assert_eq!(light(&window).as_deref(), Some("accepts"));
    let body = window.centre(".c-body");
    let over_body = window
        .harness
        .file_drag(FileDragInput::Moved { point: body });
    assert_eq!(
        over_body,
        DropAcceptance::Copy,
        "the composer takes no files"
    );
    assert_eq!(light(&window).as_deref(), Some("target"));
    window.harness.file_drag(FileDragInput::Left);
    assert_eq!(light(&window), None, "the light stayed");

    // Dropped on the list it is refused; on the composer it is attached; a folder is refused
    // with a note.
    let list = window.centre(".list .ds-thread");
    let told = drop_at(&mut window.harness, list, vec![notes.clone()]);
    assert_eq!(told, DropAcceptance::Refuse);
    window.harness.advance(super::window::ms(500));
    assert_eq!(
        window.harness.count(ATTACHED),
        1,
        "the list's drop attached"
    );
    let body = window.centre(".c-body");
    let told = drop_at(&mut window.harness, body, vec![notes]);
    assert_eq!(told, DropAcceptance::Copy, "the composer took the drop");
    window.until("the dropped file is attached", |h| h.count(ATTACHED) == 2);
    let photos = window.file("photos", b"");
    std::fs::remove_file(&photos).expect("room for a folder");
    std::fs::create_dir(&photos).expect("a folder");
    let body = window.centre(".c-body");
    drop_at(&mut window.harness, body, vec![photos]);
    window.until("the folder is refused with a note", |h| {
        h.count(".cpage .ds-inline-banner") == 1
    });
    assert_eq!(window.harness.count(ATTACHED), 2, "the folder was attached");

    // Send: one message to Ada, with the text and both files.
    window.click(&send());
    window.until("the pill offers Undo", |h| h.count(UNDO) == 1);
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

/// A picture in the composer's body.
const PICTURE: &str = ".c-body figure.o-img img";

/// Pictures go in the body every way: `/` › Image's dialog, dropped from a file manager, and
/// pasted from the clipboard. Each is drawn where the caret was and kept out of the strip of
/// files, and the message carries them inside the HTML's `multipart/related`, each under the
/// `Content-ID` the HTML shows it by.
#[test]
fn pictures_go_in_the_body_by_dialog_drop_and_paste_and_are_sent_inline() {
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nthe garden";
    const GIF: &[u8] = b"GIF89a the cat";
    const PASTED: &[u8] = b"\x89PNG\r\n\x1a\na screenshot";
    let mut window = Window::open_with(
        chrono::Utc::now(),
        |_| {},
        |_, contexts| {
            contexts.with(mail_app::ui::native::ClipboardImage::with_answer(|| {
                Some(PASTED.to_vec())
            }))
        },
    );
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    window.click(TO);
    window.type_text("lovel");
    window.until("To offers Ada", |h| h.count(".ds-menu .ds-menu-item") > 0);
    window.press(&[], Key::Enter, 1);
    window.until("Ada is in To", |h| {
        h.count(".c-props [*|data-row=to] .ds-chip") == 1
    });
    window.click(".c-title input");
    window.type_text("The garden");
    window.click(".c-body");
    window.type_text("Look:\n");

    // `/` › Image asks the dialog for pictures, and the one picked is drawn in the body.
    let garden = window.file("garden.png", PNG);
    window.answer(vec![garden]);
    window.type_text("/image");
    window.until("/ offers Image", |h| h.count(".ds-menu .ds-menu-item") > 0);
    window.press(&[], Key::Enter, 1);
    window.until("the picked picture is drawn", |h| h.count(PICTURE) == 1);
    assert_eq!(window.asked(), [mail_app::ui::native::DialogAsk::Images]);
    assert!(
        window
            .harness
            .attr(PICTURE, "src")
            .is_some_and(|src| src.starts_with("data:image/png;base64,")),
        "{:?}",
        window.harness.attr(PICTURE, "src")
    );

    // A picture dropped on the composer goes in the body too, not in the strip of files.
    let cat = window.file("cat.gif", GIF);
    let body = window.centre(".c-body");
    let told = drop_at(&mut window.harness, body, vec![cat]);
    assert_eq!(told, DropAcceptance::Copy, "the composer took the drop");
    window.until("the dropped picture is drawn", |h| h.count(PICTURE) == 2);

    // The paste chord with only a picture on the clipboard pastes the picture.
    window.press(&[PRIMARY], Key::Char('v'), 1);
    window.until("the pasted picture is drawn", |h| h.count(PICTURE) == 3);
    assert_eq!(
        window.harness.count(ATTACHED),
        0,
        "a picture is listed as a file"
    );

    window.click(&send());
    window.until("the pill offers Undo", |h| h.count(UNDO) == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    assert!(
        sent[0].raw.contains("multipart/related"),
        "the pictures are not in a related part:\n{}",
        sent[0].raw
    );
    let mail = parsed(&sent[0].raw);
    let html = mail.html.as_deref().unwrap_or_default();
    let mut inline = Vec::new();
    for part in &mail.attachments {
        let mail_domain::Inline::Embedded { cid } = &part.inline else {
            panic!("{} went as a file", part.name);
        };
        assert!(
            html.contains(&format!("cid:{cid}")),
            "{cid} is not in {html}"
        );
        inline.push((part.name.as_str(), part.bytes.as_slice()));
    }
    assert_eq!(
        inline,
        [
            ("garden.png", PNG),
            ("cat.gif", GIF),
            (copy::fields::PASTED_IMAGE, PASTED)
        ]
    );
}

/// The composer is drawn in the reader column, and Esc, nothing typed, closes it.
fn composes_here_then_closes(window: &mut Window, what: &str) {
    window.until(what, |h| h.count(".cpage .c-body") == 1);
    window.press(&[], Key::Escape, 1);
    window.until("Esc closes it", |h| h.count(".cpage") == 0);
}

#[test]
fn a_new_message_opens_here_every_way_and_a_draft_is_parked_brought_back_and_deleted() {
    let mut window = Window::open(|_| {});
    window.open_subject(INBOX[0].1);

    // A reply stays under its thread.
    window.press(&[], Key::Char('r'), 1);
    window.until("r replies under the thread", |h| {
        h.count(".inline-reply .c-body") == 1 && h.count(".cpage") == 0
    });
    window.press(&[], Key::Escape, 1);
    window.until("Esc closes the reply", |h| {
        h.count(".inline-reply .c-body") == 0
    });
    let drafts = |window: &Window| window.store.drafts(account()).map(|d| d.len()).ok();
    let kept = drafts(&window);

    // New Message, ⌘N and ⌘K's New Message each compose in the reader column, and an empty
    // composer leaves no draft.
    window.click(&format!("[*|aria-label=\"{}\"]", copy::aria::NEW_MESSAGE));
    composes_here_then_closes(&mut window, "New Message composes here");
    window.press(&[PRIMARY], Key::Char('n'), 1);
    composes_here_then_closes(&mut window, "⌘N composes here");
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("⌘K brings up the panel", |h| h.is_focused(FIELD));
    window.type_text("new message");
    window.until("the panel offers New Message first", |h| {
        h.text_of(".ds-search-card .ds-menu-item[*|data-selected=true] .ds-menu-title")
            .is_some_and(|title| title.trim() == copy::menus::NEW_MESSAGE)
    });
    window.press(&[], Key::Enter, 1);
    composes_here_then_closes(&mut window, "⌘K's New Message composes here");
    assert_eq!(drafts(&window), kept, "an empty composer left a draft");

    // What is not an address: left, To keeps it and marks itself, with no banner and no line
    // under it, and Send stays put; typed over, the mark goes.
    window.press(&[PRIMARY], Key::Char('n'), 1);
    window.until("⌘N gives To the keyboard", |h| h.is_focused(TO));
    composer_sizes_hold(&window.harness);
    window.type_text("zz@");
    window.click(".c-title input");
    let to = ".cpage .c-props [*|data-row=to] input";
    assert_eq!(window.harness.attr(to, "value").as_deref(), Some("zz@"));
    assert_eq!(
        window.harness.attr(to, "aria-invalid").as_deref(),
        Some("true")
    );
    assert!(
        window.harness.attr(to, "aria-description").is_some(),
        "no reason"
    );
    assert_eq!(window.harness.count(".cpage .ds-inline-banner"), 0);
    assert_eq!(
        window.harness.count(".cpage .c-props .ds-text-field-help"),
        0
    );
    window.press(&[PRIMARY], Key::Enter, 1);
    window.harness.advance(super::window::ms(300));
    assert_eq!(window.harness.count(".cpage"), 1, "Send went ahead");
    let held = window.store.drafts(account()).unwrap_or_default();
    assert!(
        held.iter().all(|d| d.state == SendState::Editing),
        "{held:?}"
    );
    window.click(to);
    window.press(&[PRIMARY], Key::Char('a'), 1);
    window.type_text("zz@example.test");
    window.click(".c-title input");
    assert_ne!(
        window.harness.attr(to, "aria-invalid").as_deref(),
        Some("true")
    );

    // Esc parks a written draft; ⌘K, nothing typed, lists it under Recent, and a press brings it
    // back as it was.
    window.type_text("Lunch plans");
    window.press(&[], Key::Escape, 1);
    window.until("Esc parks it", |h| h.count(".cpage") == 0);
    window.press(&[PRIMARY], Key::Char('k'), 1);
    window.until("Recent lists it", |h| {
        h.is_focused(FIELD) && panel_row(h, "Lunch plans").is_some()
    });
    let n = panel_row(&window.harness, "Lunch plans").unwrap_or_default();
    window.click(&format!(".ds-search-card-rows .ds-menu > :nth-child({n})"));
    window.until("it comes back", |h| h.count(".cpage .c-body") == 1);
    assert_eq!(
        window
            .harness
            .attr(".cpage .c-title input", "value")
            .as_deref(),
        Some("Lunch plans")
    );
    window.press(&[], Key::Escape, 1);
    window.until("Esc parks it again", |h| h.count(".cpage") == 0);

    // In Drafts it opens here with its row selected, and its menu deletes it, offering it back.
    window.click("[*|data-place=\"Drafts\"]");
    let row = format!("[*|aria-label=\"{}\"]", copy::aria::open_row("Lunch plans"));
    window.until("Drafts lists it", |h| h.count(&row) == 1);
    window.click(&row);
    window.until("it opens here, selected", |h| {
        h.count(".cpage .c-body") == 1 && h.attr(&row, "aria-selected").as_deref() == Some("true")
    });
    let at = window.centre(&row);
    window.harness.press(at, PointerButton::Secondary);
    window.until("its menu opens", |h| h.count(".ds-menu .ds-menu-item") > 0);
    press_menu_item(&mut window.harness, copy::menus::DELETE_DRAFT);
    window.until("it goes, and its composer", |h| {
        h.count(&row) == 0 && h.count(".cpage") == 0
    });
    assert_eq!(drafts(&window), kept, "the draft is still stored");
    assert!(
        window
            .text(".ds-toast-body")
            .contains(copy::toasts::DRAFT_DELETED),
        "no toast offers it back"
    );
}

#[test]
fn a_mailto_link_opens_the_window_on_its_composer_and_sends_nothing() {
    let uri = "mailto:ada@example.test?subject=About%20the%20flight&body=Which%20gate%3F";
    let link = mail_app::ui::mailto_of(&[uri.to_owned()]).expect("read as a mailto link");
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = super::seed::seeded(dir.path());
    let start = mail_app::ui::start_mailto(
        &mail_client::Client::inline(
            store.clone(),
            std::sync::Arc::new(mail_core::mail::SystemClock),
        ),
        &link,
        chrono::Utc::now(),
    )
    .expect("the link's draft is saved");
    let mut harness = crate::opening::open_window(
        &store,
        crate::opening::Open {
            start,
            ..crate::opening::Open::default()
        },
    );
    super::hands::until(&mut harness, "the composer opens", |h| {
        h.count(".c-title input") == 1
    });
    assert_eq!(
        harness.attr(".c-title input", "value").as_deref(),
        Some("About the flight")
    );
    assert!(super::hands::text(&harness, ".c-body").contains("Which gate?"));
    assert_eq!(harness.count(".c-props [*|data-row=to] .ds-chip"), 1);

    // A draft, held and not queued: a link can open a composer, never send.
    let drafts = store.drafts(account()).expect("the drafts");
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    assert_eq!(drafts[0].state, SendState::Editing);
    assert!(queued(&store).is_empty());
}

/// A photo of soft gradients with a fine texture, `width` × `height`, as a JPEG.
fn photo(width: u32, height: u32) -> Vec<u8> {
    let picture = image::RgbImage::from_fn(width, height, |x, y| {
        let grain = ((x * 7 + y * 13) % 17) as u8;
        image::Rgb([
            (x * 200 / width) as u8 + grain,
            (y * 200 / height) as u8 + grain,
            90 + grain,
        ])
    });
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(picture)
        .write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .expect("a jpeg");
    out
}

#[test]
fn a_photo_past_what_the_server_takes_is_warned_of_before_send_and_goes_smaller() {
    use mail_core::compose::{PictureSize, sized};
    use mail_domain::human_size;
    use mail_mime::base64_len;

    // A server that takes about halfway between the photo as it is and the photo made small,
    // in octets as it goes: the size its `SIZE` said at the last submission.
    let beach = photo(1600, 1200);
    let small = sized(&beach, PictureSize::Small).len() as u64;
    let limit = (base64_len(beach.len() as u64) + base64_len(small)) / 2 + 8 * 1024;
    let mut window = Window::open(|store| {
        store
            .put_send_limit(account(), Some(limit))
            .expect("the limit is kept");
    });

    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer", |h| h.count(".cpage .c-body") == 1);
    window.click(TO);
    window.type_text("bea@example.test\n");
    window.until("Bea is in To", |h| {
        h.count(".c-props [*|data-row=to] .ds-chip") == 1
    });
    window.click(".c-title input");
    window.type_text("The beach");

    // Attached, it is weighed: Picture Size is offered at the strip, and the foot says the
    // message is larger than its server takes, by how much, before Send is pressed.
    window.answer(vec![window.file("beach.jpg", &beach)]);
    window.click(&format!(
        ".cpage [*|aria-label=\"{}\"]",
        copy::controls::ATTACH
    ));
    let sizes = |name: &str| {
        format!(
            ".c-attached [*|aria-label=\"{}\"]",
            copy::controls::picture_size(name)
        )
    };
    let actual = sizes(copy::menus::PICTURES_ACTUAL);
    window.until("Picture Size is offered", |h| h.count(&actual) == 1);
    let banner = ".cpage .c-foot .ds-inline-banner";
    window.until("the foot warns of the size", |h| h.count(banner) == 1);
    let said = window.text(banner);
    assert!(
        said.contains(&human_size(limit)),
        "the limit is not said: {said}"
    );
    assert!(
        window.harness.count(&format!("{banner} button")) >= 1,
        "no smaller size is offered: {said}"
    );

    // Send stops once, and offers Send Anyway; nothing is queued.
    let anyway = format!("[*|aria-label=\"{}\"]", copy::controls::SEND_ANYWAY);
    window.click(&send());
    window.until("Send stops for the size", |h| h.count(&anyway) == 1);
    assert!(
        queued(&window.store).is_empty(),
        "it was queued over the limit"
    );

    // Small: the warning goes, the strip says what goes with the location.
    window.click(&actual);
    window.until("Picture Size opens", |h| {
        h.count(".ds-menu .ds-menu-item") == 4
    });
    press_menu_item(&mut window.harness, copy::menus::PICTURES_SMALL);
    window.until("the warning goes", |h| {
        h.count(banner) == 0 && h.count(&sizes(copy::menus::PICTURES_SMALL)) == 1
    });
    assert!(
        window
            .text(".c-attached")
            .contains(copy::status::PICTURES_WITHOUT_LOCATION),
        "{}",
        window.text(".c-attached")
    );

    // Sent, the photo goes at 640 pixels, and the draft keeps it as it was attached.
    window.click(&send());
    window.until("the pill offers Undo", |h| h.count(UNDO) == 1);
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one submission");
    let mail = parsed(&sent[0].raw);
    assert_eq!(mail.attachments.len(), 1);
    let went = image::load_from_memory(&mail.attachments[0].bytes).expect("a picture went");
    assert_eq!((went.width(), went.height()), (640, 480));
    let draft = window.store.draft(sent[0].draft).expect("the sent draft");
    let kept = window
        .store
        .blobs()
        .get(draft.attachments[0].blob)
        .expect("the original is kept");
    assert_eq!(kept, beach, "the draft's photo was changed");
}

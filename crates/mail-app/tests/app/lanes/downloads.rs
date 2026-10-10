//! Downloads: an attachment saved from the reader is listed behind the sidebar's Downloads
//! button and opens from there; its right click shows it in its folder, leads back to its
//! message, and clears the list while the file stays.

use super::window::{Window, deliver, hours_ago};

const SUBJECT: &str = "Notes from Thursday";
const NAME: &str = "notes.txt";
const BUTTON: &str = ".ds-spaces-foot [*|aria-label=\"Downloads\"]";
const LIST: &str = ".downloads";
const DOT: &str = ".downloads-dot";

fn with_notes(store: &mail_core::SqliteStore) {
    let raw = format!(
        "From: Grace Hopper <grace@example.test>\r\nTo: Me <me@example.test>\r\n\
         Subject: {SUBJECT}\r\nDate: {}\r\nMessage-ID: <notes1@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nThe notes are attached.\r\n\
         --mix\r\nContent-Type: text/plain; charset=UTF-8; name=\"{NAME}\"\r\n\
         Content-Disposition: attachment; filename=\"{NAME}\"\r\n\r\n\
         Ship the downloads list.\r\n--mix--\r\n",
        hours_ago(0)
    );
    deliver(store, "notes1", &raw);
}

/// Right-click the list's first file, and wait for its menu.
fn row_menu(window: &mut Window) {
    use super::drive::Drive as _;
    use ds_harness::Query as _;
    let at = window.centre(&format!("{LIST} .ds-row"));
    window
        .harness
        .press(at, ds::base::press::PointerButton::Secondary);
    window.until("the row's menu opens", |h| {
        h.count(".ds-menu .ds-menu-item") > 0
    });
}

fn listed(h: &ds_harness::Harness) -> bool {
    use ds_harness::Query as _;
    h.text_of(LIST).is_some_and(|list| list.contains(NAME))
}

#[test]
fn a_saved_attachment_is_listed_opened_shown_and_cleared() {
    use ds_harness::Query as _;
    let mut window = Window::open(with_notes);
    assert_eq!(
        window.harness.count(DOT),
        0,
        "a dot before anything was saved"
    );
    window.click(BUTTON);
    window.until("the empty list opens", |h| {
        h.text_of(LIST)
            .is_some_and(|list| list.contains("show up here"))
    });
    // A second click on the button closes it: the list's catch takes it, as any click outside.
    let at = window.centre(BUTTON);
    {
        use super::drive::Drive as _;
        window.harness.click(at);
    }
    window.until("the list closes again", |h| h.count(LIST) == 0);

    window.open_subject(SUBJECT);
    window.until("the reader lists the attachment", |h| {
        h.text_of(".reader .attachments")
            .is_some_and(|rows| rows.contains(NAME))
    });
    // The attachment's paperclip stands beside its name, on the row's line, not above it.
    let line = window
        .harness
        .rect(".reader .att-line")
        .expect("no attachment line");
    let mark = window
        .harness
        .rect(".reader .att-line > :first-child")
        .expect("no paperclip");
    let name = window
        .harness
        .rect(".reader .att-line > :last-child")
        .expect("no row");
    assert!(
        mark.origin.x.0 + mark.size.width.0 <= name.origin.x.0 + 0.5,
        "the paperclip is not before the name: {mark:?} {name:?}"
    );
    assert!(
        mark.origin.y.0 >= line.origin.y.0 - 0.5
            && mark.origin.y.0 + mark.size.height.0 <= line.origin.y.0 + line.size.height.0 + 0.5,
        "the paperclip is not on the row's line: {mark:?} in {line:?}"
    );
    window.click(&format!(".reader [*|aria-label=\"Save {NAME}\"]"));
    let saved = window.saves().join(NAME);
    window.until("the file is saved and the button has its dot", |h| {
        saved.exists() && h.count(DOT) == 1
    });

    // The list: the file, its size and its message; looking at it puts the dot away.
    window.click(BUTTON);
    window.until("the list shows the file", listed);
    let list = window.text(LIST);
    assert!(list.contains(SUBJECT), "the message is not named: {list}");
    assert_eq!(
        window.harness.count(DOT),
        0,
        "the dot stayed once looked at"
    );

    // A click on the row opens the file, and the list closes.
    window.click(&format!("{LIST} .ds-row"));
    window.until_launched("the file is opened", 1);
    window.until("the list closes on open", |h| h.count(LIST) == 0);
    assert_eq!(window.launched(), [("open", saved.clone())]);

    // Show in Folder, from the row's right click.
    window.click(BUTTON);
    window.until("the list opens again", listed);
    row_menu(&mut window);
    window.menu_item("Show in Folder");
    window.until_launched("the file is shown in its folder", 2);
    assert_eq!(window.launched()[1], ("reveal", saved.clone()));

    // Go to Message closes the list on the conversation.
    window.click(BUTTON);
    window.until("the list opens for the message", listed);
    row_menu(&mut window);
    window.menu_item("Go to Message");
    window.until("the list closes on the message", |h| h.count(LIST) == 0);
    assert!(
        window.text(".reader").contains(SUBJECT),
        "the reader is not on the message"
    );

    // Clear List empties the list, in this window and on disk, and the file stays.
    window.click(BUTTON);
    window.until("the list opens a third time", listed);
    row_menu(&mut window);
    window.menu_item("Clear List");
    window.until("the list closes on Clear", |h| h.count(LIST) == 0);
    window.click(BUTTON);
    window.until("the list is empty", |h| {
        h.text_of(LIST)
            .is_some_and(|list| !list.contains(NAME) && list.contains("show up here"))
    });
    assert!(saved.exists(), "Clear removed the file");
    let stored =
        std::fs::read_to_string(window.state().join("downloads.json")).expect("the list is stored");
    assert!(
        !stored.contains(NAME),
        "the stored list kept the file: {stored}"
    );
}

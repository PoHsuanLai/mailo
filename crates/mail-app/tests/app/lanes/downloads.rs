//! Downloads: an attachment saved from the reader is listed behind the sidebar's Downloads
//! button and opens from there; its right click shows it in its folder, leads back to its
//! message, and clears the list while the file stays.

use ds::prelude::{Point, Px};

use super::window::{Window, deliver, hours_ago, ms};

const SUBJECT: &str = "Notes from Thursday";
const NAME: &str = "notes.txt";
const BUTTON: &str = ".side-head [*|aria-label=\"Downloads\"]";
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
    let at = window.centre(&format!("{LIST} .ds-row"));
    window
        .harness
        .press(at, ds::base::press::PointerButton::Secondary);
    window.until("the row's menu opens", |h| {
        h.count(".ds-menu .ds-menu-item") > 0
    });
}

fn listed(h: &ds_harness::Harness) -> bool {
    h.text_of(LIST).is_some_and(|list| list.contains(NAME))
}

#[test]
fn a_saved_attachment_is_listed_opened_shown_and_cleared() {
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
    // The attachment is a tile: its paperclip stands over its name.
    let mark = window
        .harness
        .rect(".reader .att-tile > :first-child")
        .expect("no paperclip");
    let name = window
        .harness
        .rect(".reader .att-tile .att-text")
        .expect("no name");
    assert!(
        mark.origin.y.0 + mark.size.height.0 <= name.origin.y.0 + 0.5,
        "the paperclip is not over the name: {mark:?} {name:?}"
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

const LONG_SUBJECT: &str = "The board's papers";
/// A long name with spaces, which would wrap, and one with none, which would run out of its tile.
const LONG_NAMES: [&str; 3] = [
    "Quarterly report for the board meeting, final revised version three.pdf",
    "Quarterly_report_for_the_board_meeting_final_revised_version_three.pdf",
    "a.txt",
];

fn with_long_names(store: &mail_core::SqliteStore) {
    let parts: String = LONG_NAMES
        .iter()
        .map(|name| {
            format!(
                "--mix\r\nContent-Type: text/plain; charset=UTF-8; name=\"{name}\"\r\n\
                 Content-Disposition: attachment; filename=\"{name}\"\r\n\r\nThe papers.\r\n"
            )
        })
        .collect();
    let raw = format!(
        "From: Grace Hopper <grace@example.test>\r\nTo: Me <me@example.test>\r\n\
         Subject: {LONG_SUBJECT}\r\nDate: {}\r\nMessage-ID: <papers1@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nThe papers are attached.\r\n\
         {parts}--mix--\r\n",
        hours_ago(0)
    );
    deliver(store, "papers1", &raw);
}

/// A long name keeps to its tile's one line, cut short before its extension: it does not wrap to
/// make the strip taller, which takes the room from the body above, nor run out of the border.
#[test]
fn a_long_name_is_cut_short_inside_its_tile() {
    let mut window = Window::open(with_long_names);
    window.open_subject(LONG_SUBJECT);
    window.until("the reader shows the three tiles", |h| {
        h.count(".reader .att-tile") == 3
    });
    let tile = |n: usize| format!(".reader .att-strip > .att-tile:nth-child({n})");
    let rect = |selector: &str| window.harness.rect(selector).expect(selector);
    // The strip draws its tiles one height, so a name is held to the one line a short one takes.
    let line = rect(&format!("{} .att-name", tile(3))).size.height.0;
    for n in 1..=2 {
        let long = rect(&tile(n));
        let name = rect(&format!("{} .att-name", tile(n)));
        assert!(
            name.size.height.0 <= line + 0.5,
            "tile {n}'s name takes more than one line: {name:?}, a line is {line}"
        );
        for part in [
            ".att-name",
            ".att-name > :first-child",
            ".att-name > :last-child",
        ] {
            let inside = rect(&format!("{} {part}", tile(n)));
            assert!(
                inside.origin.x.0 >= long.origin.x.0 - 0.5
                    && inside.origin.x.0 + inside.size.width.0
                        <= long.origin.x.0 + long.size.width.0 + 0.5,
                "tile {n}'s {part} runs out of the tile: {inside:?} {long:?}"
            );
        }
        assert_eq!(
            window.text(&format!("{} .att-name > :last-child", tile(n))),
            ".pdf",
            "tile {n} lost its extension"
        );
        // The whole name is still there to read out, and on its Save button.
        assert_eq!(
            window.text(&format!("{} .att-name", tile(n))),
            LONG_NAMES[n - 1]
        );
    }

    // A name cut short shows itself whole as its tooltip; one drawn whole has none.
    for (n, tip) in [
        (1, Some(LONG_NAMES[0])),
        (2, Some(LONG_NAMES[1])),
        (3, None),
    ] {
        {
            use super::drive::Drive as _;
            window.harness.pointer_move(Point {
                x: Px(5.0),
                y: Px(790.0),
            });
        }
        window.harness.advance(ms(2500));
        assert_eq!(
            window.harness.count(".ds-tooltip"),
            0,
            "a tip stands with the pointer away"
        );
        let at = window.centre(&format!("{} .att-name > :first-child", tile(n)));
        {
            use super::drive::Drive as _;
            window.harness.pointer_move(at);
        }
        window.harness.advance(ms(1300));
        assert_eq!(
            window.harness.text_of(".ds-tooltip").as_deref(),
            tip,
            "tile {n}'s tooltip"
        );
    }
}

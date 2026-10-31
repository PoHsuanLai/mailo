//! Downloads: an attachment saved from the reader is listed behind the sidebar's Downloads
//! button and opens from there; its right click shows it in its folder, leads back to its
//! message, and clears the list while the file stays.

use ds::prelude::{Point, Px};

use super::window::{Window, deliver, hours_ago, ms};

const SUBJECT: &str = "Notes from Thursday";
const NAME: &str = "notes.txt";
const BUTTON: &str = ".side-head .downloads-hold .ds-button";
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

/// A saved file's menu, by place: Open, Show in Folder, Go to Message, a rule, Remove from List
/// and Clear List.
const SHOW_IN_FOLDER: usize = 2;
const GO_TO_MESSAGE: usize = 3;
const CLEAR_LIST: usize = 6;

/// Pick the open menu's `n`th row.
fn pick(window: &mut Window, n: usize) {
    window.click(&format!(".ds-menu .ds-menu-list > :nth-child({n})"));
    window.harness.advance(ms(300));
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
    // The tile's own button: Save, as the part is here.
    window.click(".reader .att-tile > .ds-button");
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
    pick(&mut window, SHOW_IN_FOLDER);
    window.until_launched("the file is shown in its folder", 2);
    assert_eq!(window.launched()[1], ("reveal", saved.clone()));

    // Go to Message closes the list on the conversation.
    window.click(BUTTON);
    window.until("the list opens for the message", listed);
    row_menu(&mut window);
    pick(&mut window, GO_TO_MESSAGE);
    window.until("the list closes on the message", |h| h.count(LIST) == 0);
    assert!(
        window.text(".reader").contains(SUBJECT),
        "the reader is not on the message"
    );

    // Clear List empties the list, in this window and on disk, and the file stays.
    window.click(BUTTON);
    window.until("the list opens a third time", listed);
    row_menu(&mut window);
    pick(&mut window, CLEAR_LIST);
    window.until("the list closes on Clear", |h| h.count(LIST) == 0);
    window.click(BUTTON);
    window.until("the list is empty", |h| {
        h.count(&format!("{LIST} .ds-empty-state")) == 1
            && h.text_of(LIST).is_some_and(|list| !list.contains(NAME))
    });
    assert!(saved.exists(), "Clear removed the file");
    let stored = std::fs::read_to_string(window.dirs().state.join("downloads.json"))
        .expect("the list is stored");
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

const SAVE_ALL_SUBJECT: &str = "Both copies of the minutes";
/// Two parts that claim one name, and one whose claimed name climbs out of the folder.
const CLAIMED: [&str; 3] = ["minutes.txt", "minutes.txt", "../../escape.txt"];

fn with_three_parts(store: &mail_core::SqliteStore) {
    let parts: String = CLAIMED
        .iter()
        .enumerate()
        .map(|(n, name)| {
            format!(
                "--mix\r\nContent-Type: text/plain; charset=UTF-8; name=\"{name}\"\r\n\
                 Content-Disposition: attachment; filename=\"{name}\"\r\n\r\npart {n}\r\n"
            )
        })
        .collect();
    let raw = format!(
        "From: Grace Hopper <grace@example.test>\r\nTo: Me <me@example.test>\r\n\
         Subject: {SAVE_ALL_SUBJECT}\r\nDate: {}\r\nMessage-ID: <minutes1@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nBoth are attached.\r\n\
         {parts}--mix--\r\n",
        hours_ago(0)
    );
    deliver(store, "minutes1", &raw);
}

/// Save All… asks for a folder and writes every part into it in one go: a name already taken
/// there, or taken by the part before, gets " (2)" and on, and nothing is written over; a name
/// that climbs out of the folder is written inside it. The plain Save folder is not touched.
#[test]
fn save_all_writes_every_part_into_the_folder_chosen() {
    let mut window = Window::open(with_three_parts);
    let chosen = tempfile::tempdir().expect("a folder to choose");
    std::fs::write(chosen.path().join("minutes.txt"), b"already here").expect("a file there");
    window.answer(vec![chosen.path().to_path_buf()]);

    window.open_subject(SAVE_ALL_SUBJECT);
    window.until("the reader shows the three tiles", |h| {
        h.count(".reader .att-tile") == 3
    });
    window.click(".reader .att-head .ds-button");
    let written = |name: &str| chosen.path().join(name);
    window.until("every part is written into the folder", |_| {
        written("minutes (3).txt").exists() && written("escape.txt").exists()
    });

    assert_eq!(
        window.asked(),
        [mail_ui_kit::pick::Ask::Folder],
        "Save All asked for something other than a folder"
    );
    // (file, what it holds)
    let want = [
        ("minutes.txt", "already here"),
        ("minutes (2).txt", "part 0"),
        ("minutes (3).txt", "part 1"),
        ("escape.txt", "part 2"),
    ];
    for (name, holds) in want {
        let text = std::fs::read_to_string(written(name)).unwrap_or_default();
        assert_eq!(text.trim_end(), holds, "{name}");
    }
    let mut names: Vec<String> = std::fs::read_dir(chosen.path())
        .expect("the folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "escape.txt",
            "minutes (2).txt",
            "minutes (3).txt",
            "minutes.txt"
        ]
    );
    assert!(
        !chosen
            .path()
            .parent()
            .is_some_and(|up| up.join("escape.txt").exists()),
        "a part was written outside the folder"
    );
    let saves = std::fs::read_dir(window.saves())
        .expect("the saves folder")
        .count();
    assert_eq!(saves, 0, "Save All wrote into the plain Save folder");
}

/// Right-click a tile, wait for its menu, and pick its `n`th row.
fn tile_menu(window: &mut Window, tile: usize, n: usize) {
    use super::drive::Drive as _;
    let at = window.centre(&format!(
        ".reader .att-strip > .att-tile:nth-child({tile}) .att-text"
    ));
    window
        .harness
        .press(at, ds::base::press::PointerButton::Secondary);
    window.until("the tile's menu opens", |h| {
        h.count(".ds-menu .ds-menu-item") > 0
    });
    pick(window, n);
}

/// A tile's Save to… asks for a folder and writes that part alone into it; its Save writes into
/// the folder chosen in Settings rather than the system's Downloads.
#[test]
fn save_to_asks_for_a_folder_and_save_follows_the_setting() {
    let folder = tempfile::tempdir().expect("the folder Save is set to");
    let set = folder.path().to_path_buf();
    let mut window = Window::open_with(
        chrono::Utc::now(),
        with_three_parts,
        move |dir, contexts| {
            let settings = mail_ui_model::settings::root_for(&dir.join("config"));
            mail_ui_model::settings::change(&settings, |settings| {
                settings.attachments.downloads_folder = set.display().to_string();
            })
            .expect("the setting is written");
            contexts
        },
    );
    let chosen = tempfile::tempdir().expect("a folder to choose");
    window.answer(vec![chosen.path().to_path_buf()]);
    window.open_subject(SAVE_ALL_SUBJECT);
    window.until("the reader shows the three tiles", |h| {
        h.count(".reader .att-tile") == 3
    });

    // Save to…, the menu's second row, on the third tile.
    tile_menu(&mut window, 3, 2);
    let escaped = chosen.path().join("escape.txt");
    window.until("the part is written into the folder chosen", |_| {
        escaped.exists()
    });
    assert_eq!(window.asked(), [mail_ui_kit::pick::Ask::Folder]);
    let count = |dir: &std::path::Path| std::fs::read_dir(dir).expect("a folder").count();
    assert_eq!(count(chosen.path()), 1, "Save to… wrote more than its part");

    // Save, the tile's own button, on the first: into the folder Settings names.
    window.click(".reader .att-strip > .att-tile:nth-child(1) > .ds-button");
    let saved = folder.path().join("minutes.txt");
    window.until("the part is saved into the folder set", |_| saved.exists());
    assert_eq!(
        count(window.saves().as_path()),
        0,
        "Save ignored the setting"
    );
    assert_eq!(window.asked().len(), 1, "Save asked for a folder");
}

/// Open the reader's ⋯ and pick `name`, which the menu says by how many files the message has.
fn more_menu(window: &mut Window, name: &str) {
    window.click(&format!(
        ".reader-head button[*|aria-label=\"{}\"]",
        mail_ui_model::copy::aria::MORE
    ));
    window.menu_item(name);
}

/// The reader's ⋯ saves a message's files without a right click: Save Attachments… asks for a
/// folder and writes every file into it.
#[test]
fn the_more_menu_saves_the_files_into_a_folder_chosen() {
    let mut window = Window::open(with_three_parts);
    let chosen = tempfile::tempdir().expect("a folder to choose");
    window.answer(vec![chosen.path().to_path_buf()]);
    window.open_subject(SAVE_ALL_SUBJECT);
    window.until("the reader shows the three tiles", |h| {
        h.count(".reader .att-tile") == 3
    });
    more_menu(&mut window, mail_ui_model::copy::menus::SAVE_ATTACHMENTS);
    let written = |name: &str| chosen.path().join(name);
    window.until("every part is written into the folder", |_| {
        written("minutes.txt").exists()
            && written("minutes (2).txt").exists()
            && written("escape.txt").exists()
    });
    assert_eq!(
        window.asked(),
        [mail_ui_kit::pick::Ask::Folder],
        "Save Attachments asked for something other than a folder"
    );
}

/// With one file the same row says so, and writes that file.
#[test]
fn the_more_menu_names_a_single_file_and_saves_it() {
    let mut single = Window::open(with_notes);
    let chosen = tempfile::tempdir().expect("a folder to choose");
    single.answer(vec![chosen.path().to_path_buf()]);
    single.open_subject(SUBJECT);
    single.until("the reader shows the tile", |h| {
        h.count(".reader .att-tile") == 1
    });
    more_menu(&mut single, mail_ui_model::copy::menus::SAVE_ATTACHMENT);
    single.until("the file is written into the folder", |_| {
        chosen.path().join(NAME).exists()
    });
}

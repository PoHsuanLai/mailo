//! Lane 14: the Settings window beside the main one. Keyboard rebinds Archive, refusing a key
//! another action holds, and the main window, open all along, archives on the new key at once
//! and on the shipped one again after Reset; Accounts pushes an account's page, which Escape and
//! ⌘[ come back from and whose Remove question Escape answers; Spaces are put in order by
//! dragging a row's grip; and every page keeps the button size rule.

use ds::prelude::{Point, Px};
use ds_harness::{Harness, Input};
use mail_ui::ui::copy;
use mail_ui::ui::keymap;
use mail_ui::ui::space;
use mail_ui::ui::view::Shortcut;

use super::drive::{Drive, Key};
use super::hands::{centre, click, until};
use super::look::{inbox_at, sizes_hold};
use super::window::{INBOX, Window, account, ms, row_of};

/// Archive's row on the Keyboard page.
const ARCHIVE: &str = "[*|data-action=Archive]";

/// The account's row on the Accounts page, and its page's back button.
fn account_row() -> String {
    format!("#accounts-pane-open-{}", account())
}
const BACK: &str = "#accounts-pane-back-detail";

/// Whether only the account's page is drawn, its slide done.
fn at_account(settings: &Harness) -> bool {
    settings.count(".ds-pane-stack-page") == 1 && settings.count(BACK) == 1
}

/// Whether only the list of accounts is drawn.
fn at_accounts(settings: &Harness) -> bool {
    settings.count(".ds-pane-stack-page") == 1 && settings.count(BACK) == 0
}

/// The `n`th row (from 1) of the Spaces' own list, the one that can be reordered.
fn space_row(n: usize) -> String {
    format!("[*|data-reorder=\"grip\"] > .ds-list-item:nth-child({n})")
}

/// Press `from`, move to `to` a frame at a time, and release there; `during` looks at the
/// window while the row is held over `to`.
fn drag(settings: &mut Harness, from: Point, to: Point, during: impl FnOnce(&mut Harness)) {
    settings.send(Input::pointer_move(from));
    settings.send(Input::pointer_down(from));
    settings.advance(ms(50));
    for step in 1..=10 {
        let part = step as f32 / 10.0;
        settings.send(Input::pointer_move(Point {
            x: Px(from.x.0 + (to.x.0 - from.x.0) * part),
            y: Px(from.y.0 + (to.y.0 - from.y.0) * part),
        }));
        settings.advance(ms(16));
    }
    during(settings);
    settings.send(Input::pointer_up(to));
    settings.advance(ms(100));
}

#[test]
fn settings_rebinds_a_key_for_the_open_window_pushes_an_account_and_orders_the_spaces() {
    let mut window = Window::open(|_| {});
    let dirs = window.dirs();
    let inbox = |window: &Window| inbox_at(&window.store, chrono::Utc::now()).len();
    window.open_subject(INBOX[0].1);

    // A key another action holds is refused, and nothing is kept; a free one is.
    window.in_settings(copy::titles::KEYBOARD, |settings| {
        sizes_hold(settings, copy::titles::KEYBOARD);
        let recorder = format!("{ARCHIVE} .ds-shortcut-field-well");
        click(settings, &recorder);
        settings.key(Key::Char('s'));
        until(settings, "the key is refused", |h| {
            h.count(".ds-alert") == 1
        });
        settings.key(Key::Escape);
        until(settings, "Escape closes the refusal", |h| {
            h.count(".ds-alert") == 0
        });
        assert!(
            !dirs.config.join(keymap::FILE_NAME).exists(),
            "a refused key was kept"
        );
        click(settings, &recorder);
        settings.key(Key::Char('x'));
        until(settings, "the free key is kept", |_| {
            keymap::load(&dirs.config).keys(Shortcut::Archive) == ["x"]
        });
    });

    // The main window, open all along: the old key does nothing, and the new one archives.
    window.press(&[], Key::Char('e'), 1);
    window.harness.advance(ms(600));
    assert_eq!(inbox(&window), INBOX.len(), "the old key still archived");
    window.press(&[], Key::Char('x'), 1);
    window.until("the new key archives", |h| row_of(h, INBOX[0].1).is_none());

    // Reset puts the shipped key back, and with nothing changed there is nothing to restore.
    window.in_settings(copy::titles::KEYBOARD, |settings| {
        click(settings, &format!("{ARCHIVE} .ds-button"));
        until(settings, "Archive is as it ships", |h| {
            h.count(&format!("{ARCHIVE} .ds-button")) == 0
                && !h
                    .text_of(".settings-page")
                    .is_some_and(|page| page.contains(copy::controls::RESTORE_DEFAULTS))
        });
    });
    assert_eq!(keymap::load(&dirs.config), keymap::Keymap::default());
    // The rows below the archived one heal up first.
    window.harness.advance(ms(600));
    window.open_subject(INBOX[1].1);
    window.press(&[], Key::Char('x'), 1);
    window.harness.advance(ms(600));
    assert_eq!(
        inbox(&window),
        INBOX.len() - 1,
        "the reset key still archived"
    );
    window.press(&[], Key::Char('e'), 1);
    window.until("the shipped key archives again", |h| {
        row_of(h, INBOX[1].1).is_none()
    });

    // The account's row pushes its page, the keyboard on its back button; Escape and ⌘[ come
    // back to the row; Escape on the Remove question answers it and stays on the page.
    window.in_settings(copy::titles::ACCOUNTS, |settings| {
        let row = account_row();
        until(settings, "the account is listed", |h| h.count(&row) == 1);
        sizes_hold(settings, copy::titles::ACCOUNTS);
        click(settings, &row);
        until(settings, "its page is pushed", at_account);
        until(settings, "the back button has the keyboard", |h| {
            h.is_focused(BACK)
        });
        settings.key(Key::Escape);
        until(settings, "Escape comes back", at_accounts);
        until(settings, "the row has the keyboard", |h| h.is_focused(&row));
        click(settings, &row);
        until(settings, "its page is pushed again", |h| {
            at_account(h) && h.is_focused(BACK)
        });
        settings.chord(&[Key::Super], Key::Char('['));
        until(settings, "⌘[ comes back", at_accounts);
        click(settings, &row);
        until(settings, "its page is pushed a third time", at_account);
        click(settings, ".acct-actions button");
        until(settings, "Remove asks", |h| h.count(".ds-alert") == 1);
        settings.advance(ms(400));
        settings.key(Key::Escape);
        until(settings, "Escape answers it", |h| h.count(".ds-alert") == 0);
        settings.advance(ms(400));
        assert!(at_account(settings), "Escape went back past the page");
    });

    // Home's grip, dropped on Travel's row: Home goes last, the rows it passes making room.
    let three = space::built(
        ["Home", "Work", "Travel"]
            .into_iter()
            .map(|name| {
                let look = ds::prelude::SpaceLook::default();
                (name.to_owned(), look, space::Mail::default())
            })
            .collect(),
        0,
    );
    space::save(Some(&dirs), &three).expect("the Spaces are written");
    window.in_settings(copy::titles::SPACES, |settings| {
        until(settings, "each Space's row has its grip", |h| {
            h.count(".ds-list-grip") == 3
        });
        let from = centre(settings, &format!("{} .ds-list-grip", space_row(1)));
        let to = centre(settings, &space_row(3));
        drag(settings, from, to, |held| {
            assert_eq!(held.count("[*|data-drag=\"lifted\"]"), 1, "no row lifted");
            assert_eq!(held.count("[*|data-drag=\"moved\"]"), 2, "no room made");
        });
        until(settings, "the list is redrawn in its new order", |h| {
            h.count("[*|data-drag]") == 0
        });
        // The row itself is still a press that opens the Space's page.
        click(settings, &format!("{} .ds-row", space_row(1)));
        until(settings, "Work's page opens", |h| {
            h.count("[*|value=\"Work\"]") == 1
        });
    });
    let names: Vec<String> = space::stored(&dirs)
        .expect("the Spaces are stored")
        .list()
        .iter()
        .map(|one| one.name.clone())
        .collect();
    assert_eq!(names, ["Work", "Travel", "Home"]);

    // Every page keeps the button size rule.
    for page in [copy::titles::GENERAL, copy::titles::RULES] {
        window.in_settings(page, |settings| {
            settings.advance(ms(300));
            sizes_hold(settings, page);
        });
    }
}

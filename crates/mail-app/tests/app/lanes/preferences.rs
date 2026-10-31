//! Lane 14: the Settings window beside the main one. Keyboard rebinds Archive, refusing a key
//! another action holds, and the main window, open all along, archives on the new key at once
//! and on the shipped one again after Reset; Accounts pushes an account's page, which Escape and
//! ⌘[ come back from and whose Remove question Escape answers; Spaces are put in order by
//! dragging a row's grip; an account's page names it, pauses it and shows its servers without
//! editing them; and every page keeps the button size rule.

use ds::prelude::{Point, Px};
use ds_harness::{Harness, Input};
use mail_ui_kit::keymap;
use mail_ui_kit::space;
use mail_ui_kit::view::Shortcut;
use mail_ui_model::copy;

use super::drive::{Drive, Key};
use super::hands::{centre, click, scroll_to, type_text, until};
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
        // Remove Account… is the page's last part, below the servers and the folders.
        scroll_to(settings, ".settings-scroll", ".acct-actions button");
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

/// The list's ⋯ menu is kept: snippets hidden and grouping by date are written to
/// `settings.toml`, and the next window opens with them.
#[test]
fn the_list_opens_the_way_it_was_left() {
    use mail_ui_model::settings::{GroupByDate, LabelChips, Snippets, load, root_for, store};
    let options = format!("[aria-label=\"{}\"]", copy::aria::MORE);
    let mut window = Window::open(|_| {});
    window.click(&options);
    super::hands::menu_item(&mut window.harness, copy::menus::HIDE_SNIPPETS);
    super::hands::menu_item(&mut window.harness, copy::menus::GROUP_BY_DATE);
    let kept = load(&root_for(&window.dirs().config));
    assert_eq!(kept.window.snippets, Snippets::Off);
    assert_eq!(kept.window.label_chips, LabelChips::On);
    assert_eq!(kept.window.group_by_date, GroupByDate::On);

    let mut next = Window::open_with(
        chrono::Utc::now(),
        |_| {},
        |dir, contexts| {
            store(root_for(&dir.join("config")))
                .save(&kept)
                .expect("settings written");
            contexts
        },
    );
    next.click(&options);
    until(&mut next.harness, "the menu offers snippets back", |h| {
        super::look::menu_item_of(h, copy::menus::SHOW_SNIPPETS).is_some()
            && super::look::menu_item_of(h, copy::menus::HIDE_LABELS).is_some()
    });
}

/// "Check for mail when quit" on General: off as mailo ships, and each turn of its switch is
/// handed to the login item (a recorder here, never the system's) and kept in `settings.toml`.
#[test]
fn check_for_mail_when_quit_starts_and_stops_the_login_item() {
    let mut window = Window::open(|_| {});
    let dirs = window.dirs();
    let root = mail_ui_model::settings::root_for(&dirs.config);
    let while_quit = move || mail_ui_model::settings::load(&root).sync.while_quit;
    assert_eq!(while_quit(), mail_ui_model::settings::WhileQuit::Off);
    let switch = "[*|aria-label=\"Check for mail when quit\"][*|aria-checked]";
    let login = window.login_item();
    window.in_settings(copy::titles::GENERAL, |settings| {
        until(settings, "the switch is drawn, off", |h| {
            h.count(&format!("{switch}[*|aria-checked=\"false\"]")) == 1
        });
        click(settings, switch);
        until(settings, "the switch is on", |h| {
            h.count(&format!("{switch}[*|aria-checked=\"true\"]")) == 1
        });
        // The login item is asked off the thread that draws.
        until(settings, "the login item is asked", |_| {
            login.asked().len() == 1
        });
    });
    assert_eq!(while_quit(), mail_ui_model::settings::WhileQuit::On);
    assert_eq!(window.login_asked(), ["register"]);
    window.in_settings(copy::titles::GENERAL, |settings| {
        until(settings, "the switch is drawn, on", |h| {
            h.count(&format!("{switch}[*|aria-checked=\"true\"]")) == 1
        });
        click(settings, switch);
        until(settings, "the switch is off", |h| {
            h.count(&format!("{switch}[*|aria-checked=\"false\"]")) == 1
        });
        until(settings, "the login item is asked", |_| {
            login.asked().len() == 2
        });
    });
    assert_eq!(while_quit(), mail_ui_model::settings::WhileQuit::Off);
    assert_eq!(window.login_asked(), ["register", "unregister"]);
}

/// The lane's account as Add Account would have kept one typed by hand: IMAP and SMTP, a
/// password.
fn typed_by_hand() -> mail_domain::AccountPlan {
    use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing, SaslMech, Tls, Username};
    AccountPlan {
        address: super::seed::ME.to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example.test".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

/// Lane 14, the account's page: a description typed there names the account in the main
/// window, the Sync switch pauses it, and its servers are shown as they are, with nothing to
/// type into: editing them is porter's server editor's.
#[test]
fn an_account_s_page_names_pauses_and_shows_its_servers() {
    let plan = typed_by_hand();
    let mut window = Window::open(|store| {
        store
            .set_account_plan(account(), &plan)
            .expect("the plan is written");
    });
    let store = std::sync::Arc::clone(&window.store);
    window.in_settings(copy::titles::ACCOUNTS, |settings| {
        let row = account_row();
        click(settings, &row);
        until(settings, "its page is pushed", at_account);
        sizes_hold(settings, copy::titles::ACCOUNTS);

        click(settings, "#acct-description");
        type_text(settings, "Work mail");
        settings.key(Key::Tab);
        settings.advance(ms(100));
        until(settings, "the description is kept", |_| {
            store
                .account_prefs(account())
                .ok()
                .and_then(|prefs| prefs.description)
                == Some("Work mail".to_owned())
        });

        let switch = format!(
            ".ds-toggle[*|aria-label=\"{}\"]",
            copy::fields::account::SYNC_THIS_ACCOUNT
        );
        click(settings, &switch);
        until(settings, "the account is paused", |h| {
            h.attr(&switch, "aria-checked").as_deref() == Some("false")
                && store
                    .account_prefs(account())
                    .is_ok_and(|prefs| prefs.syncing == mail_core::Syncing::Paused)
        });
        until(settings, "the page says what paused means", |h| {
            h.html().contains(copy::status::account::PAUSED_HELP)
        });

        let page = settings.html();
        assert!(
            page.contains("imap.example.test"),
            "the incoming server is shown: {page}"
        );
        for field in ["#acct-in-host", "#acct-in-port", "#acct-save-servers"] {
            assert_eq!(
                settings.count(field),
                0,
                "{field}: the servers are edited here"
            );
        }
    });
    window.until("the sidebar names the account by its description", |h| {
        h.html().contains("Work mail")
    });
    let kept = window
        .store
        .account(account())
        .expect("the account is read")
        .and_then(|kept| kept.plan.ok());
    assert_eq!(kept, Some(plan), "the page changed the servers");
}

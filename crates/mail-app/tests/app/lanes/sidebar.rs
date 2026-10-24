//! Lane 13: the sidebar. A right click on the account's tile opens its own menu, not the Space's
//! around it; a Space's name opens the Space's menu only on a right click, where Edit Space… asks
//! for its page in Settings and Delete Space… asks first, Escape keeping the Space; a folder's
//! rename ends with Escape, or by going to another folder, writing nothing; a hidden sidebar
//! comes back from the head of the list. Two fingers moving sideways over the sidebar go to the
//! Space beside this one, one a gesture; moving down they scroll its lists, as before.

use ds::base::geometry::units::Px;
use ds::base::press::PointerButton;
use mail_app::ui::appearance::WindowDirs;
use mail_app::ui::copy;
use mail_app::ui::native::{OpenSettings, SettingsAt, SettingsWindows};
use mail_app::ui::space::{self, Mail};
use mail_core::{SqliteStore, Store};
use mail_domain::{Folder, Holds, Subscription, presets};
use std::sync::{Arc, Mutex};

use super::drive::{Drive, Key};
use super::hands::{menu_item, text};
use super::row_menu::menu_names;
use super::window::{Window, account};
use crate::lanes::seed::ME;

/// A folder of the account's, and another beside it.
const PROJECTS: &str = "Projects";
const RECEIPTS: &str = "Receipts";

/// The account as an IMAP account whose folders have been listed.
fn with_folders(store: &SqliteStore) {
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual(ME, &manual, chrono::Utc::now());
    store
        .set_account_plan(account(), &preset.plan)
        .expect("the plan");
    store
        .put_caps(account(), &preset.expected_caps, chrono::Utc::now())
        .expect("the capabilities");
    let folder = |path: &str| Folder {
        account: account(),
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    store
        .put_folders(
            account(),
            vec![folder("INBOX"), folder(PROJECTS), folder(RECEIPTS)],
        )
        .expect("the folders");
}

/// Every place the window asked the Settings window to open at.
#[derive(Default)]
struct Asks(Mutex<Vec<Option<SettingsAt>>>);

impl OpenSettings for Asks {
    fn open(&self, at: Option<SettingsAt>) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(at);
    }
}

/// How many Spaces the foot draws: a dot each, or, with one Space, none.
fn spaces_drawn(window: &Window) -> usize {
    window.harness.count(".ds-spaces-dots > *").max(1)
}

/// Right-click `selector` and wait for a menu.
fn right_click(window: &mut Window, selector: &str) {
    let at = window.centre(selector);
    window.harness.press(at, PointerButton::Secondary);
    window.until("a menu opens", |h| h.count(".ds-menu .ds-menu-item") > 0);
}

/// The open menu's last row's label.
fn last_row(window: &Window) -> String {
    menu_names(&window.harness).pop().unwrap_or_default()
}

#[test]
fn the_sidebar_s_tiles_spaces_and_folders_answer_their_menus() {
    let asks = Arc::new(Asks::default());
    let first = Arc::new(Mutex::new(None));
    let mut window = Window::open_with(chrono::Utc::now(), with_folders, {
        let asks = Arc::clone(&asks);
        let first = Arc::clone(&first);
        move |dir, contexts| {
            let two = space::built(
                (1..=2)
                    .map(|n| {
                        let look = ds::prelude::SpaceLook::default();
                        (format!("Space {n}"), look, Mail::of(vec![account()]))
                    })
                    .collect(),
                0,
            );
            *first.lock().unwrap_or_else(|e| e.into_inner()) = Some(two.current().id);
            let dirs = WindowDirs {
                config: dir.join("config"),
                state: dir.join("state"),
            };
            space::save(Some(&dirs), &two).expect("the Spaces are written");
            contexts.with(SettingsWindows(asks as Arc<dyn OpenSettings>))
        }
    });
    assert_eq!(spaces_drawn(&window), 2, "not two Spaces");

    // The account's tile: its own menu, and not the Space's around it as well.
    let tile = format!(
        "[*|role=\"group\"][*|aria-label=\"{}\"] [*|aria-label=\"{ME}\"]",
        copy::aria::ACCOUNTS_IN_SPACE
    );
    right_click(&mut window, &tile);
    assert_eq!(window.harness.count(".ds-menu"), 1, "two menus opened");
    assert_eq!(
        menu_names(&window.harness),
        [
            copy::menus::SYNC_NOW,
            copy::menus::REMOVE_FROM_SPACE,
            copy::menus::ACCOUNT_SETTINGS
        ]
    );
    window.press(&[], Key::Escape, 1);
    window.until("Escape closes it", |h| h.count(".ds-menu") == 0);

    // The Space's name: a left click opens nothing, a right click its menu, whose Edit Space…
    // asks Settings for the Space's page and opens nothing here.
    let name = ".ds-space-head";
    window.click(name);
    assert_eq!(
        window.harness.count(".ds-menu"),
        0,
        "a left click opened a menu"
    );
    right_click(&mut window, name);
    window.click(".ds-menu-item:first-child");
    window.until("the menu goes", |h| h.count(".ds-menu") == 0);
    assert_eq!(window.harness.count(".ds-sheet"), 0, "a sheet opened here");
    let first = first
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .expect("the first Space");
    assert_eq!(
        *asks.0.lock().unwrap_or_else(|e| e.into_inner()),
        [Some(SettingsAt::Space(first))]
    );

    // Delete Space… asks; Escape keeps the Space; asked again and confirmed, it goes.
    let asking = ".ds-alert[*|data-style=critical]";
    right_click(&mut window, name);
    assert_eq!(last_row(&window), copy::controls::DELETE_SPACE);
    menu_item(&mut window.harness, copy::controls::DELETE_SPACE);
    window.until("it asks", |h| h.count(asking) == 1);
    window.press(&[], Key::Escape, 1);
    window.until("Escape closes the question", |h| h.count(asking) == 0);
    assert_eq!(spaces_drawn(&window), 2, "Escape removed the Space");
    right_click(&mut window, name);
    menu_item(&mut window.harness, copy::controls::DELETE_SPACE);
    window.until("it asks again", |h| h.count(asking) == 1);
    window.click(&format!(
        "{asking} .ds-alert-slot:nth-child(1) > .ds-button"
    ));
    window.until("the Space goes", |h| {
        h.count(asking) == 0 && h.count(".ds-spaces-dots > *") == 0
    });

    // The one Space left offers no Delete.
    right_click(&mut window, name);
    assert_eq!(last_row(&window), copy::controls::NEW_SPACE);
    window.press(&[], Key::Escape, 1);
    window.until("the menu goes", |h| h.count(".ds-menu") == 0);

    // A folder's rename, begun from its menu: Escape ends it, and so does going to another
    // folder, and neither writes the name.
    let renaming = format!(
        ".ds-row input[*|aria-label=\"{}\"]",
        copy::fields::FOLDER_NAME
    );
    let begin = |window: &mut Window| {
        right_click(window, &format!(".ds-row[*|data-place=\"{PROJECTS}\"]"));
        menu_item(&mut window.harness, copy::menus::RENAME);
        window.until("the rename field has its name selected", |h| {
            h.count(".ds-menu") == 0
                && h.is_focused(&renaming)
                && h.selected_text(&renaming).as_deref() == Some(PROJECTS)
        });
    };
    let folders = |window: &Window| -> Vec<String> {
        window
            .store
            .folders(account())
            .expect("the folders")
            .into_iter()
            .map(|folder| folder.path)
            .collect()
    };
    begin(&mut window);
    window.press(&[], Key::Escape, 1);
    window.until("Escape ends it", |h| h.count(&renaming) == 0);
    assert!(folders(&window).contains(&PROJECTS.to_owned()));
    begin(&mut window);
    window.click(&format!(".ds-row[*|data-place=\"{RECEIPTS}\"]"));
    window.until("going elsewhere ends it", |h| h.count(&renaming) == 0);
    assert!(folders(&window).contains(&PROJECTS.to_owned()));

    // Hide Sidebar, a row of the foot's menu, leaves a way back at the head of the list.
    let more = format!(
        ".ds-spaces-foot [*|aria-label=\"{}\"]",
        copy::aria::SIDEBAR_MENU
    );
    let show = format!(".list-head [*|aria-label=\"{}\"]", copy::aria::SHOW_SIDEBAR);
    assert_eq!(
        window.harness.count(&show),
        0,
        "a way back with the sidebar shown"
    );
    window.click(&more);
    menu_item(&mut window.harness, copy::menus::HIDE_SIDEBAR);
    window.until("the sidebar hides", |h| h.count(&show) == 1);
    window.click(&show);
    window.until("the sidebar comes back", |h| {
        h.count(&show) == 0 && h.rect(".side").is_some_and(|r| r.size.width.0 > 100.0)
    });
}

/// The sidebar's head, which names the Space on screen.
const SPACE_HEAD: &str = ".side .ds-space-head";

/// The name of the Space on screen, as the sidebar's head says it.
fn space_on_screen(window: &Window) -> String {
    window.text(SPACE_HEAD)
}

/// Two fingers moving `dx`, `dy` a frame over the sidebar's lists, `frames` times, then the hand
/// still for long enough that the next gesture is a new one.
fn swipe(window: &mut Window, dx: f32, dy: f32, frames: usize) {
    let at = window.centre(".side .slide");
    for _ in 0..frames {
        window.harness.wheel(at, Px(dx), Px(dy));
        window.harness.advance(ms(16));
    }
    window.harness.advance(ms(400));
}

#[test]
fn a_sideways_swipe_over_the_sidebar_moves_one_space_and_a_scroll_down_does_not() {
    let ids = Arc::new(Mutex::new(Vec::new()));
    let mut window = Window::open_with(chrono::Utc::now(), with_folders, {
        let ids = Arc::clone(&ids);
        move |dir, contexts| {
            let two = space::built(
                (1..=2)
                    .map(|n| {
                        let look = ds::prelude::SpaceLook::default();
                        (format!("Space {n}"), look, Mail::of(vec![account()]))
                    })
                    .collect(),
                0,
            );
            *ids.lock().unwrap_or_else(|e| e.into_inner()) =
                two.list().iter().map(|space| space.id).collect();
            let dirs = WindowDirs {
                config: dir.join("config"),
                state: dir.join("state"),
            };
            space::save(Some(&dirs), &two).expect("the Spaces are written");
            contexts
        }
    });
    let ids = ids.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let stored = |window: &Window| space::stored(&window.dirs()).map(|spaces| spaces.current().id);
    assert_eq!(space_on_screen(&window), "Space 1");

    // Down, and down with a drift sideways: the lists', never a switch.
    swipe(&mut window, 0.0, -30.0, 12);
    swipe(&mut window, -4.0, -30.0, 12);
    assert_eq!(
        space_on_screen(&window),
        "Space 1",
        "a scroll down switched"
    );

    // Towards the left, before the first Space: it stretches and stays.
    swipe(&mut window, 30.0, 0.0, 12);
    assert_eq!(
        space_on_screen(&window),
        "Space 1",
        "a Space before the first"
    );

    // The content carried to the left, a long swipe with its momentum after: the next Space,
    // and only that one, as a click on its dot would.
    swipe(&mut window, -30.0, 0.0, 40);
    window.until("the swipe switches", |h| text(h, SPACE_HEAD) == "Space 2");
    assert_eq!(
        stored(&window),
        ids.get(1).copied(),
        "the switch was not kept"
    );
    assert_eq!(
        window.harness.count(".side .slide.a-slide-r"),
        1,
        "it slides in from the right"
    );

    // And back again, the other way.
    swipe(&mut window, 30.0, 0.0, 12);
    window.until("the swipe back switches", |h| {
        text(h, SPACE_HEAD) == "Space 1"
    });
    assert_eq!(stored(&window), ids.first().copied());
    assert_eq!(window.harness.count(".side .slide.a-slide-l"), 1);
    // At rest, nothing is carried.
    assert_eq!(window.harness.count(".side [*|data-swipe]"), 0);
}

//! Lane 7: triage. From the keyboard and the row menu: j and k walk the conversations, s stars,
//! m mutes, e archives, # trashes, and Ctrl-Z takes each back. Rows picked together (Ctrl-A,
//! Shift- and Ctrl-clicks, Shift-J and K) are acted on as one gesture by a key or the selection
//! bar, and one Ctrl-Z takes all of it back. Snooze until puts one away until its time; Move to…
//! offers the account's folders; in Trash, Delete Forever and Empty Trash ask first and offer no
//! Undo after.

use mail_core::Store;
use mail_domain::{
    Change, ChangeId, MailboxRole, Mute, Patch, ProtoOp, Snooze, Star, ThreadSummary,
};
use mail_ui::ui::bin::{Bin, Destroying, Reach, words};
use mail_ui::ui::copy;

use super::drive::{Drive, Key, PRIMARY};
use super::look::{conversation, inbox_at, listed};
use super::row_menu::{menu_names, open_row_menu, press_menu_item, row_action};
use super::window::{INBOX, Window, account, ms, row_of};

/// The list's row for `subject`, as `row_menu` takes it.
fn row_for(window: &Window, subject: &str) -> String {
    let n = row_of(&window.harness, subject).unwrap_or_else(|| panic!("no row reads {subject:?}"));
    item(n)
}

/// The `n`th item of the list.
fn item(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

/// Whether the reader shows `subject`.
fn reading(harness: &ds_harness::Harness, subject: &str) -> bool {
    harness
        .text_of(".reader-head")
        .is_some_and(|head| head.contains(subject))
}

/// The subjects of the account's stored conversations `keep` holds for, newest first.
fn stored_where(window: &Window, keep: impl Fn(&ThreadSummary) -> bool) -> Vec<String> {
    listed(
        &window.store,
        mail_domain::Filter::Account(account()),
        chrono::Utc::now(),
    )
    .into_iter()
    .filter(|thread| keep(thread))
    .map(|thread| thread.subject)
    .collect()
}

/// Click the `n`th row at the start of its subject line with `held` down.
fn click_row(window: &mut Window, n: usize, held: &[Key]) {
    let line = format!("{} .ds-thread-sub", item(n));
    let rect = window
        .harness
        .rect(&line)
        .unwrap_or_else(|| panic!("{line} is not drawn"));
    window.harness.click_with(
        ds::prelude::Point {
            x: ds::prelude::Px(rect.origin.x.0 + 24.0),
            y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
        },
        held,
    );
    window.harness.advance(ms(300));
}

/// Which rows (1-based) are drawn selected, top to bottom.
fn picked(window: &Window) -> Vec<usize> {
    (1..=window.harness.count(".list .ds-thread"))
        .filter(|n| {
            window
                .harness
                .attr(&format!("{} .ds-row", item(*n)), "aria-selected")
                .as_deref()
                == Some("true")
        })
        .collect()
}

/// Whether the list bar counts `count` picked rows.
fn counted(window: &Window, count: usize) -> bool {
    window
        .text(".list-col")
        .contains(&copy::status::selected(count))
}

/// The control whose accessible name is `name`.
fn labelled(name: &str) -> String {
    format!("[*|aria-label=\"{name}\"]")
}

/// The list bar's Empty Trash, which its alert's button is also called.
fn empty_trash() -> String {
    let all = Destroying {
        bin: Bin::Trash,
        reach: Reach::Everything,
        threads: Vec::new(),
        messages: 0,
    };
    words(&all).confirm
}

#[test]
fn the_keyboard_and_the_menus_walk_star_mute_archive_trash_and_undo() {
    let mut window = Window::open(|_| {});
    let [first, second, third, fourth] = INBOX.map(|(_, subject)| subject);
    window.open_subject(first);

    // j and k: the next and the previous conversation open.
    window.press(&[], Key::Char('j'), 1);
    window.until("j opens the second", |h| reading(h, second));
    window.press(&[], Key::Char('j'), 1);
    window.until("j opens the third", |h| reading(h, third));
    window.press(&[], Key::Char('k'), 1);
    window.until("k opens the second again", |h| reading(h, second));

    // s stars the open one.
    window.press(&[], Key::Char('s'), 1);
    window.until_stored("s stars it", |store| {
        conversation(store, second).star == Star::Starred
    });

    // m mutes it and marks its row, leaving it where it is; Ctrl-Z unmutes it.
    window.press(&[], Key::Char('m'), 1);
    window.until_stored("m mutes it", |store| {
        conversation(store, second).mute == Mute::Muted
    });
    window.until("its row shows it is muted", |h| {
        h.count(&format!("{} .mute-mark", item(2))) == 1
    });
    assert_eq!(window.subjects(), [first, second, third, fourth]);
    window.press(&[PRIMARY], Key::Char('z'), 1);
    window.until_stored("Ctrl-Z unmutes it", |store| {
        conversation(store, second).mute != Mute::Muted
    });

    // The reader's ⋯ mutes too, and offers Unmute once it is muted; m then unmutes.
    let more = format!(".reader-head {}", labelled(copy::aria::MORE));
    window.click(&more);
    window.menu_item(copy::menus::MUTE);
    window.until_stored("the reader's Mute mutes it", |store| {
        conversation(store, second).mute == Mute::Muted
    });
    window.click(&more);
    window.until("the ⋯ offers Unmute", |h| {
        menu_names(h).iter().any(|name| name == copy::menus::UNMUTE)
    });
    window.press(&[], Key::Escape, 1);
    window.until("the menu goes", |h| h.count(".ds-menu") == 0);
    window.press(&[], Key::Char('m'), 1);
    window.until_stored("m unmutes it", |store| {
        conversation(store, second).mute != Mute::Muted
    });

    // e archives it, with a toast that goes after its hold; Ctrl-Z brings it back.
    window.press(&[], Key::Char('e'), 1);
    // The row leaves on the press; the toast says it once the archive is written.
    window.until("e archives it", |h| row_of(h, second).is_none());
    window.until("archiving says so", |h| h.count(".ds-toast") == 1);
    let hold = ds::style::tokens::delay::DelayToken::ToastHold.delay();
    window.harness.advance(hold / 2);
    assert_eq!(
        window.harness.count(".ds-toast"),
        1,
        "the toast left before its hold"
    );
    window.harness.advance(hold);
    window.until("the toast goes", |h| h.count(".ds-toast") == 0);
    window.press(&[PRIMARY], Key::Char('z'), 1);
    window.until("Ctrl-Z brings it back", |h| row_of(h, second).is_some());
    assert_eq!(window.subjects(), [first, second, third, fourth]);

    // # moves the open one to the Trash; Ctrl-Z again.
    window.open_subject(third);
    window.press(&[], Key::Char('#'), 1);
    window.until("# trashes it", |h| row_of(h, third).is_none());
    window.press(&[PRIMARY], Key::Char('z'), 1);
    window.until("Ctrl-Z brings it back", |h| row_of(h, third).is_some());
    assert_eq!(window.subjects(), [first, second, third, fourth]);

    // Another row archived from its menu: the keyboard stays the window's, so e still archives
    // the open conversation.
    window.open_subject(first);
    let row = row_for(&window, third);
    row_action(&mut window.harness, &row, copy::menus::ARCHIVE);
    window.until("the menu's row leaves", |h| row_of(h, third).is_none());
    window.until("the window has the keyboard", |h| h.is_focused(".app"));
    window.press(&[], Key::Char('e'), 1);
    window.until("e archives the open one", |h| row_of(h, first).is_none());
    assert_eq!(window.subjects(), [second, fourth]);
}

#[test]
fn rows_picked_together_are_acted_on_as_one_gesture_and_one_undo() {
    let mut window = Window::open(|_| {});
    let subjects: Vec<&str> = INBOX.iter().map(|(_, subject)| *subject).collect();

    // Ctrl-A picks every row, Escape lets them go.
    assert!(picked(&window).is_empty(), "picked at the start");
    window.press(&[PRIMARY], Key::Char('a'), 1);
    assert_eq!(picked(&window), [1, 2, 3, 4]);
    assert!(counted(&window, 4), "the bar does not count them");
    window.press(&[], Key::Escape, 1);
    assert!(picked(&window).is_empty(), "Escape kept them");

    // Picked, then to Starred and back: the selection does not follow.
    window.press(&[PRIMARY], Key::Char('a'), 1);
    window.click("[*|data-place=\"Starred\"]");
    window.click("[*|data-place=\"Inbox\"]");
    window.until("the inbox is back", |h| {
        h.count(".list .ds-thread") == INBOX.len()
    });
    assert!(picked(&window).is_empty(), "the selection followed");

    // Shift-click picks a range from the open row, measured again from the same anchor; a plain
    // click lets it go.
    click_row(&mut window, 2, &[]);
    assert_eq!(picked(&window), [2], "a plain click selects its row");
    click_row(&mut window, 4, &[Key::Shift]);
    assert_eq!(picked(&window), [2, 3, 4], "Shift-click down");
    assert!(counted(&window, 3));
    click_row(&mut window, 1, &[Key::Shift]);
    assert_eq!(picked(&window), [1, 2], "Shift-click up");
    click_row(&mut window, 4, &[]);
    assert_eq!(picked(&window), [4], "a plain click after Shift-click");

    // Ctrl-click adds and takes away one row at a time, and opens nothing.
    click_row(&mut window, 1, &[]);
    click_row(&mut window, 3, &[PRIMARY]);
    click_row(&mut window, 4, &[PRIMARY]);
    assert_eq!(picked(&window), [1, 3, 4], "Ctrl-click adds");
    click_row(&mut window, 4, &[PRIMARY]);
    assert_eq!(picked(&window), [1, 3], "Ctrl-click takes away");
    assert!(reading(&window.harness, subjects[0]), "Ctrl-click opened");

    // The bar's Star and Mute act on both; each is one Ctrl-Z.
    let starred = |window: &Window| stored_where(window, |t| t.star == Star::Starred);
    window.click(&labelled(&copy::aria::on_selected(copy::menus::STAR, 2)));
    window.until_stored("the bar stars both", |_| true);
    assert_eq!(starred(&window), [subjects[0], subjects[2]]);
    window.press(&[PRIMARY], Key::Char('z'), 1);
    assert!(starred(&window).is_empty(), "one Ctrl-Z, both");
    let muted = |window: &Window| stored_where(window, |t| t.mute == Mute::Muted);
    window.click(&labelled(&copy::aria::on_selected(copy::menus::MUTE, 2)));
    assert_eq!(muted(&window), [subjects[0], subjects[2]]);
    window.press(&[PRIMARY], Key::Char('z'), 1);
    assert!(muted(&window).is_empty(), "one Ctrl-Z, both");

    // The bar's last button clears the pick, leaving the open row.
    window.click(&labelled(copy::aria::CLEAR_THE_SELECTION));
    assert_eq!(picked(&window), [1], "Clear left a pick");

    // Shift-J and Shift-K move the end of the range and open nothing on the way.
    click_row(&mut window, 2, &[]);
    window.press(&[Key::Shift], Key::Char('j'), 2);
    assert_eq!(picked(&window), [2, 3, 4], "Shift-J twice");
    window.press(&[Key::Shift], Key::Char('k'), 1);
    assert_eq!(picked(&window), [2, 3], "Shift-K");
    assert!(reading(&window.harness, subjects[1]), "Shift-J opened");

    // e archives all three; one Ctrl-Z puts them all back, and is the whole entry.
    window.press(&[Key::Shift], Key::Char('j'), 1);
    window.press(&[], Key::Char('e'), 1);
    window.until("the three leave", |h| h.count(".list .ds-thread") == 1);
    assert_eq!(inbox_at(&window.store, chrono::Utc::now()), [subjects[0]]);
    window.press(&[PRIMARY], Key::Char('z'), 1);
    window.until("one Ctrl-Z brings all three back", |h| {
        h.count(".list .ds-thread") == INBOX.len()
    });
    assert_eq!(window.subjects(), subjects);
    window.press(&[PRIMARY], Key::Char('z'), 1);
    assert_eq!(
        inbox_at(&window.store, chrono::Utc::now()).len(),
        INBOX.len()
    );
}

/// Snooze `subject` until tomorrow from its row's menu.
fn snooze_until_tomorrow(window: &mut Window, subject: &str) -> chrono::DateTime<chrono::Utc> {
    let row = row_for(window, subject);
    row_action(&mut window.harness, &row, copy::menus::SNOOZE);
    window.menu_item(copy::menus::TOMORROW);
    window.until("the snoozed row leaves the inbox", |h| {
        row_of(h, subject).is_none()
    });
    let Snooze::Until(at) = conversation(&window.store, subject).snooze else {
        panic!("{subject:?} is not snoozed");
    };
    at
}

#[test]
fn snooze_puts_a_conversation_away_until_its_time_and_pop_has_no_folder_to_move_to() {
    let mut window = Window::open(|_| {});
    let subject = INBOX[3].1;
    let at = snooze_until_tomorrow(&mut window, subject);
    let now = chrono::Utc::now();
    assert!(at > now + chrono::Duration::hours(1), "not tomorrow: {at}");
    assert!(at < now + chrono::Duration::hours(48), "not tomorrow: {at}");
    assert!(
        !inbox_at(&window.store, at - chrono::Duration::minutes(1)).contains(&subject.to_owned()),
        "back before its time"
    );
    assert!(
        inbox_at(&window.store, at + chrono::Duration::minutes(1)).contains(&subject.to_owned()),
        "not back once its time has come"
    );

    // A POP3 account has no folders to move to.
    let row = row_for(&window, INBOX[0].1);
    open_row_menu(&mut window.harness, &row);
    press_menu_item(&mut window.harness, copy::menus::MOVE_TO);
    window.until("Move to offers nowhere to move it", |h| {
        h.count(".ds-pick-list") == 1 && h.count(".ds-pick-list-rows .ds-row") == 0
    });
    window.press(&[], Key::Escape, 1);
    assert_eq!(window.subjects().len(), INBOX.len() - 1, "something moved");
}

#[test]
fn a_snoozed_conversation_comes_back_to_the_inbox_when_the_window_s_clock_passes_its_time() {
    let mut window = Window::open(|_| {});
    let subject = INBOX[3].1;
    let at = snooze_until_tomorrow(&mut window, subject);
    let wait = (at - window.now()).to_std().unwrap_or_default();
    window.harness.wait(wait.as_millis() as u64 + 60_000);
    window.until("the conversation is back in the inbox", |h| {
        row_of(h, subject).is_some()
    });
}

/// The deletions forever waiting in the outbox, each as how many server addresses it names.
fn deletions(window: &Window) -> Vec<usize> {
    let far = chrono::Utc::now() + chrono::Duration::days(3650);
    window
        .store
        .outbox_due(account(), far)
        .expect("the outbox")
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::Destroy { remotes } => Some(remotes.len()),
            _ => None,
        })
        .collect()
}

#[test]
fn in_trash_a_row_is_deleted_forever_and_the_trash_emptied_only_once_asked() {
    // The last three conversations already in Trash, as an earlier move left them.
    let mut window = Window::open(|store| {
        let changes = INBOX[1..]
            .iter()
            .flat_map(|(_, subject)| {
                let thread = conversation(store, subject).id;
                store.thread(thread).expect("the conversation").messages
            })
            .map(|message| Change::MessageMailbox(message, MailboxRole::Trash))
            .collect();
        let patch = Patch {
            id: ChangeId::generate(),
            changes,
        };
        store.apply(account(), &patch).expect("filed in Trash");
    });
    let alert = ".ds-alert";
    let in_trash =
        |window: &Window| stored_where(window, |t| t.mailboxes.contains(MailboxRole::Trash)).len();

    // Outside Trash, deleting is a move to Trash and nothing offers Delete Forever.
    open_row_menu(&mut window.harness, &item(1));
    let offered = menu_names(&window.harness);
    assert!(offered.iter().any(|op| op == copy::menus::MOVE_TO_TRASH));
    assert!(!offered.iter().any(|op| op == copy::menus::DELETE_FOREVER));
    window.press(&[], Key::Escape, 1);

    // In Trash, a row's Delete Forever asks first, then takes only that row, with no Undo.
    window.click("[*|data-place=\"Trash\"]");
    window.until("Trash shows its three", |h| {
        h.count(".list .ds-thread") == 3
    });
    row_action(&mut window.harness, &item(2), copy::menus::DELETE_FOREVER);
    window.until("the alert asks", |h| h.count(alert) == 1);
    assert_eq!(in_trash(&window), 3, "deleted before it was answered");
    window.click(&format!("{alert} {}", labelled(copy::aria::DELETE_FOREVER)));
    window.until("the row goes", |h| h.count(".list .ds-thread") == 2);
    assert_eq!(in_trash(&window), 2);
    assert_eq!(
        window.harness.count(".ds-toast-tab"),
        0,
        "an Undo is offered"
    );

    // Empty Trash asks; Escape answers no, and the second ask is answered yes.
    let empty = format!(".list-col {}", labelled(&empty_trash()));
    window.click(&empty);
    window.until("the alert asks", |h| h.count(alert) == 1);
    window.press(&[], Key::Escape, 1);
    window.until("Escape answers no", |h| h.count(alert) == 0);
    assert_eq!(in_trash(&window), 2);
    window.click(&empty);
    window.click(&format!("{alert} {}", labelled(&empty_trash())));
    window.until("the list empties", |h| h.count(".list .ds-thread") == 0);
    assert_eq!(in_trash(&window), 0);
    assert_eq!(
        inbox_at(&window.store, chrono::Utc::now()),
        [INBOX[0].1],
        "the inbox is untouched"
    );
    assert_eq!(deletions(&window), [1, 1, 1], "one per conversation");
    assert_eq!(
        window.harness.count(".ds-toast-tab"),
        0,
        "an Undo is offered"
    );
    window.press(&[PRIMARY], Key::Char('z'), 1);
    assert_eq!(in_trash(&window), 0, "Ctrl-Z took something back");
}

/// A click on a row only opens it, wherever on the row it lands: at 70% of its width, where a
/// strip of actions once stood and a pointer that had just arrived clicked Archive or Trash, the
/// conversation opens and nothing leaves the list. A right click only opens the row's menu, at
/// the pointer: the conversation neither leaves nor opens.
#[test]
fn a_click_on_a_row_opens_it_and_a_right_click_only_opens_its_menu_at_the_pointer() {
    use ds::prelude::{Point, Px};
    let mut window = Window::open(|_| {});
    let first = window.text(&format!("{} .ds-thread-sub", item(1)));
    assert!(!first.is_empty(), "the first row reads no subject");
    let first = first.as_str();
    let rows = window.subjects();
    let any_open = |window: &Window| INBOX.iter().any(|(_, s)| reading(&window.harness, s));
    assert!(
        !any_open(&window),
        "a conversation is open before any click"
    );

    // A right click: the menu, at the pointer, and nothing else.
    let line = format!("{} .ds-thread-sub", item(1));
    let at = window.centre(&line);
    window
        .harness
        .press(at, ds::base::press::PointerButton::Secondary);
    window.until("the right click opens the row's menu", |h| {
        h.count(".ds-menu .ds-menu-item") > 0
    });
    let menu = window.harness.rect(".ds-menu").expect("the menu is drawn");
    assert!(
        (menu.origin.x.0 - at.x.0).abs() < 24.0,
        "the menu {menu:?} does not hang at the pointer {at:?}"
    );
    window.harness.advance(ms(300));
    assert_eq!(window.subjects(), rows, "the right click acted on a row");
    assert!(
        !any_open(&window),
        "the right click opened the conversation"
    );
    window.press(&[], Key::Escape, 1);
    window.until("Escape closes the menu", |h| h.count(".ds-menu") == 0);

    // A click at 70% of the row's width opens it, and acts on nothing.
    let row = window
        .harness
        .rect(&item(1))
        .expect("the first row is drawn");
    let at = Point {
        x: Px(row.origin.x.0 + row.size.width.0 * 0.7),
        y: Px(row.origin.y.0 + row.size.height.0 / 2.0),
    };
    window.harness.pointer_move(at);
    window.harness.click(at);
    window.until("the click opens the conversation", |h| reading(h, first));
    window.harness.advance(ms(300));
    assert_eq!(window.subjects(), rows, "the click acted on a row");
    assert_eq!(
        stored_where(&window, |thread| thread.subject == first).len(),
        1,
        "the click moved the conversation out of the account"
    );
}

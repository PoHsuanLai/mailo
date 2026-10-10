//! Lane 7: triage from the keyboard and the row menu. j and k walk the conversations, s stars,
//! e archives, # trashes, Ctrl-Z takes each back; Snooze until puts one away until its time;
//! Move to… offers the account's folders.

use ds_harness::Query;
use mail_domain::{Snooze, Star};

use super::drive::{Drive, Key, PRIMARY};
use super::look::{conversation, inbox_at};
use super::row_menu::{open_row_menu, press_menu_item, row_action};
use super::window::{INBOX, Window, row_of};

/// The list's row for `subject`, as `row_menu` takes it.
fn row_for(window: &Window, subject: &str) -> String {
    let n = row_of(&window.harness, subject).unwrap_or_else(|| panic!("no row reads {subject:?}"));
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

/// Whether the reader shows `subject`.
fn reading(harness: &ds_harness::Harness, subject: &str) -> bool {
    harness
        .text_of(".reader-head")
        .is_some_and(|head| head.contains(subject))
}

#[test]
fn j_k_star_archive_trash_and_undo_walk_and_sort_the_inbox() {
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

    // e archives it; Ctrl-Z brings it back.
    window.press(&[], Key::Char('e'), 1);
    window.until("e archives it", |h| row_of(h, second).is_none());
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
}

/// Snooze `subject` until tomorrow from its row's menu.
fn snooze_until_tomorrow(window: &mut Window, subject: &str) -> chrono::DateTime<chrono::Utc> {
    let row = row_for(window, subject);
    row_action(&mut window.harness, &row, "Snooze…");
    window.until("Snooze until opens", |h| {
        h.text_of(".ds-menu")
            .is_some_and(|menu| menu.contains("Tomorrow"))
    });
    press_menu_item(&mut window.harness, "Tomorrow");
    window.until("the snoozed row leaves the inbox", |h| {
        row_of(h, subject).is_none()
    });
    let Snooze::Until(at) = conversation(&window.store, subject).snooze else {
        panic!("{subject:?} is not snoozed");
    };
    at
}

#[test]
fn snooze_until_tomorrow_puts_a_conversation_away_until_its_time() {
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
}

#[test]
#[ignore = "gap: snoozing reads the system clock (ui/menus/mod.rs:171) and the list asks the store at Utc::now() only when the revision moves (ui/list_query.rs:64,76); no timer brings a snoozed conversation back when its time comes"]
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

#[test]
fn move_to_offers_no_folder_on_an_account_without_folders() {
    let mut window = Window::open(|_| {});
    let row = row_for(&window, INBOX[0].1);
    open_row_menu(&mut window.harness, &row);
    press_menu_item(&mut window.harness, "Move to…");
    window.until("Move to says there is nowhere to move it", |h| {
        h.text_of(".ds-pick-list")
            .is_some_and(|list| list.contains("No folders"))
    });
    window.press(&[], Key::Escape, 1);
    assert_eq!(window.subjects().len(), INBOX.len(), "something moved");
}

use super::*;
use ds::components::app::spaces::Epoch;
use mail_domain::{DraftId, ThreadId};
use uuid::Uuid;

fn thread(n: u128) -> ThreadId {
    ThreadId::from_uuid(Uuid::from_u128(n))
}

fn draft(n: u128) -> DraftId {
    DraftId::from_uuid(Uuid::from_u128(n))
}

fn recent(n: u128) -> Recent {
    Recent {
        thread: thread(n),
        title: format!("Conversation {n}"),
        face: face('A', AvatarTone::Account(PersonSwatch::nth(0).colour())),
    }
}

/// The menu's rows as a person reads them: a heading as `# Name`, a rule as `-`.
fn read(rows: &[MenuItem<Pick>]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            MenuItem::Item { title, .. } => title.clone(),
            MenuItem::Header(title) => format!("# {title}"),
            MenuItem::Separator => "-".to_owned(),
            MenuItem::Submenu { title, .. } => title.clone(),
            MenuItem::Info { title, .. } => title.clone(),
        })
        .collect()
}

fn bare() -> Foot {
    Foot {
        today: Vec::new(),
        parked: Vec::new(),
        waiting: Vec::new(),
        sidebar: Shown::Visible,
    }
}

#[test]
fn the_menu_lists_at_most_eight_of_todays_newest_first_and_clear_today_under_them() {
    assert_eq!(TODAY_SHOWN, 8);
    // Nine open conversations, newest first as `gather` hands them over.
    let nine = Foot {
        today: (1..=9).map(recent).collect(),
        ..bare()
    };
    let rows = read(&menu_items(&nine));
    assert_eq!(
        &rows[..11],
        [
            "# Today",
            "Conversation 1",
            "Conversation 2",
            "Conversation 3",
            "Conversation 4",
            "Conversation 5",
            "Conversation 6",
            "Conversation 7",
            "Conversation 8",
            "Clear Today",
            "-",
        ],
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row == "Conversation 9"),
        "the ninth is shown: {rows:?}"
    );
}

#[test]
fn todays_conversations_are_ordered_newest_first_by_when_they_were_opened() {
    let live: Vec<(ThreadId, Epoch)> = vec![
        (thread(1), Epoch(100)),
        (thread(2), Epoch(300)),
        (thread(3), Epoch(200)),
    ];
    assert_eq!(newest_first(live), [thread(2), thread(3), thread(1)]);
}

#[test]
fn nothing_in_today_means_no_heading_and_no_clear() {
    let rows = read(&menu_items(&bare()));
    assert_eq!(
        rows,
        [
            "Show all history",
            "-",
            "New Space",
            "Settings\u{2026}",
            "Hide sidebar",
        ]
    );
}

#[test]
fn drafts_put_aside_and_messages_waiting_stay_reachable_under_their_own_headings() {
    let foot = Foot {
        today: vec![recent(1)],
        parked: vec![(draft(7), "Offsite".to_owned())],
        waiting: vec![(draft(8), "Friday".to_owned(), "Tomorrow 08:00".to_owned())],
        sidebar: Shown::Hidden,
    };
    let rows = menu_items(&foot);
    assert_eq!(
        read(&rows),
        [
            "# Today",
            "Conversation 1",
            "Clear Today",
            "-",
            "# Drafts put aside",
            "Offsite",
            "-",
            "# Waiting to be sent",
            "Cancel sending Friday",
            "-",
            "Show all history",
            "-",
            "New Space",
            "Settings\u{2026}",
            "Show sidebar",
        ]
    );
    let picks: Vec<Pick> = rows
        .into_iter()
        .filter_map(|row| match row {
            MenuItem::Item { value, .. } => Some(value),
            _ => None,
        })
        .collect();
    assert_eq!(
        picks,
        [
            Pick::Thread(thread(1)),
            Pick::ClearToday,
            Pick::Draft(draft(7)),
            Pick::CancelSend(draft(8)),
            Pick::History,
            Pick::NewSpace,
            Pick::Settings,
            Pick::ToggleSidebar,
        ]
    );
}

#[test]
fn settings_and_the_sidebar_toggle_show_their_keys_and_the_rest_show_none() {
    let rows = menu_items(&bare());
    let keyed: Vec<(String, bool)> = rows
        .into_iter()
        .filter_map(|row| match row {
            MenuItem::Item { title, key, .. } => Some((title, key.is_some())),
            _ => None,
        })
        .collect();
    assert_eq!(
        keyed,
        [
            ("Show all history".to_owned(), false),
            ("New Space".to_owned(), false),
            ("Settings\u{2026}".to_owned(), true),
            ("Hide sidebar".to_owned(), true),
        ]
    );
}

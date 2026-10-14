use super::*;
use crate::ui::today::at;
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
        face: face('A', 0),
    }
}

/// The menu's rows as a person reads them.
fn read(rows: &[MenuItem<TodayPick>]) -> Vec<String> {
    rows.iter()
        .map(|row| match row {
            MenuItem::Item { title, .. } => title.clone(),
            other => panic!("not an item: {other:?}"),
        })
        .collect()
}

#[test]
fn today_lists_its_conversations_then_drafts_then_messages_waiting() {
    let gathered = Gathered {
        today: vec![recent(1), recent(2)],
        parked: vec![(draft(7), "Offsite".to_owned())],
        scheduled: vec![(draft(8), "Friday".to_owned(), "Tomorrow 08:00".to_owned())],
    };
    let listed: Vec<(TodayKey, String, Option<String>)> = rows(&gathered)
        .into_iter()
        .map(|(key, title, _, when)| (key, title, when))
        .collect();
    assert_eq!(
        listed,
        [
            (
                TodayKey::Thread(thread(1)),
                "Conversation 1".to_owned(),
                None
            ),
            (
                TodayKey::Thread(thread(2)),
                "Conversation 2".to_owned(),
                None
            ),
            (TodayKey::Draft(draft(7)), "Offsite".to_owned(), None),
            (
                TodayKey::Scheduled(draft(8)),
                "Friday".to_owned(),
                Some("Tomorrow 08:00".to_owned())
            ),
        ]
    );
    assert!(
        rows(&Gathered::default()).is_empty(),
        "an empty Today draws nothing"
    );
}

/// Clear Today is Today's only removal, in its right-click menu; a message waiting is taken back
/// only by an item that says so.
#[test]
fn the_right_click_menu_clears_today_and_takes_a_message_back_by_name() {
    let gathered = Gathered {
        today: vec![recent(1)],
        parked: vec![(draft(7), "Offsite".to_owned())],
        scheduled: vec![(draft(8), "Friday".to_owned(), "Tomorrow 08:00".to_owned())],
    };
    assert_eq!(
        read(&menu_for(&TodayKey::Thread(thread(1)), &gathered)),
        ["Clear Today"]
    );
    assert_eq!(
        read(&menu_for(&TodayKey::Draft(draft(7)), &gathered)),
        ["Clear Today"]
    );
    assert_eq!(
        read(&menu_for(&TodayKey::Scheduled(draft(8)), &gathered)),
        [
            copy::menus::cancel_sending("Friday"),
            "Clear Today".to_owned()
        ]
    );
    let nothing_opened = Gathered {
        today: Vec::new(),
        ..gathered
    };
    assert!(
        menu_for(&TodayKey::Draft(draft(7)), &nothing_opened).is_empty(),
        "Clear Today with nothing to clear"
    );
}

#[test]
fn todays_conversations_are_ordered_newest_first_by_when_they_were_opened() {
    let opened: Vec<(ThreadId, Epoch)> = vec![
        (thread(1), Epoch(100)),
        (thread(2), Epoch(300)),
        (thread(3), Epoch(200)),
    ];
    assert_eq!(newest_first(opened), [thread(2), thread(3), thread(1)]);
}

/// Nothing leaves Today by time: a conversation opened two days ago is still listed.
#[test]
fn a_conversation_opened_two_days_ago_is_still_in_today() {
    let (store, _dir) = crate::ui::fixtures::seeded();
    let hi = crate::ui::fixtures::thread_like(&store, "hi");
    let space = SpaceId(0);
    let mut today = Today::default();
    let long_ago = chrono::Utc::now() - chrono::Duration::hours(48);
    today.opened(space, hi, at(long_ago));
    let listed: Vec<ThreadId> = gather(&store, &today, space)
        .today
        .into_iter()
        .map(|recent| recent.thread)
        .collect();
    assert_eq!(listed, [hi]);
    assert!(
        gather(&store, &today, SpaceId(1)).today.is_empty(),
        "another Space's Today"
    );
}

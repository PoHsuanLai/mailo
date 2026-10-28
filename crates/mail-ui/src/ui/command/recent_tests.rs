use super::{SHOWN, draft_key, draft_of, rows, thread_key, thread_of};
use crate::ui::fixtures::{acct_account, inbox_query, seeded, work};
use crate::ui::space::SpaceId;
use crate::ui::today::{Today, at};
use chrono::{TimeZone, Utc};
use mail_core::SqliteStore;
use mail_core::Store;
use mail_core::when::Clock;
use mail_domain::{Address, Draft, DraftId, ThreadId};

fn time(minute: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + minute * 60, 0).unwrap()
}

/// What was opened last comes first, the drafts put aside after, and only this Space's.
#[test]
fn recent_lists_this_spaces_conversations_newest_first_then_its_drafts() {
    let built = work();
    let store = built.store.clone();
    let threads: Vec<ThreadId> = store
        .threads(&inbox_query(), Utc::now())
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    let (older, newer, elsewhere) = (threads[0], threads[1], threads[2]);
    let draft = DraftId::generate();
    // Not in the store: listed only if it were, so it is left out.
    let mut today = Today::default();
    today.opened(SpaceId(0), older, at(time(0)));
    today.opened(SpaceId(0), newer, at(time(5)));
    today.opened(SpaceId(1), elsewhere, at(time(9)));
    today.park(SpaceId(0), draft, "Plans", at(time(7)));

    let keys: Vec<String> = rows(
        &crate::ui::fixtures::client(&store),
        &today,
        SpaceId(0),
        time(10),
        Clock::H24,
    )
    .into_iter()
    .map(|row| row.key)
    .collect();
    assert_eq!(keys, [thread_key(newer), thread_key(older)]);
    let _ = draft;
}

/// Nothing opened and nothing put aside: Recent lists nothing of its own.
#[test]
fn recent_is_empty_with_nothing_opened() {
    let (store, _dir) = seeded();
    assert!(
        rows(
            &crate::ui::fixtures::client(&store),
            &Today::default(),
            SpaceId(0),
            time(10),
            Clock::H24
        )
        .is_empty()
    );
}

/// However much was opened, Recent lists a handful; the rest are a search away.
#[test]
fn recent_lists_at_most_a_handful_of_conversations() {
    let built = work();
    let store = built.store.clone();
    let threads: Vec<ThreadId> = store
        .threads(&inbox_query(), Utc::now())
        .unwrap()
        .items
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert!(threads.len() > SHOWN, "the fixture has too few threads");
    let mut today = Today::default();
    for (minute, thread) in threads.iter().enumerate() {
        today.opened(SpaceId(0), *thread, at(time(minute as i64)));
    }
    assert_eq!(
        rows(
            &crate::ui::fixtures::client(&store),
            &today,
            SpaceId(0),
            time(10),
            Clock::H24
        )
        .len(),
        SHOWN
    );
}

/// A row's key reads back as what it stands for.
#[test]
fn recent_keys_read_back() {
    let thread = ThreadId::generate();
    let draft = DraftId::generate();
    let key = thread_key(thread);
    assert_eq!(
        thread_of(key.strip_prefix("recent:").unwrap()),
        Some(thread)
    );
    let key = draft_key(draft);
    assert_eq!(draft_of(key.strip_prefix("draft:").unwrap()), Some(draft));
    assert_eq!(thread_of("not an id"), None);
}

/// A draft put aside in `store` with `subject`, `to` and `body`, parked at `minute`.
fn parked(
    store: &SqliteStore,
    today: &mut Today,
    subject: &str,
    to: &[Address],
    body: &str,
    minute: i64,
) -> Draft {
    let draft =
        mail_core::compose::draft_new(store, acct_account(), to, subject, body, time(minute))
            .unwrap_or_else(|why| panic!("a draft: {why}"));
    today.park(SpaceId(0), draft.id, "Draft", at(time(minute)));
    draft
}

/// Each draft's row, newest put aside first, is named by what is in it, its subject, else who it goes to, else its
/// first words, and says it is a draft and when it was last touched. A draft with nothing in it,
/// or one no longer in the store, has no row.
#[test]
fn each_draft_is_named_by_what_is_in_it_and_an_empty_or_gone_one_is_left_out() {
    let (store, _dir) = seeded();
    let mut today = Today::default();
    let ada = Address {
        name: Some("Ada Lovelace".to_owned()),
        email: "ada@example.test".to_owned(),
    };
    let long = "A note about the offsite that runs on well past forty characters";
    let named = parked(&store, &mut today, "Offsite plans", &[], "", 1);
    let addressed = parked(&store, &mut today, "", std::slice::from_ref(&ada), "", 2);
    let written = parked(&store, &mut today, "", &[], long, 3);
    let _empty = parked(&store, &mut today, "", &[], "", 4);
    let gone = parked(&store, &mut today, "Gone", &[], "", 5);
    mail_core::compose::discard(&store, gone.id).unwrap_or_else(|why| panic!("{why}"));

    let got: Vec<(String, String, Option<String>)> = rows(
        &crate::ui::fixtures::client(&store),
        &today,
        SpaceId(0),
        time(10),
        Clock::H24,
    )
    .into_iter()
    .map(|row| (row.key, row.name, row.help))
    .collect();
    let when = |draft: &Draft| {
        Some(format!(
            "Draft \u{b7} {}",
            mail_core::when::listed(draft.updated, time(10), &chrono::Local, Clock::H24)
        ))
    };
    // Newest put aside first.
    assert_eq!(
        got,
        [
            (
                draft_key(written.id),
                "A note about the offsite that runs on we\u{2026}".to_owned(),
                when(&written)
            ),
            (
                draft_key(addressed.id),
                "To Ada Lovelace".to_owned(),
                when(&addressed)
            ),
            (
                draft_key(named.id),
                "Offsite plans".to_owned(),
                when(&named)
            ),
        ]
    );
}

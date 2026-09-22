//! Putting a conversation off, through the store rather than only through the predicate.
//!
//! `Op::SetSnooze` and `Filter::Snoozed`/`SnoozeDue` had been in the domain and answerable by
//! SQLite since phase 1, and nothing could set one. What matters here is not that the op applies
//! — it is that the conversation *leaves the inbox* and *comes back on its own*, which is the
//! whole of what snoozing means and the part a pure predicate test cannot show.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

#[path = "../src/snooze.rs"]
mod snooze;
#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 22, 6, 0, 0).unwrap()
}

fn seeded() -> (SqliteStore, tempfile::TempDir, ThreadId) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    absorb(
        &store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: "u1".to_owned(),
            },
            raw: b"From: ada@example.test\r\nSubject: deal with this later\r\n\
                   Message-ID: <l@example.test>\r\n\r\nnot now\r\n"
                .to_vec(),
        }],
        false,
        now(),
    )
    .unwrap();
    let thread = listed(&store, Filter::All, now())[0];
    (store, dir, thread)
}

fn query(filter: Filter) -> Query {
    Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    }
}

fn listed(store: &SqliteStore, filter: Filter, at: DateTime<Utc>) -> Vec<ThreadId> {
    store
        .threads(&query(filter), at)
        .unwrap()
        .items
        .into_iter()
        .map(|t| t.id)
        .collect()
}

/// The inbox as the shell and the CLI list it.
///
/// Asked for rather than restated. The first version of this spelled the filter out here, and
/// so tested a filter the test had written: the CLI was still listing snoozed conversations and
/// every one of these passed. Running `mailo list` found it in a second.
fn inbox() -> Filter {
    view::place_filter(MailboxRole::Inbox)
}

#[test]
fn a_snoozed_conversation_leaves_the_inbox_and_returns_by_itself() {
    let (store, _dir, thread) = seeded();
    assert_eq!(
        listed(&store, inbox(), now()),
        vec![thread],
        "it starts here"
    );

    snooze::snooze(&store, thread, "tomorrow", now()).unwrap();

    // Gone from the inbox, findable in Snoozed — at the same instant.
    assert!(
        listed(&store, inbox(), now()).is_empty(),
        "still in the inbox"
    );
    assert_eq!(listed(&store, view::pending_snooze(), now()), vec![thread]);

    // And back on its own. Nothing ran in between: `SnoozeDue` is resolved against `now` at
    // query time, so the conversation returns whether the client was awake or not.
    let later = now() + TimeDelta::try_days(2).unwrap();
    assert_eq!(
        listed(&store, inbox(), later),
        vec![thread],
        "it did not come back"
    );
    assert!(
        listed(&store, view::pending_snooze(), later).is_empty(),
        "a due conversation is still listed as snoozed"
    );
}

#[test]
fn waking_one_brings_it_back_before_its_hour() {
    let (store, _dir, thread) = seeded();
    snooze::snooze(&store, thread, "weekend", now()).unwrap();
    assert!(listed(&store, inbox(), now()).is_empty());

    snooze::wake(&store, thread, now()).unwrap();

    assert_eq!(listed(&store, inbox(), now()), vec![thread]);
    assert!(listed(&store, view::pending_snooze(), now()).is_empty());
}

#[test]
fn what_it_says_names_the_hour_it_chose() {
    // The one thing the user checks: that "tomorrow" meant what they thought.
    let (store, _dir, thread) = seeded();
    let out = snooze::snooze(&store, thread, "tomorrow", now()).unwrap();
    assert!(out.starts_with("snoozed until "), "{out}");
    assert!(out.contains("2026-09-23"), "{out}");
}

#[test]
fn a_time_it_does_not_understand_changes_nothing() {
    // The failure must not be half-applied: a conversation that stayed in the inbox is better
    // than one that vanished to an hour nobody chose.
    let (store, _dir, thread) = seeded();
    let why = snooze::snooze(&store, thread, "when pigs fly", now()).unwrap_err();
    assert!(why.contains("tomorrow"), "{why}");
    assert_eq!(listed(&store, inbox(), now()), vec![thread]);
}

#[test]
fn snoozing_does_not_archive_it() {
    // Snooze is thread-level state, not a move. A conversation that came back into a folder it
    // was never in would be a different bug wearing the same button.
    let (store, _dir, thread) = seeded();
    snooze::snooze(&store, thread, "+2h", now()).unwrap();
    let later = now() + TimeDelta::try_hours(3).unwrap();
    assert_eq!(
        listed(&store, Filter::InMailbox(MailboxRole::Inbox), later),
        vec![thread]
    );
    assert!(listed(&store, Filter::InMailbox(MailboxRole::Archive), later).is_empty());
}

/// The two surfaces must agree about what a place holds.
mod one_definition {
    use super::*;

    #[test]
    fn the_shell_and_the_command_list_the_same_inbox() {
        // They did not. `mailo list` built `Filter::InMailbox(Inbox)` of its own, so the day the
        // window learned to hide a snoozed conversation the command did not — and the tests
        // agreed with the window because they spelled its filter out themselves.
        let from_the_sidebar = view::default_places()
            .into_iter()
            .find(|p| p.name == "Inbox")
            .expect("an Inbox place")
            .source;
        assert_eq!(
            from_the_sidebar,
            view::Source::Mail(view::place_filter(MailboxRole::Inbox))
        );
    }

    #[test]
    fn every_other_place_is_still_just_its_mailbox() {
        // The inbox is the only one that hides anything. A conversation you snoozed is not a
        // conversation you lost, so Archive, Sent, Spam and Trash show everything.
        for role in [
            MailboxRole::Archive,
            MailboxRole::Sent,
            MailboxRole::Spam,
            MailboxRole::Trash,
        ] {
            assert_eq!(view::place_filter(role), Filter::InMailbox(role));
        }
    }

    #[test]
    fn a_snoozed_conversation_is_gone_from_the_command_too() {
        // The end-to-end version of the same thing, through the filter the CLI actually uses.
        let (store, _dir, thread) = seeded();
        snooze::snooze(&store, thread, "tomorrow", now()).unwrap();
        assert!(
            listed(&store, view::place_filter(MailboxRole::Inbox), now()).is_empty(),
            "`mailo list` would still show it"
        );
    }
}

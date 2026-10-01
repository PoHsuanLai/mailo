use super::*;
use crate::fetch::{First, Live};
use chrono::TimeZone;
use mail_domain::SyncCursor;

const NEW: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const KNOWN: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));
const GONE: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a3"));

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
}

fn every() -> Duration {
    Duration::from_secs(300)
}

fn empty_store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    (SqliteStore::in_memory(dir.path()).unwrap(), dir)
}

fn stamped(store: &SqliteStore, account: AccountId, path: &str, at: &str) {
    // The stamp belongs to an account the store has.
    store
        .connection()
        .execute(
            "INSERT OR IGNORE INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'x@example.test', '{}', datetime('now'))",
            [account.to_string()],
        )
        .unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO sync_state (account, mailbox, cursor, synced_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                account.to_string(),
                path,
                serde_json::to_string(&SyncCursor::Pop).unwrap(),
                at
            ],
        )
        .unwrap();
}

#[test]
fn an_account_the_store_knows_nothing_of_begins_fresh() {
    let (store, _dir) = empty_store();
    assert_eq!(last_synced(&store, NEW), None);
    assert_eq!(probe(&store, NEW, now()), Link::Fresh);
    assert_eq!(Link::Fresh.first(), First::Yes);
}

#[test]
fn an_account_with_a_recorded_pass_begins_current_as_of_it() {
    let (store, _dir) = empty_store();
    stamped(&store, KNOWN, "INBOX", "2026-10-01 11:55:30");
    // The newest of its mailboxes' stamps, not the first.
    stamped(&store, KNOWN, "Sent", "2026-10-01 11:58:00");
    let at = Utc.with_ymd_and_hms(2026, 10, 1, 11, 58, 0).unwrap();
    assert_eq!(last_synced(&store, KNOWN), Some(at));
    assert_eq!(
        probe(&store, KNOWN, now()),
        Link::Current {
            at,
            trouble: vec![],
            live: Live::Polling
        }
    );
    // Another account's stamp is not this one's.
    assert_eq!(probe(&store, NEW, now()), Link::Fresh);
}

#[test]
fn mail_with_no_stamp_is_a_fetched_account_current_as_of_now() {
    let (store, _dir) = crate::ui::fixtures::seeded();
    let link = probe(&store, crate::ui::fixtures::ACCOUNT, now());
    assert_eq!(
        link,
        Link::Current {
            at: now(),
            trouble: vec![],
            live: Live::Polling
        }
    );
}

#[test]
fn the_table_of_what_a_link_begins_as() {
    let earlier = now() - chrono::TimeDelta::minutes(3);
    let later = now() + chrono::TimeDelta::minutes(3);
    let current = |at| Link::Current {
        at,
        trouble: vec![],
        live: Live::Polling,
    };
    let cases: [(&str, Option<DateTime<Utc>>, bool, Link); 5] = [
        ("nothing at all", None, false, Link::Fresh),
        ("mail but no time", None, true, current(now())),
        ("a time", Some(earlier), false, current(earlier)),
        ("a time and mail", Some(earlier), true, current(earlier)),
        // A clock that went backwards must not make the account "updated in the future".
        ("a time from the future", Some(later), true, current(now())),
    ];
    for (name, last, mail, expected) in cases {
        assert_eq!(link_for(last, mail, now()), expected, "{name}");
    }
}

#[test]
fn local_only_accounts_have_no_link_because_they_are_not_listed() {
    // `initial` makes a link for each account it is given, and it is given the ones with a
    // server (`sync::due::intervals` leaves out the ones that keep mail here).
    let (store, _dir) = empty_store();
    let links = initial(&store, &[(NEW, every()), (KNOWN, every())], now());
    assert_eq!(links.keys().copied().collect::<Vec<_>>(), [NEW, KNOWN]);
    assert!(initial(&store, &[], now()).is_empty());
}

/// What happened, the accounts that exist after it, and what the links do about it.
type Case = (&'static str, Vec<(AccountId, Duration)>, Changes);

#[test]
fn the_links_follow_the_accounts_as_they_come_and_go() {
    let known: BTreeSet<AccountId> = [KNOWN, GONE].into();
    let cases: [Case; 4] = [
        (
            "nothing changed",
            vec![(KNOWN, every()), (GONE, every())],
            Changes::default(),
        ),
        (
            "one added",
            vec![(KNOWN, every()), (GONE, every()), (NEW, every())],
            Changes {
                added: vec![NEW],
                removed: vec![],
            },
        ),
        (
            "one removed",
            vec![(KNOWN, every())],
            Changes {
                added: vec![],
                removed: vec![GONE],
            },
        ),
        (
            "swapped",
            vec![(KNOWN, every()), (NEW, every())],
            Changes {
                added: vec![NEW],
                removed: vec![GONE],
            },
        ),
    ];
    for (name, wanted, expected) in cases {
        assert_eq!(reconcile(&known, &wanted), expected, "{name}");
    }
    // A changed interval is not a change of accounts.
    assert_eq!(
        reconcile(&known, &[(KNOWN, Duration::from_secs(60)), (GONE, every())]),
        Changes::default()
    );
}

//! Migration 0028: the record that an account's secrets were moved into porter's store.

use chrono::{TimeZone, Utc};
use mail_store::SqliteStore;
use porter_core::AccountId;

fn account(store: &SqliteStore, id: &str, created: &str) -> AccountId {
    let address = format!("{}@example.test", &id[id.len() - 4..]);
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?3, '{}', ?2)",
            [id, created, &address],
        )
        .unwrap();
    AccountId::parse(id).unwrap()
}

#[test]
fn an_account_is_unadopted_until_it_is_marked_and_marking_twice_keeps_the_first_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let a = account(&store, "00000000-0000-4000-8000-00000000000a", "2026-01-01");
    let b = account(&store, "00000000-0000-4000-8000-00000000000b", "2026-01-02");
    assert_eq!(store.unadopted_accounts().unwrap(), [a.clone(), b.clone()]);
    assert!(!store.secrets_adopted(&a).unwrap());

    let first = Utc.with_ymd_and_hms(2026, 10, 6, 1, 2, 3).unwrap();
    store.mark_secrets_adopted(&a, first).unwrap();
    store
        .mark_secrets_adopted(&a, first + chrono::TimeDelta::try_days(1).unwrap())
        .unwrap();
    assert!(store.secrets_adopted(&a).unwrap());
    assert_eq!(store.unadopted_accounts().unwrap(), [b]);
    let at: String = store
        .connection()
        .query_row("SELECT adopted_at FROM secrets_adopted", [], |r| r.get(0))
        .unwrap();
    assert_eq!(at, "2026-10-06T01:02:03Z");
}

#[test]
fn marking_an_account_the_store_does_not_hold_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let ghost = AccountId::parse("00000000-0000-4000-8000-0000000000ff").unwrap();
    store.mark_secrets_adopted(&ghost, Utc::now()).unwrap();
    assert!(!store.secrets_adopted(&ghost).unwrap());
}

#[test]
fn removing_an_account_forgets_that_it_was_adopted() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let a = account(&store, "00000000-0000-4000-8000-00000000000a", "2026-01-01");
    store.mark_secrets_adopted(&a, Utc::now()).unwrap();
    store.remove_account(a.clone()).unwrap().unwrap();
    assert!(!store.secrets_adopted(&a).unwrap());
    let rows: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM secrets_adopted", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

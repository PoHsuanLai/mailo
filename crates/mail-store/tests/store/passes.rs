//! How each account's passes last ended: a finish clears a failure, a failure keeps the finish,
//! and the row goes with its account.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_store::{PassFailed, Passes};
use porter_core::AccountId;

fn account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b3"))
}

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap() + TimeDelta::minutes(minutes)
}

#[test]
fn a_finish_clears_a_failure_and_a_failure_keeps_the_finish() {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    assert_eq!(store.passes(account()).unwrap(), Passes::default());

    store
        .pass_failed(account(), at(0), "cannot connect")
        .unwrap();
    assert_eq!(
        store.passes(account()).unwrap(),
        Passes {
            finished: None,
            failed: Some(PassFailed {
                at: at(0),
                why: "cannot connect".to_owned(),
            }),
        }
    );

    store.pass_finished(account(), at(5)).unwrap();
    assert_eq!(
        store.passes(account()).unwrap(),
        Passes {
            finished: Some(at(5)),
            failed: None,
        }
    );

    // A later failure says the account has not been fetched since the finish; it does not
    // forget the finish.
    store
        .pass_failed(account(), at(40), "no answer from the server")
        .unwrap();
    let passes = store.passes(account()).unwrap();
    assert_eq!(passes.finished, Some(at(5)));
    assert_eq!(passes.failed.map(|f| f.at), Some(at(40)));
}

#[test]
fn an_account_the_store_does_not_hold_is_not_recorded() {
    let store = mail_store::testing::in_memory();
    store.pass_finished(account(), at(0)).unwrap();
    assert_eq!(store.passes(account()).unwrap(), Passes::default());
}

#[test]
fn the_record_goes_with_its_account() {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "me@example.test");
    store.pass_finished(account(), at(0)).unwrap();
    store.remove_account(account()).unwrap();
    assert_eq!(store.passes(account()).unwrap(), Passes::default());
}

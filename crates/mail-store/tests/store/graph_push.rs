//! A Graph account's push relay and what was set up there: the setting is the person's, a key
//! belongs to the relay it was registered with, a subscription to its key, and the row goes with
//! its account.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_store::{GraphPush, Registered, Subscribed};
use porter_core::AccountId;

fn account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c7"))
}

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap() + TimeDelta::minutes(minutes)
}

fn registered(relay: &str, key: &str) -> Registered {
    Registered {
        relay: relay.to_owned(),
        key: key.to_owned(),
    }
}

#[test]
fn a_relay_is_set_registered_with_and_subscribed_through() {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "me@contoso.example");
    assert_eq!(store.graph_push(account()).unwrap(), GraphPush::default());

    store
        .set_push_relay(account(), Some("https://relay.example.test"))
        .unwrap();
    store
        .push_registered(account(), &registered("https://relay.example.test", "k1"))
        .unwrap();
    let subscribed = Subscribed {
        id: "sub-1".to_owned(),
        expires: at(4230),
    };
    store.push_subscribed(account(), Some(&subscribed)).unwrap();
    assert_eq!(
        store.graph_push(account()).unwrap(),
        GraphPush {
            relay: Some("https://relay.example.test".to_owned()),
            registered: Some(registered("https://relay.example.test", "k1")),
            subscription: Some(subscribed.clone()),
        }
    );

    // Turned off: what was set up is kept for the next watch to let go of.
    store.set_push_relay(account(), None).unwrap();
    let held = store.graph_push(account()).unwrap();
    assert_eq!(held.relay, None);
    assert_eq!(held.subscription, Some(subscribed));

    // A new key forgets the subscription made for the old one.
    store
        .push_registered(account(), &registered("https://other.example.test", "k2"))
        .unwrap();
    let held = store.graph_push(account()).unwrap();
    assert_eq!(
        held.registered,
        Some(registered("https://other.example.test", "k2"))
    );
    assert_eq!(held.subscription, None);

    store.push_subscribed(account(), None).unwrap();
    assert_eq!(store.graph_push(account()).unwrap().subscription, None);
}

#[test]
fn an_account_the_store_does_not_hold_is_not_recorded() {
    let store = mail_store::testing::in_memory();
    store
        .set_push_relay(account(), Some("https://relay.example.test"))
        .unwrap();
    assert_eq!(store.graph_push(account()).unwrap(), GraphPush::default());
}

#[test]
fn the_row_goes_with_its_account() {
    let store = mail_store::testing::in_memory();
    mail_store::testing::seed_account(&store, account(), "me@contoso.example");
    store
        .set_push_relay(account(), Some("https://relay.example.test"))
        .unwrap();
    store.remove_account(account()).unwrap();
    assert_eq!(store.graph_push(account()).unwrap(), GraphPush::default());
}

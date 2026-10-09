//! Accounts, identities and the lookups beside them: what a caller asks the store for instead of
//! reading its tables.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::{account_id_from_uuid, new_account_id};
use mail_domain::presets::{self, Manual};
use mail_domain::{Address, DraftId, Identity, IdentityId, IsDefault, LabelOrigin, ThreadId};
use mail_store::testing::{in_memory, seed_default_identity};
use mail_store::{FollowUpHold, NewAccount, StoreError};
use porter_core::AccountId;

fn at(day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
}

fn preset(address: &str, host: &str) -> presets::Preset {
    presets::manual(
        address,
        &Manual {
            imap_host: format!("imap.{host}"),
            imap_port: 993,
            smtp_host: format!("smtp.{host}"),
            smtp_port: 465,
            login: None,
        },
        at(1),
    )
}

fn identity(account: &AccountId, email: &str, default: IsDefault) -> Identity {
    Identity {
        id: IdentityId::generate(),
        account: account.clone(),
        from: Address {
            name: None,
            email: email.to_owned(),
        },
        reply_to: None,
        signature: None,
        default,
    }
}

fn add(
    store: &mail_store::SqliteStore,
    id: &AccountId,
    preset: &presets::Preset,
    identities: &[Identity],
    day: u32,
) -> Result<(), StoreError> {
    store.upsert_account(&NewAccount {
        id,
        address: &preset.plan.address,
        plan: &preset.plan,
        caps: &preset.expected_caps,
        identities,
        at: at(day),
    })
}

#[test]
fn an_account_is_written_whole_and_read_back_by_id_and_by_address() {
    let store = in_memory();
    let id = new_account_id();
    let preset = preset("ada@example.test", "example.test");
    let me = identity(&id, "ada@example.test", IsDefault::Default);
    add(&store, &id, &preset, std::slice::from_ref(&me), 1).unwrap();

    let by_id = store.account(id.clone()).unwrap().expect("by id");
    let by_address = store
        .account_by_address("ada@example.test")
        .unwrap()
        .expect("by address");
    assert_eq!(by_id.id, id);
    assert_eq!(by_address.id, id);
    assert_eq!(by_id.plan.as_ref().unwrap(), &preset.plan);
    assert_eq!(
        by_id.caps.as_ref().unwrap().as_ref().unwrap(),
        &preset.expected_caps
    );
    assert_eq!(
        store.account_caps(id.clone()).unwrap(),
        Some(preset.expected_caps)
    );
    assert_eq!(store.identities(id).unwrap(), vec![me]);
}

#[test]
fn an_address_already_here_keeps_its_id_and_its_identity_and_takes_the_new_plan() {
    let store = in_memory();
    let id = new_account_id();
    let first = preset("ada@example.test", "example.test");
    let mut mine = identity(&id, "ada@example.test", IsDefault::Default);
    add(&store, &id, &first, std::slice::from_ref(&mine), 1).unwrap();
    store.set_signature(mine.id, Some("Ada")).unwrap();
    mine.signature = Some("Ada".to_owned());

    // Added again with a better host, and an identity of the same id that knows nothing of the
    // signature set since.
    let better = preset("ada@example.test", "better.example.test");
    let bare = Identity {
        signature: None,
        ..mine.clone()
    };
    add(&store, &id, &better, &[bare], 9).unwrap();

    let stored = store.account(id.clone()).unwrap().unwrap();
    assert_eq!(stored.plan.unwrap(), better.plan);
    assert_eq!(store.list_accounts().unwrap().len(), 1);
    assert_eq!(
        store.identities(id).unwrap(),
        vec![mine],
        "the signature the user set survives an account being added again"
    );
}

#[test]
fn an_account_whose_identity_cannot_be_written_is_not_written_at_all() {
    // One transaction: the account row, its identities and its capabilities go together. Here
    // the address is taken by another account, so the row is updated in place and the identity,
    // which names an account that does not exist, is refused; the update must not outlive it.
    let store = in_memory();
    let held = new_account_id();
    let original = preset("ada@example.test", "example.test");
    add(&store, &held, &original, &[], 1).unwrap();

    let stranger = new_account_id();
    let replacement = preset("ada@example.test", "other.example.test");
    let orphan = identity(&stranger, "ada@example.test", IsDefault::Default);
    let refused = add(&store, &held, &replacement, &[orphan], 2);
    assert!(
        matches!(refused, Err(StoreError::Conflict(_))),
        "{refused:?}"
    );

    let stored = store.account(held.clone()).unwrap().unwrap();
    assert_eq!(
        stored.plan.unwrap(),
        original.plan,
        "the plan was replaced though the account was refused"
    );
    assert!(store.identities(held).unwrap().is_empty());
}

#[test]
fn a_plan_that_no_longer_reads_still_lists_the_account() {
    let store = in_memory();
    let id = account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
    mail_store::testing::seed_account(&store, id.clone(), "old@example.test");

    let listed = store.list_accounts().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].address, "old@example.test");
    assert!(
        matches!(listed[0].plan, Err(StoreError::Decode { .. })),
        "{:?}",
        listed[0].plan
    );
    assert!(listed[0].caps.is_none(), "nothing has connected");
}

#[test]
fn accounts_are_listed_oldest_first() {
    let store = in_memory();
    let (newer, older) = (new_account_id(), new_account_id());
    add(&store, &newer, &preset("b@example.test", "b.test"), &[], 5).unwrap();
    add(&store, &older, &preset("a@example.test", "a.test"), &[], 2).unwrap();

    let order: Vec<AccountId> = store
        .list_accounts()
        .unwrap()
        .into_iter()
        .map(|account| account.id)
        .collect();
    assert_eq!(order, vec![older, newer]);
}

#[test]
fn setting_the_plan_of_an_account_that_is_not_there_says_so() {
    let store = in_memory();
    let ghost = new_account_id();
    let plan = preset("ghost@example.test", "example.test").plan;
    match store.set_account_plan(ghost.clone(), &plan) {
        Err(StoreError::NoAccount(id)) => assert_eq!(id, ghost),
        other => panic!("{other:?}"),
    }
}

#[test]
fn setting_the_plan_replaces_it() {
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();
    let next = preset("ada@example.test", "next.example.test").plan;
    store.set_account_plan(id.clone(), &next).unwrap();
    assert_eq!(store.account(id).unwrap().unwrap().plan.unwrap(), next);
}

#[test]
fn identities_come_default_first_then_by_id() {
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();
    let alias = identity(&id, "alias@example.test", IsDefault::Alternate);
    let main = identity(&id, "ada@example.test", IsDefault::Default);
    store
        .upsert_account(&NewAccount {
            id: &id,
            address: "ada@example.test",
            plan: &preset("ada@example.test", "example.test").plan,
            caps: &preset("ada@example.test", "example.test").expected_caps,
            identities: &[alias.clone(), main.clone()],
            at: at(1),
        })
        .unwrap();

    let all = store.identities(id).unwrap();
    assert_eq!(all[0], main);
    assert_eq!(all[1], alias);
    assert_eq!(store.identity(alias.id).unwrap(), Some(alias.clone()));
    assert_eq!(
        store
            .identity_for_address(" ALIAS@example.test ")
            .unwrap()
            .map(|found| found.id),
        Some(alias.id),
        "found by address, whatever its case"
    );
    assert_eq!(store.identity(IdentityId::generate()).unwrap(), None);
}

#[test]
fn a_signature_is_set_and_cleared_on_its_identity() {
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();
    let mine = seed_default_identity(&store, id.clone(), "ada@example.test", Some("Ada"));

    store
        .set_signature(mine, Some("Ada, sent from mailo"))
        .unwrap();
    assert_eq!(
        store.identity(mine).unwrap().unwrap().signature.as_deref(),
        Some("Ada, sent from mailo")
    );
    store.set_signature(mine, None).unwrap();
    assert_eq!(store.identity(mine).unwrap().unwrap().signature, None);
}

#[test]
fn a_label_the_user_names_twice_on_one_account_is_a_conflict() {
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();

    let first = store.create_label(id.clone(), "travel").unwrap();
    assert!(matches!(
        store.create_label(id.clone(), "travel"),
        Err(StoreError::Conflict(_))
    ));
    let labels = mail_store::Store::labels(&store, id.clone()).unwrap();
    assert_eq!(labels.len(), 1);
    assert_eq!(labels[0].id, first);
    assert_eq!(labels[0].origin, LabelOrigin::User);

    // The same word on another account is another label.
    let other = new_account_id();
    add(
        &store,
        &other,
        &preset("bee@example.test", "bee.test"),
        &[],
        2,
    )
    .unwrap();
    store.create_label(other, "travel").unwrap();
}

#[test]
fn the_notify_floor_is_armed_once_and_then_read_back() {
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();

    assert_eq!(store.arm_notify_floor(id.clone(), at(3)).unwrap(), at(3));
    assert_eq!(
        store.arm_notify_floor(id, at(20)).unwrap(),
        at(3),
        "the first writer wins"
    );
}

#[test]
fn a_hold_is_kept_replaced_listed_and_let_go() {
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();
    let draft = DraftId::generate();
    let mut hold = FollowUpHold {
        draft,
        account: id,
        message_id: "m1@example.test".to_owned(),
        thread: Some(ThreadId::generate()),
        at: at(10),
        set: at(4),
    };
    store.hold_follow_up(&hold).unwrap();
    hold.message_id = "m2@example.test".to_owned();
    hold.thread = None;
    store.hold_follow_up(&hold).unwrap();

    assert_eq!(store.follow_up_holds().unwrap(), vec![hold]);
    store.release_follow_up(draft).unwrap();
    assert!(store.follow_up_holds().unwrap().is_empty());
    store.release_follow_up(draft).unwrap();
}

#[test]
fn a_message_is_found_by_the_message_id_it_came_with() {
    use mail_store::Store;
    let store = in_memory();
    let id = new_account_id();
    add(
        &store,
        &id,
        &preset("ada@example.test", "example.test"),
        &[],
        1,
    )
    .unwrap();

    assert_eq!(
        store
            .message_by_rfc_id(id.clone(), "nope@example.test")
            .unwrap(),
        None
    );
    assert_eq!(
        store
            .thread_by_rfc_id(id.clone(), "nope@example.test")
            .unwrap(),
        None
    );

    let raw = store.blobs().put(b"raw").unwrap();
    let message = mail_domain::Message {
        id: mail_domain::MessageId::generate(),
        thread: ThreadId::generate(),
        account: id.clone(),
        key: mail_domain::MessageKey::Rfc("m1@example.test".to_owned()),
        date: at(2),
        from: Address {
            name: None,
            email: "bee@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "hello".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("m1@example.test".to_owned()),
        read: mail_domain::ReadState::Unread,
        star: mail_domain::Star::Unstarred,
        mailbox: mail_domain::MailboxRole::Inbox,
        labels: vec![],
        body: mail_domain::Body::Present { text: None, raw },
        attachments: vec![],
    };
    let (mid, tid) = (message.id, message.thread);
    store
        .apply(
            id.clone(),
            &mail_domain::Patch {
                id: mail_domain::ChangeId::generate(),
                changes: vec![mail_domain::Change::MessageUpsert(Box::new(message))],
            },
        )
        .unwrap();

    assert_eq!(
        store
            .message_by_rfc_id(id.clone(), "m1@example.test")
            .unwrap(),
        Some(mid)
    );
    assert_eq!(
        store.thread_by_rfc_id(id, "m1@example.test").unwrap(),
        Some(tid)
    );
}

#[test]
fn blobs_are_read_and_written_without_naming_a_connection() {
    let store = in_memory();
    let small = store.blobs().put(b"hello").unwrap();
    assert_eq!(store.blobs().put(b"hello").unwrap(), small);
    assert_eq!(store.blobs().get(small).unwrap(), b"hello");
    assert_eq!(store.blobs().head(small, 2).unwrap(), b"he");
    assert_eq!(store.blobs().size(small).unwrap(), 5);
    assert!(store.blobs().get(mail_domain::BlobId::generate()).is_err());
}

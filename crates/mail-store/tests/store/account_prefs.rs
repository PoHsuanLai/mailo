//! What the person says about an account beyond its plan: a description, a paused switch, and
//! the folders picked for roles, which a refresh of the capabilities must not undo.

use chrono::{TimeZone, Utc};
use mail_domain::id::new_account_id;
use mail_domain::presets::{self, Manual};
use mail_domain::{FolderRoles, MailboxRole};
use mail_store::testing::in_memory;
use mail_store::{AccountPrefs, NewAccount, Store, StoreError, Syncing};

fn added(store: &mail_store::SqliteStore) -> porter_core::AccountId {
    let id = new_account_id();
    let preset = presets::manual(
        "ada@example.test",
        &Manual {
            imap_host: "imap.example.test".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        },
        Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap(),
    );
    store
        .upsert_account(&NewAccount {
            id: &id,
            address: &preset.plan.address,
            plan: &preset.plan,
            caps: &preset.expected_caps,
            identities: &[],
            at: Utc::now(),
        })
        .unwrap();
    id
}

#[test]
fn an_account_nobody_has_described_has_the_defaults() {
    let store = in_memory();
    let id = added(&store);
    assert_eq!(store.account_prefs(id).unwrap(), AccountPrefs::default());
}

#[test]
fn a_description_and_a_pause_are_kept_and_a_blank_description_is_none() {
    let store = in_memory();
    let id = added(&store);
    store
        .set_account_description(id.clone(), Some("  Work  "))
        .unwrap();
    store
        .set_account_syncing(id.clone(), Syncing::Paused)
        .unwrap();
    let prefs = store.account_prefs(id.clone()).unwrap();
    assert_eq!(prefs.description.as_deref(), Some("Work"));
    assert_eq!(prefs.syncing, Syncing::Paused);
    store
        .set_account_description(id.clone(), Some("   "))
        .unwrap();
    assert_eq!(store.account_prefs(id).unwrap().description, None);
}

#[test]
fn prefs_for_an_account_that_is_not_there_are_refused() {
    let store = in_memory();
    let stranger = new_account_id();
    assert!(matches!(
        store.set_account_syncing(stranger, Syncing::Paused),
        Err(StoreError::NoAccount(_))
    ));
}

#[test]
fn a_picked_folder_survives_a_refresh_of_the_capabilities() {
    let store = in_memory();
    let id = added(&store);
    let mut observed = store.account_caps(id.clone()).unwrap().unwrap();
    observed.folders = FolderRoles(vec![
        ("Sent".to_owned(), MailboxRole::Sent),
        ("Bin".to_owned(), MailboxRole::Trash),
    ]);
    store.put_caps(id.clone(), &observed, Utc::now()).unwrap();
    store
        .set_folder_role(id.clone(), MailboxRole::Sent, Some("Outbox/Sent Items"))
        .unwrap();
    let picked = store.account_caps(id.clone()).unwrap().unwrap();
    assert_eq!(
        picked.folders.path(MailboxRole::Sent),
        Some("Outbox/Sent Items")
    );
    assert_eq!(
        picked.folders.role("Sent"),
        None,
        "the folder the server named no longer plays the role"
    );

    // What a refresh writes: the server's own answer again, which still names `Sent`.
    store.put_caps(id.clone(), &observed, Utc::now()).unwrap();
    let refreshed = store.account_caps(id.clone()).unwrap().unwrap();
    assert_eq!(
        refreshed.folders.path(MailboxRole::Sent),
        Some("Outbox/Sent Items")
    );
    assert_eq!(
        refreshed.folders.path(MailboxRole::Trash),
        Some("Bin"),
        "the rest stay observed"
    );
    let listed = store
        .account(id.clone())
        .unwrap()
        .unwrap()
        .caps
        .unwrap()
        .unwrap();
    assert_eq!(
        listed.folders.path(MailboxRole::Sent),
        Some("Outbox/Sent Items")
    );

    // Given back to Automatic, the role plays the folder the server named for it at once.
    store
        .set_folder_role(id.clone(), MailboxRole::Sent, None)
        .unwrap();
    let given_back = store.account_caps(id.clone()).unwrap().unwrap();
    assert_eq!(given_back.folders.path(MailboxRole::Sent), Some("Sent"));
    assert_eq!(given_back.folders.path(MailboxRole::Trash), Some("Bin"));
}

#[test]
fn a_role_given_back_before_any_refresh_plays_the_folder_first_observed() {
    let store = in_memory();
    let id = added(&store);
    let mut observed = store.account_caps(id.clone()).unwrap().unwrap();
    observed.folders = FolderRoles(vec![("Junk".to_owned(), MailboxRole::Spam)]);
    store.put_caps(id.clone(), &observed, Utc::now()).unwrap();
    store
        .set_folder_role(id.clone(), MailboxRole::Spam, Some("Spam"))
        .unwrap();
    store
        .set_folder_role(id.clone(), MailboxRole::Trash, Some("Junk"))
        .unwrap();
    assert_eq!(
        store
            .account_caps(id.clone())
            .unwrap()
            .unwrap()
            .folders
            .role("Junk"),
        Some(MailboxRole::Trash)
    );
    store
        .set_folder_role(id.clone(), MailboxRole::Spam, None)
        .unwrap();
    store
        .set_folder_role(id.clone(), MailboxRole::Trash, None)
        .unwrap();
    let back = store.account_caps(id).unwrap().unwrap();
    assert_eq!(back.folders.path(MailboxRole::Spam), Some("Junk"));
    assert_eq!(back.folders.path(MailboxRole::Trash), None);
}

#[test]
fn a_folder_role_for_an_account_that_is_not_there_writes_nothing() {
    let store = in_memory();
    let stranger = new_account_id();
    assert!(matches!(
        store.set_folder_role(stranger.clone(), MailboxRole::Sent, Some("Sent")),
        Err(StoreError::NoAccount(_))
    ));
    assert_eq!(
        store.account_prefs(stranger).unwrap(),
        AccountPrefs::default()
    );
}

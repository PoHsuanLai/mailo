//! Linked to the desktop's accountd, the accounts Mail loads and syncs are accountd's alone. The
//! ones Mail signed in itself are set aside, and nothing of theirs is deleted or changed: their
//! stored plans and their keyring items stay for a start that is not linked.
//!
//! The rows are made from the real presets and the real `reconcile`, so the kinds the store looks
//! at are the ones a plan is actually written with.

use mail_core::account::reconcile;
use mail_domain::presets::{self, Manual, Preset};
use mail_store::SqliteStore;
use porter_core::capability::{
    Access, Capability, Delta, LabelModel, MailCap, MailTransport, Offered,
};
use porter_core::{
    AccountId, AccountLabel, Candidate, EndpointUrl, Family, GrantId, LoginName, ProviderId,
    Restriction, ServiceEndpoint, Subject,
};

const PASSWORD: &str = "held-password@example.test";
const OAUTH: &str = "held-oauth@example.test";
const GRANTED: &str = "me@example.test";

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn endpoint(family: Family, url: &str, tls: porter_core::Tls) -> ServiceEndpoint {
    ServiceEndpoint {
        family,
        url: EndpointUrl::parse(url).unwrap(),
        tls,
        login: LoginName(GRANTED.to_owned()),
    }
}

fn fastmail() -> Candidate {
    Candidate {
        account: AccountId::parse("fastmail-me").unwrap(),
        label: AccountLabel(GRANTED.to_owned()),
        provider: ProviderId::parse("fastmail").unwrap(),
        subject: Subject::Account,
        capability: Capability::Mail(MailCap {
            access: Access::ReadWrite,
            send: Offered::Present,
            delta: Delta::Push,
            transport: MailTransport::Imap,
            labels: LabelModel::Folders,
        }),
        restriction: Restriction::none(),
        grant: GrantId::parse("grant-1").unwrap(),
        endpoints: vec![
            endpoint(
                Family::Imap,
                "imaps://imap.fastmail.test:993",
                porter_core::Tls::Implicit,
            ),
            endpoint(
                Family::Smtp,
                "smtp://smtp.fastmail.test:587",
                porter_core::Tls::StartTls,
            ),
        ],
    }
}

/// A row as an earlier, unlinked start wrote it: its plan, and the capabilities beside it.
fn row(store: &SqliteStore, n: u128, preset: Preset) -> AccountId {
    let id = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(n));
    let db = store.connection();
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            id.to_string(),
            preset.plan.address,
            serde_json::to_string(&preset.plan).unwrap(),
            format!("2026-01-0{n}T00:00:00Z"),
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            id.to_string(),
            serde_json::to_string(&preset.expected_caps).unwrap(),
            now().to_rfc3339(),
        ],
    )
    .unwrap();
    id
}

/// Accounts of every kind: two Mail signed in itself, Local folders, and accountd's.
fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let manual = Manual {
        imap_host: "imap.example.test".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.example.test".to_owned(),
        smtp_port: 465,
        login: None,
    };
    row(&store, 1, presets::manual(PASSWORD, &manual, now()));
    row(&store, 2, presets::microsoft_preset(OAUTH, now()));
    row(&store, 3, presets::local_folders(now()));
    reconcile(&store, &[fastmail()], now()).unwrap();
    (store, dir)
}

fn plans(store: &SqliteStore) -> Vec<(String, String)> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT address, plan FROM accounts ORDER BY address")
        .unwrap();
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn named(store: &SqliteStore) -> Vec<String> {
    let mut named: Vec<String> = mail_core::sync::addresses(store)
        .into_iter()
        .map(|(_, address)| address)
        .collect();
    named.sort();
    named
}

#[test]
fn unlinked_every_account_is_loaded_and_synced() {
    let (store, _dir) = store();
    assert!(!store.granted_only());
    assert_eq!(named(&store).len(), 4, "{:?}", named(&store));
    assert_eq!(store.held_accounts().len(), 2);
}

#[test]
fn linked_loads_only_the_granted_accounts_and_syncs_none_of_the_held() {
    let (store, _dir) = store();
    store.set_granted_only(true);

    // Every sync entry point (`sync::run`, the folder and body fetches, the watch, the rules, the
    // contacts) takes its accounts from this read.
    assert_eq!(named(&store), [presets::LOCAL_FOLDERS, GRANTED]);
    let auth = mail_core::sync::auth_by_account(&store).unwrap();
    assert!(
        matches!(
            auth.get(GRANTED),
            Some(mail_domain::AuthPlan::Granted { .. })
        ),
        "{auth:?}"
    );
    assert!(!auth.contains_key(PASSWORD) && !auth.contains_key(OAUTH));

    // The sender picker offers the same accounts.
    let from: Vec<String> = mail_core::compose::sending_accounts(&store)
        .into_iter()
        .map(|(address, _)| address)
        .collect();
    assert_eq!(from, [GRANTED]);
}

#[test]
fn linked_leaves_what_mailo_held_exactly_as_it_was() {
    let (store, _dir) = store();
    let before = plans(&store);
    store.set_granted_only(true);

    // Read accountd's accounts again, and have a removed one forgotten: the held rows are not
    // part of either.
    reconcile(&store, &[fastmail()], now()).unwrap();
    let gone = mail_core::account::linked_accounts(&store);
    assert_eq!(gone.len(), 1);
    let freed = mail_core::account::forget(&store, &gone);
    assert!(freed.iter().all(|(_, result)| result.is_ok()));

    let after: Vec<_> = plans(&store)
        .into_iter()
        .filter(|(address, _)| address != GRANTED)
        .collect();
    let kept: Vec<_> = before
        .into_iter()
        .filter(|(address, _)| address != GRANTED)
        .collect();
    assert_eq!(after, kept, "a stored plan of Mail's own changed");

    // Taken back, unlinked, they are all still loaded.
    store.set_granted_only(false);
    assert!(named(&store).contains(&PASSWORD.to_owned()));
    assert!(named(&store).contains(&OAUTH.to_owned()));
}

//! Templates survive a round trip, in both [`Store`] implementations, and agree.
//!
//! Asked of both and compared, for the reason `drafts.rs` gives: the parity proptest only sees
//! what a filter can ask, and a template is not something a filter can ask about.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};
use porter_core::AccountId;

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
fn acct_other() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
}
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const OTHER_IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));

struct Both {
    sqlite: SqliteStore,
    memory: MemoryStore,
    _dir: tempfile::TempDir,
}

/// Both stores, with the account and identity rows SQLite's foreign keys require.
fn both() -> Both {
    let dir = tempfile::tempdir().unwrap();
    let sqlite = SqliteStore::in_memory(dir.path()).unwrap();
    {
        let db = sqlite.raw_connection();
        for (account, identity, address) in [
            (acct_account(), IDENTITY, "me@example.test"),
            (acct_other(), OTHER_IDENTITY, "also-me@example.test"),
        ] {
            db.execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, '{}', datetime('now'))",
                [account.to_string(), address.to_owned()],
            )
            .unwrap();
            db.execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES (?1, ?2, 'Me', ?3, '\"default\"')",
                [
                    identity.to_string(),
                    account.to_string(),
                    address.to_owned(),
                ],
            )
            .unwrap();
        }
    }
    Both {
        sqlite,
        memory: MemoryStore::new(),
        _dir: dir,
    }
}

impl Both {
    /// Each store, named for a failure message.
    fn each(&self) -> [(&'static str, &dyn Store); 2] {
        [("sqlite", &self.sqlite), ("memory", &self.memory)]
    }
}

fn addr(email: &str) -> Address {
    Address {
        name: Some("Someone".to_owned()),
        email: email.to_owned(),
    }
}

/// A template with every field populated, so the round trip proves each column.
fn template(name: &str) -> Template {
    Template {
        id: TemplateId::generate(),
        account: acct_account(),
        identity: IDENTITY,
        name: name.to_owned(),
        to: vec![addr("team@example.test"), addr("lead@example.test")],
        cc: vec![addr("carbon@example.test")],
        bcc: vec![addr("blind@example.test")],
        subject: "Weekly report".to_owned(),
        text: "This week:\r\n- \r\n".to_owned(),
        html: Some("<p>This week:</p>".to_owned()),
        attachments: vec![PendingAttachment {
            name: "plan.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            blob: BlobId::generate(),
        }],
        receipt: ReceiptRequest::Requested,
        openpgp: OpenPgp::SignAndEncrypt,
        smime: mail_domain::Smime::Sign,
        updated: at(10),
    }
}

fn put(b: &Both, template: &Template) {
    b.sqlite.put_template(template).unwrap();
    b.memory.put_template(template).unwrap();
}

/// One life of a template list, asked of both stores at each step: templates list by name
/// whatever the case and only for their own account, keeping one again replaces it, and a
/// deleted one is gone and a second delete says so.
#[test]
fn templates_in_both_stores_list_by_name_replace_and_delete() {
    let b = both();
    let mut weekly = template("weekly");
    put(&b, &weekly);
    for name in ["Absence", "birthday", "absence", "Zebra"] {
        put(&b, &template(name));
    }
    let mut elsewhere = template("on another account");
    elsewhere.account = acct_other();
    elsewhere.identity = OTHER_IDENTITY;
    put(&b, &elsewhere);

    let listed = |want: &[&str], step: &str| {
        let sqlite = b.sqlite.templates(acct_account()).unwrap();
        let memory = b.memory.templates(acct_account()).unwrap();
        assert_eq!(
            sqlite, memory,
            "{step}: the two stores list templates differently"
        );
        let names: Vec<String> = sqlite.iter().map(|t| t.name.to_lowercase()).collect();
        assert_eq!(
            names, want,
            "{step}: by name, ignoring case, and only this account's"
        );
        sqlite
    };
    listed(
        &["absence", "absence", "birthday", "weekly", "zebra"][..],
        "listing",
    );

    weekly.name = "weekly, renamed".to_owned();
    weekly.updated = at(20);
    put(&b, &weekly);
    let after = listed(
        &["absence", "absence", "birthday", "weekly, renamed", "zebra"][..],
        "keeping again",
    );
    assert_eq!(
        after[3], weekly,
        "keeping again replaces the template whole"
    );

    let stays: Vec<Template> = after.into_iter().filter(|t| t.id != weekly.id).collect();
    for (label, store) in b.each() {
        store.delete_template(weekly.id).unwrap();
        assert!(
            matches!(
                store.template(weekly.id),
                Err(StoreError::NoTemplate(id)) if id == weekly.id
            ),
            "{label}: a deleted template is gone"
        );
        assert!(
            matches!(
                store.delete_template(weekly.id),
                Err(StoreError::NoTemplate(_))
            ),
            "{label}: a second delete says so"
        );
        assert_eq!(
            store.templates(acct_account()).unwrap(),
            stays,
            "{label}: the others stay"
        );
    }
}

#[test]
fn a_template_is_not_a_draft() {
    // The reason for a table of their own: nothing that lists, sends or discards drafts can
    // meet a template by accident.
    let b = both();
    put(&b, &template("weekly"));
    assert_eq!(b.sqlite.drafts(acct_account()).unwrap(), vec![]);
    assert_eq!(b.memory.drafts(acct_account()).unwrap(), vec![]);
}

#[test]
fn a_draft_started_from_a_template_is_saved_beside_it_and_leaves_it_alone() {
    let b = both();
    let kept = template("weekly");
    put(&b, &kept);
    let started = kept.draft(at(30));
    let patch = Patch {
        id: ChangeId::generate(),
        changes: vec![Change::DraftUpsert(Box::new(started.clone()))],
    };
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        store.apply(acct_account(), &patch).unwrap();
        assert_eq!(store.draft(started.id).unwrap(), started);
        assert_eq!(store.template(kept.id).unwrap(), kept);
    }
}

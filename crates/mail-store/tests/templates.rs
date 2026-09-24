//! Templates survive a round trip, in both [`Store`] implementations, and agree.
//!
//! Asked of both and compared, for the reason `drafts.rs` gives: the parity proptest only sees
//! what a filter can ask, and a template is not something a filter can ask about.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{MemoryStore, SqliteStore, Store, StoreError};

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const OTHER: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"));
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
        let db = sqlite.connection();
        for (account, identity, address) in [
            (ACCOUNT, IDENTITY, "me@example.test"),
            (OTHER, OTHER_IDENTITY, "also-me@example.test"),
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
        account: ACCOUNT,
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
        updated: at(10),
    }
}

fn put(b: &Both, template: &Template) {
    b.sqlite.put_template(template).unwrap();
    b.memory.put_template(template).unwrap();
}

#[test]
fn a_kept_template_reads_back_whole_from_both_stores() {
    let b = both();
    let kept = template("weekly");
    put(&b, &kept);
    assert_eq!(b.sqlite.template(kept.id).unwrap(), kept);
    assert_eq!(b.memory.template(kept.id).unwrap(), kept);
}

#[test]
fn templates_list_by_name_whatever_the_case_in_both_stores() {
    let b = both();
    let names = ["weekly", "Absence", "birthday", "absence", "Zebra"];
    for name in names {
        put(&b, &template(name));
    }
    let mut elsewhere = template("on another account");
    elsewhere.account = OTHER;
    elsewhere.identity = OTHER_IDENTITY;
    put(&b, &elsewhere);

    let sqlite = b.sqlite.templates(ACCOUNT).unwrap();
    let memory = b.memory.templates(ACCOUNT).unwrap();
    assert_eq!(sqlite, memory, "the two stores list templates differently");
    let listed: Vec<String> = sqlite.iter().map(|t| t.name.to_lowercase()).collect();
    assert_eq!(
        listed,
        ["absence", "absence", "birthday", "weekly", "zebra"],
        "by name, ignoring case, and only this account's"
    );
}

#[test]
fn keeping_a_template_again_replaces_it() {
    let b = both();
    let mut kept = template("weekly");
    put(&b, &kept);
    kept.name = "weekly, renamed".to_owned();
    kept.updated = at(20);
    put(&b, &kept);
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        assert_eq!(store.templates(ACCOUNT).unwrap(), vec![kept.clone()]);
    }
}

#[test]
fn a_deleted_template_is_gone_and_a_second_delete_says_so() {
    let b = both();
    let kept = template("weekly");
    let stays = template("other");
    put(&b, &kept);
    put(&b, &stays);
    for store in [&b.sqlite as &dyn Store, &b.memory] {
        store.delete_template(kept.id).unwrap();
        assert!(matches!(
            store.template(kept.id),
            Err(StoreError::NoTemplate(id)) if id == kept.id
        ));
        assert!(matches!(
            store.delete_template(kept.id),
            Err(StoreError::NoTemplate(_))
        ));
        assert_eq!(store.templates(ACCOUNT).unwrap(), vec![stays.clone()]);
    }
}

#[test]
fn a_template_is_not_a_draft() {
    // The reason for a table of their own: nothing that lists, sends or discards drafts can
    // meet a template by accident.
    let b = both();
    put(&b, &template("weekly"));
    assert_eq!(b.sqlite.drafts(ACCOUNT).unwrap(), vec![]);
    assert_eq!(b.memory.drafts(ACCOUNT).unwrap(), vec![]);
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
        store.apply(ACCOUNT, &patch).unwrap();
        assert_eq!(store.draft(started.id).unwrap(), started);
        assert_eq!(store.template(kept.id).unwrap(), kept);
    }
}

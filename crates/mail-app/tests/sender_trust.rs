//! Sender trust against a real store: which `Authentication-Results` the app believes for an
//! account, read from the stored message, and "Block sender" as a rule.
//!
//! The account reads its mail from `pop.provider.example`, so the provider is
//! `provider.example`, and its receiving server writes `mx.provider.example`. Everything is in a
//! `TempDir`; nothing reaches a server.

use mail_app::auth::{Standing, results_of, standing};
use mail_app::rules::block::{Blocked, block, condition, unblock};
use mail_domain::*;
use mail_mime::Verdict;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn seeded(dir: &std::path::Path) -> SqliteStore {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    let plan = mail_domain::presets::manual_pop3(
        "me@provider.example",
        &mail_domain::presets::ManualPop3 {
            pop3_host: "pop.provider.example".to_owned(),
            pop3_port: 995,
            smtp_host: "smtp.provider.example".to_owned(),
            smtp_port: 465,
            login: None,
        },
        chrono::Utc::now(),
    )
    .plan;
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@provider.example', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    store
}

/// Store a message from `from` with `headers` above its own, and return it.
fn arrive(store: &SqliteStore, n: u32, from: &str, headers: &str) -> Message {
    let raw = format!(
        "{headers}From: {from}\r\nTo: me@provider.example\r\nSubject: note {n}\r\n\
         Date: Fri, 25 Sep 2026 10:00:0{n} +0000\r\nMessage-ID: <note{n}@sender.example>\r\n\r\n\
         Hello.\r\n"
    );
    absorb(
        store,
        ACCOUNT,
        MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: format!("note{n}"),
            },
            raw: raw.into_bytes(),
        }],
        false,
        chrono::Utc::now(),
    )
    .unwrap();
    let rfc = format!("note{n}@sender.example");
    let thread = store
        .threads(
            &Query {
                filter: Filter::Subject(TextMatch::Exact(format!("note {n}"))),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 5,
                },
            },
            chrono::Utc::now(),
        )
        .unwrap()
        .items
        .remove(0);
    store
        .thread(thread.id)
        .unwrap()
        .messages
        .iter()
        .map(|id| store.message(*id).unwrap())
        .find(|message| message.rfc_message_id.as_deref() == Some(rfc.as_str()))
        .unwrap()
}

#[test]
fn the_providers_field_is_believed_and_a_forged_one_below_it_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let message = arrive(
        &store,
        1,
        "Bank <alerts@bank.example>",
        "Authentication-Results: mx.provider.example; spf=fail smtp.mailfrom=bank.example;\r\n \
         dkim=none; dmarc=fail header.from=bank.example\r\n\
         Received: from attacker.example by mx.provider.example; Fri, 25 Sep 2026 10:00:00 +0000\r\n\
         Authentication-Results: mx.provider.example; spf=pass; dkim=pass header.d=bank.example;\r\n \
         dmarc=pass header.from=bank.example\r\n",
    );
    let results = results_of(&store, &message).expect("the provider's field is read");
    assert_eq!(
        results.dmarc.as_ref().map(|c| c.verdict),
        Some(Verdict::Fail)
    );
    assert_eq!(standing(&results), Standing::Failed);
}

#[test]
fn a_senders_own_field_is_never_a_pass() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // The provider wrote none; the only field is one the sender put in its own message.
    let message = arrive(
        &store,
        2,
        "Bank <alerts@bank.example>",
        "Received: from attacker.example by mx.provider.example; Fri, 25 Sep 2026 10:00:00 +0000\r\n\
         Authentication-Results: mx.bank.example; spf=pass; dkim=pass; dmarc=pass\r\n",
    );
    assert_eq!(results_of(&store, &message), None);
}

#[test]
fn blocking_makes_exactly_one_rule_and_unblocking_takes_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    arrive(&store, 3, "Pest <pest@spam.example>", "");
    let before = store.rules(ACCOUNT).unwrap();

    let Blocked::Made(rule) = block(&store, ACCOUNT, "pest@spam.example").unwrap() else {
        panic!("a first block made no rule");
    };
    let after = store.rules(ACCOUNT).unwrap();
    assert_eq!(after.len(), before.len() + 1);
    let kept = after
        .iter()
        .find(|r| r.id == rule.id)
        .expect("the rule is stored");
    assert_eq!(kept.filter, condition("pest@spam.example"));
    assert_eq!(kept.actions, vec![RuleAction::Spam]);
    assert_eq!(kept.after, AfterMatch::Stop);
    assert_eq!(kept.state, RuleState::Enabled);
    // The rule is about their mail: it matches the conversation they sent.
    let matching = store
        .count(
            &Filter::And(vec![Filter::Account(ACCOUNT), kept.filter.clone()]),
            chrono::Utc::now(),
        )
        .unwrap();
    assert_eq!(matching, 1);

    // Blocking them again, in another case, writes nothing more.
    assert!(matches!(
        block(&store, ACCOUNT, "Pest@Spam.Example").unwrap(),
        Blocked::Already(had) if had.id == rule.id
    ));
    assert_eq!(store.rules(ACCOUNT).unwrap().len(), before.len() + 1);

    unblock(&store, rule.id).unwrap();
    assert_eq!(store.rules(ACCOUNT).unwrap(), before);
}

#[test]
fn a_block_runs_before_the_rules_already_there() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let archive = Rule {
        id: RuleId::generate(),
        account: ACCOUNT,
        name: "Archive the lot".to_owned(),
        position: 1,
        state: RuleState::Enabled,
        filter: Filter::From(TextMatch::Contains("spam.example".to_owned())),
        actions: vec![RuleAction::Archive],
        after: AfterMatch::Stop,
    };
    store.put_rule(&archive).unwrap();
    let Blocked::Made(rule) = block(&store, ACCOUNT, "pest@spam.example").unwrap() else {
        panic!("no rule made");
    };
    let rules = store.rules(ACCOUNT).unwrap();
    let order: Vec<RuleId> = rule::ordered(&rules).iter().map(|r| r.id).collect();
    assert_eq!(order, vec![rule.id, archive.id]);
}

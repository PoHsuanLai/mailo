//! `mailo unsubscribe`: what a list message offers, and the `mailto:` way out.
//!
//! The one-click `POST` is covered in `mail-runtime/tests/unsubscribe.rs` against a local TLS
//! listener. What is left to prove here is the half that touches the store: the headers are
//! read from the raw message, a `mailto:` becomes a queued message from the right address, and
//! a web page is printed rather than fetched. No test here reaches the network: the only paths
//! that could are the one-click ones, and no fixture below offers one-click to the CLI.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::cli::{self, Command, UnsubscribeStep};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const DEFAULT_IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ALIAS_IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn exercise(store: &SqliteStore, command: &Command) -> Result<String, String> {
    cli::run_with_clients(
        store,
        command,
        at(100),
        &mail_runtime::OAuthRegistry::default(),
    )
}

/// An account with two identities: the default, and an alias the list mail is addressed to.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let mut plan = mail_domain::presets::manual_pop3(
        "me@example.test",
        &mail_domain::presets::ManualPop3 {
            pop3_host: "pop.example.test".to_owned(),
            pop3_port: 995,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        },
        at(0),
    )
    .plan;
    plan.address = "me@example.test".to_owned();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        for (id, email, default) in [
            (DEFAULT_IDENTITY, "me@example.test", "\"default\""),
            (ALIAS_IDENTITY, "lists@example.test", "\"alternate\""),
        ] {
            db.execute(
                "INSERT INTO identities (id, account, from_name, from_email, is_default)
                 VALUES (?1, ?2, NULL, ?3, ?4)",
                rusqlite::params![id.to_string(), ACCOUNT.to_string(), email, default],
            )
            .unwrap();
        }
    }
    (store, dir)
}

/// Store a message with these list headers, addressed to the alias, in `thread` or a new one.
/// Returns its id and thread.
fn list_message(
    store: &SqliteStore,
    n: i64,
    headers: &str,
    thread: Option<ThreadId>,
    body: Presence,
) -> (MessageId, ThreadId) {
    let rfc_id = format!("list{n}@example.test");
    let raw_bytes = format!(
        "From: news@example.test\r\nTo: lists@example.test\r\nSubject: news {n}\r\n\
         Message-ID: <{rfc_id}>\r\n{headers}\r\nthis week's news\r\n"
    );
    let raw = store
        .blobs()
        .put(&store.connection(), raw_bytes.as_bytes())
        .unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: thread.unwrap_or_else(ThreadId::generate),
        account: ACCOUNT,
        key: MessageKey::Rfc(rfc_id.clone()),
        date: at(n),
        from: Address {
            name: Some("News".to_owned()),
            email: "news@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "lists@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: format!("news {n}"),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(rfc_id),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: match body {
            Presence::Fetched => Body::Present {
                text: Some("this week's news".to_owned()),
                raw,
            },
            Presence::HeadersOnly => Body::Absent,
        },
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: format!("u{n}"),
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    let thread = store.message(id).unwrap().thread;
    (id, thread)
}

#[derive(Clone, Copy)]
enum Presence {
    Fetched,
    HeadersOnly,
}

const MAILTO: &str = "List-Id: Weekly News <news.example.test>\r\n\
    List-Unsubscribe: <https://example.test/page>, \
    <mailto:leave@example.test?subject=remove%20me&body=unsubscribe%20lists>\r\n";

fn unsubscribe(target: uuid::Uuid, step: UnsubscribeStep) -> Command {
    Command::Unsubscribe { target, step }
}

#[test]
fn show_lists_every_way_out_and_marks_the_one_it_would_take() {
    let (store, _dir) = seeded();
    let (message, _) = list_message(&store, 1, MAILTO, None, Presence::Fetched);

    let out = exercise(
        &store,
        &unsubscribe(*message.as_uuid(), UnsubscribeStep::Show),
    )
    .unwrap();
    assert!(out.contains("Weekly News <news.example.test>"), "{out}");
    assert!(
        out.contains("  web page   https://example.test/page (shown, never opened)"),
        "{out}"
    );
    assert!(
        out.contains("* mail       to leave@example.test, subject \"remove me\""),
        "{out}"
    );
    // Showing does nothing.
    assert!(store.drafts(ACCOUNT).unwrap().is_empty());
    assert!(store.outbox_due(ACCOUNT, at(1_000)).unwrap().is_empty());
}

#[test]
fn a_mailto_becomes_a_queued_message_from_the_address_the_list_writes_to() {
    let (store, _dir) = seeded();
    let (_, thread) = list_message(&store, 1, MAILTO, None, Presence::Fetched);

    let out = exercise(
        &store,
        &unsubscribe(*thread.as_uuid(), UnsubscribeStep::Act),
    )
    .unwrap();
    assert!(out.contains("queued an unsubscribe message"), "{out}");

    let drafts = store.drafts(ACCOUNT).unwrap();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    let draft = &drafts[0];
    // The alias the list mail came to, not the account's default.
    assert_eq!(draft.identity, ALIAS_IDENTITY);
    assert_eq!(
        draft.to,
        vec![Address {
            name: None,
            email: "leave@example.test".to_owned()
        }]
    );
    assert!(draft.cc.is_empty() && draft.bcc.is_empty());
    assert_eq!(draft.subject, "remove me");
    // Exactly the URI's body: no signature appended to what list software may read as commands.
    assert_eq!(draft.text, "unsubscribe lists");
    assert_eq!(draft.state, SendState::Queued);

    let due = store.outbox_due(ACCOUNT, at(1_000)).unwrap();
    assert_eq!(due.len(), 1, "nothing was queued");
    let ProtoOp::Submit {
        mail_from,
        rcpt_to,
        raw,
        ..
    } = &due[0].op
    else {
        panic!("expected a submission, got {:?}", due[0].op);
    };
    assert_eq!(mail_from, "lists@example.test");
    assert_eq!(rcpt_to, &vec!["leave@example.test".to_owned()]);
    let bytes = store.blobs().get(&store.connection(), *raw).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("Subject: remove me"), "{text}");
}

#[test]
fn a_mailto_with_no_subject_says_unsubscribe() {
    let (store, _dir) = seeded();
    let (message, _) = list_message(
        &store,
        1,
        "List-Unsubscribe: <mailto:leave@example.test>\r\n",
        None,
        Presence::Fetched,
    );
    exercise(
        &store,
        &unsubscribe(*message.as_uuid(), UnsubscribeStep::Act),
    )
    .unwrap();
    let drafts = store.drafts(ACCOUNT).unwrap();
    assert_eq!(drafts[0].subject, "unsubscribe");
    assert_eq!(drafts[0].text, "");
}

#[test]
fn a_thread_uses_its_newest_message_that_offers_a_way_out() {
    let (store, _dir) = seeded();
    let (_, thread) = list_message(
        &store,
        1,
        "List-Unsubscribe: <mailto:old@example.test>\r\n",
        None,
        Presence::Fetched,
    );
    let (_, same) = list_message(
        &store,
        2,
        "List-Unsubscribe: <mailto:new@example.test>\r\n",
        Some(thread),
        Presence::Fetched,
    );
    // A later reply in the thread with no list headers does not hide the list's own mail.
    let (_, also) = list_message(&store, 3, "", Some(thread), Presence::Fetched);
    assert_eq!((thread, thread), (same, also), "fixture: one thread");

    let out = exercise(
        &store,
        &unsubscribe(*thread.as_uuid(), UnsubscribeStep::Show),
    )
    .unwrap();
    assert!(out.contains("new@example.test"), "{out}");
    assert!(!out.contains("old@example.test"), "{out}");
}

#[test]
fn a_web_page_is_printed_and_nothing_is_sent() {
    let (store, _dir) = seeded();
    let (message, _) = list_message(
        &store,
        1,
        // One-click asked for, over http: never one-click, so only a page is left.
        "List-Unsubscribe: <http://example.test/leave?u=1>\r\n\
         List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n",
        None,
        Presence::Fetched,
    );
    let out = exercise(
        &store,
        &unsubscribe(*message.as_uuid(), UnsubscribeStep::Act),
    )
    .unwrap();
    assert!(out.contains("does not open"), "{out}");
    assert!(out.contains("http://example.test/leave?u=1"), "{out}");
    assert!(store.drafts(ACCOUNT).unwrap().is_empty());
    assert!(store.outbox_due(ACCOUNT, at(1_000)).unwrap().is_empty());
}

#[test]
fn a_message_that_offers_nothing_says_so() {
    let (store, _dir) = seeded();
    let (message, _) = list_message(&store, 1, "", None, Presence::Fetched);
    let err = exercise(
        &store,
        &unsubscribe(*message.as_uuid(), UnsubscribeStep::Act),
    )
    .expect_err("nothing to do is a failure");
    assert!(err.contains("no way to unsubscribe"), "{err}");
    let shown = exercise(
        &store,
        &unsubscribe(*message.as_uuid(), UnsubscribeStep::Show),
    )
    .unwrap();
    assert!(shown.contains("no way to unsubscribe"), "{shown}");
}

#[test]
fn a_message_whose_body_has_not_arrived_asks_for_a_sync() {
    let (store, _dir) = seeded();
    let (message, _) = list_message(&store, 1, MAILTO, None, Presence::HeadersOnly);
    let err = exercise(
        &store,
        &unsubscribe(*message.as_uuid(), UnsubscribeStep::Show),
    )
    .expect_err("no bytes to read");
    assert!(err.contains("mailo sync"), "{err}");
}

#[test]
fn an_unknown_id_is_named() {
    let (store, _dir) = seeded();
    let err = exercise(
        &store,
        &unsubscribe(uuid::Uuid::from_u128(42), UnsubscribeStep::Show),
    )
    .expect_err("nothing has that id");
    assert!(err.contains("neither a message nor a thread"), "{err}");
}

#[test]
fn the_command_line_reads_the_id_and_show() {
    let id = "00000000-0000-4000-8000-0000000000c1";
    let words = |w: &[&str]| w.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        cli::parse(&words(&["unsubscribe", id])).unwrap(),
        unsubscribe(id.parse().unwrap(), UnsubscribeStep::Act)
    );
    assert_eq!(
        cli::parse(&words(&["unsubscribe", id, "--show"])).unwrap(),
        unsubscribe(id.parse().unwrap(), UnsubscribeStep::Show)
    );
    for bad in [
        words(&["unsubscribe"]),
        words(&["unsubscribe", "nope"]),
        words(&["unsubscribe", id, "--now"]),
    ] {
        assert!(cli::parse(&bad).is_err(), "{bad:?}");
    }
    assert!(cli::usage().contains("unsubscribe <thread-or-message-id>"));
}

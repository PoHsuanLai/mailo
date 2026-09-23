//! Read receipts from the command line: `show` says a message asks, `receipt` answers it.
//!
//! Every answer is one the user typed. Nothing here, or anywhere, sends a receipt because a
//! message was opened.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::cli;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ASKING: MessageId = MessageId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn exercise(store: &SqliteStore, command: &cli::Command) -> Result<String, String> {
    cli::run_with_clients(
        store,
        command,
        at(100),
        &mail_runtime::OAuthRegistry::default(),
    )
}

fn raw_asking(dnt: &str) -> Vec<u8> {
    format!(
        "Return-Path: <ada@example.test>\r\n\
         From: Ada <ada@example.test>\r\n\
         To: me@example.test\r\n\
         Subject: figures\r\n\
         Message-ID: <figures@example.test>\r\n\
         {dnt}\
         \r\n\
         Please confirm.\r\n"
    )
    .into_bytes()
}

/// A store with one account and identity, holding one message built from `raw` — or its
/// headers only when `raw` is `None`.
fn seeded(raw: Option<Vec<u8>>) -> (SqliteStore, tempfile::TempDir, ThreadId) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test', '\"default\"')",
            rusqlite::params![IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    let blob = store
        .blobs()
        .put(&store.connection(), raw.as_deref().unwrap_or(b"headers"))
        .unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: ASKING,
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("figures@example.test".to_owned()),
        date: at(0),
        from: Address {
            name: Some("Ada".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "figures".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("figures@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: match raw {
            Some(_) => Body::Present {
                text: Some("Please confirm.".to_owned()),
                raw: blob,
            },
            None => Body::Absent,
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
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "INBOX".to_owned(),
                        uidvalidity: 7,
                        uid: 42,
                    },
                    key: message.key.clone(),
                    raw: blob,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    (store, dir, thread)
}

fn queued(store: &SqliteStore) -> Vec<ProtoOp> {
    store
        .outbox_due(ACCOUNT, at(1_000_000))
        .unwrap()
        .into_iter()
        .map(|entry| entry.op)
        .collect()
}

fn answer(message: MessageId, answer: ReceiptAnswer) -> cli::Command {
    cli::Command::Receipt { message, answer }
}

fn show(thread: ThreadId) -> cli::Command {
    cli::Command::Show { thread }
}

#[test]
fn show_says_a_message_asks_and_how_to_answer() {
    let (store, _dir, thread) = seeded(Some(raw_asking(
        "Disposition-Notification-To: ada@example.test\r\n",
    )));
    let out = exercise(&store, &show(thread)).unwrap();
    assert!(
        out.contains("asks for a read receipt, to ada@example.test"),
        "{out}"
    );
    assert!(out.contains(&format!("mailo receipt {ASKING}")), "{out}");
    assert!(!out.contains("careful"), "same domain: {out}");
    // Seeing it answers nothing.
    assert!(queued(&store).is_empty());
    assert_eq!(store.receipt_answer(ASKING).unwrap(), None);
}

#[test]
fn show_warns_when_the_receipt_would_go_elsewhere() {
    let (store, _dir, thread) = seeded(Some(raw_asking(
        "Disposition-Notification-To: tracker@collector.test\r\n",
    )));
    let out = exercise(&store, &show(thread)).unwrap();
    assert!(out.contains("careful"), "{out}");
    assert!(
        out.contains("ada@example.test"),
        "names the return path: {out}"
    );
}

#[test]
fn sending_queues_the_receipt_and_mdnsent_and_asks_once() {
    let (store, _dir, thread) = seeded(Some(raw_asking(
        "Disposition-Notification-To: ada@example.test\r\n",
    )));
    let out = exercise(&store, &answer(ASKING, ReceiptAnswer::Sent)).unwrap();
    assert!(
        out.contains("queued a read receipt to ada@example.test"),
        "{out}"
    );

    let ops = queued(&store);
    assert_eq!(ops.len(), 2, "{ops:?}");
    match &ops[0] {
        ProtoOp::Submit {
            raw,
            mail_from,
            rcpt_to,
            ..
        } => {
            assert_eq!(mail_from, "me@example.test");
            assert_eq!(rcpt_to, &vec!["ada@example.test".to_owned()]);
            let bytes = store.blobs().get(&store.connection(), *raw).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            assert!(
                text.contains("report-type=\"disposition-notification\""),
                "{text}"
            );
            assert!(text.contains("Original-Message-ID: <figures@example.test>"));
        }
        other => panic!("expected the receipt's submission first, got {other:?}"),
    }
    assert_eq!(
        ops[1],
        ProtoOp::AddKeyword {
            remotes: vec![RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 7,
                uid: 42,
            }],
            keyword: Keyword::MdnSent,
        }
    );
    assert_eq!(
        store.receipt_answer(ASKING).unwrap(),
        Some(ReceiptAnswer::Sent)
    );

    let out = exercise(&store, &show(thread)).unwrap();
    assert!(out.contains("read receipt sent"), "{out}");
    assert!(!out.contains("asks for a read receipt"), "{out}");
    // Asked once: neither a second receipt nor a late decline.
    for again in [ReceiptAnswer::Sent, ReceiptAnswer::Declined] {
        assert!(exercise(&store, &answer(ASKING, again)).is_err());
    }
    assert_eq!(queued(&store).len(), 2);
}

#[test]
fn declining_sends_nothing_but_still_settles_the_question() {
    let (store, _dir, thread) = seeded(Some(raw_asking(
        "Disposition-Notification-To: ada@example.test\r\n",
    )));
    let out = exercise(&store, &answer(ASKING, ReceiptAnswer::Declined)).unwrap();
    assert!(out.contains("declined"), "{out}");
    let ops = queued(&store);
    assert!(
        !ops.iter().any(|op| matches!(op, ProtoOp::Submit { .. })),
        "{ops:?}"
    );
    // RFC 3503: `$MDNSent` for a declined receipt too, so other clients do not ask again.
    assert!(
        ops.iter().any(|op| matches!(
            op,
            ProtoOp::AddKeyword {
                keyword: Keyword::MdnSent,
                ..
            }
        )),
        "{ops:?}"
    );
    let out = exercise(&store, &show(thread)).unwrap();
    assert!(out.contains("read receipt declined"), "{out}");
    assert!(exercise(&store, &answer(ASKING, ReceiptAnswer::Sent)).is_err());
}

#[test]
fn a_message_that_did_not_ask_is_not_answered() {
    let (store, _dir, thread) = seeded(Some(raw_asking("")));
    let out = exercise(&store, &show(thread)).unwrap();
    assert!(!out.contains("receipt"), "{out}");
    let err = exercise(&store, &answer(ASKING, ReceiptAnswer::Sent)).unwrap_err();
    assert!(err.contains("did not ask"), "{err}");
    assert!(queued(&store).is_empty());
}

#[test]
fn headers_alone_cannot_say_whether_a_message_asks() {
    let (store, _dir, _) = seeded(None);
    let err = exercise(&store, &answer(ASKING, ReceiptAnswer::Sent)).unwrap_err();
    assert!(err.contains("mailo sync"), "{err}");
    assert_eq!(store.receipt_answer(ASKING).unwrap(), None);
}

#[test]
fn the_receipt_command_parses() {
    let args = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        cli::parse(&args(&["receipt", &ASKING.to_string()])),
        Ok(answer(ASKING, ReceiptAnswer::Sent))
    );
    assert_eq!(
        cli::parse(&args(&["receipt", &ASKING.to_string(), "--decline"])),
        Ok(answer(ASKING, ReceiptAnswer::Declined))
    );
    for bad in [
        args(&["receipt"]),
        args(&["receipt", "not-a-uuid"]),
        args(&["receipt", &ASKING.to_string(), "--deny"]),
        args(&["receipt", &ASKING.to_string(), "--decline", "extra"]),
    ] {
        assert!(cli::parse(&bad).is_err(), "{bad:?}");
    }
    assert!(cli::usage().contains("receipt <message-id> [--decline]"));
}

#[test]
fn compose_can_ask_for_a_receipt_in_either_word_order() {
    let args = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    for flag in ["--request-receipt", "--receipt-request"] {
        match cli::parse(&args(&["compose", "--to", "kim@elsewhere.test", flag])) {
            Ok(cli::Command::Compose { receipt, .. }) => {
                assert_eq!(receipt, ReceiptRequest::Requested, "{flag}")
            }
            other => panic!("{flag}: {other:?}"),
        }
    }
    match cli::parse(&args(&["compose", "--to", "kim@elsewhere.test"])) {
        Ok(cli::Command::Compose { receipt, .. }) => {
            assert_eq!(receipt, ReceiptRequest::Unrequested)
        }
        other => panic!("{other:?}"),
    }
}

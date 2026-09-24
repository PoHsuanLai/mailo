//! Read receipts in the window. Opening never answers; only the bar's buttons do.

use super::{Line, Standing, line, looked_at};
use crate::receipt::ReceiptState;
use crate::ui::fixtures::{ACCOUNT, Seen, click, dispatching, rebuild_into, seeded};
use crate::ui::reading::Reader;
use crate::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::*;
use mail_mime::{ReceiptAsk, ReturnPath};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

const ASKS: &str = "Disposition-Notification-To: ada@example.test\r\n";
const ASKS_ELSEWHERE: &str = "Disposition-Notification-To: tracker@elsewhere.test\r\n";

/// Whether the body was fetched, or only its headers.
#[derive(Clone, Copy, PartialEq)]
enum Held {
    Body,
    HeadersOnly,
}

/// One message from Ada in its own thread, with `headers` in its stored bytes.
fn put(store: &SqliteStore, headers: &str, held: Held) -> (ThreadId, MessageId) {
    let rfc = format!("{}@example.test", uuid::Uuid::new_v4());
    let bytes = format!(
        "Return-Path: <ada@example.test>\r\nFrom: Ada <ada@example.test>\r\n\
         To: me@example.test\r\nSubject: figures\r\nMessage-ID: <{rfc}>\r\n\
         {headers}\r\nPlease confirm.\r\n"
    );
    let raw = store
        .blobs()
        .put(&store.connection(), bytes.as_bytes())
        .unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(rfc.clone()),
        date: chrono::Utc::now(),
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
        rfc_message_id: Some(rfc.clone()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: match held {
            Held::Body => Body::Present {
                text: Some("Please confirm.".to_owned()),
                raw,
            },
            Held::HeadersOnly => Body::Absent,
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
                    remote: RemoteRef::Pop { uidl: rfc },
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
    (store.message(id).unwrap().thread, id)
}

fn far() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::TimeDelta::days(365)
}

/// Every submission waiting in the outbox.
fn submissions(store: &SqliteStore) -> usize {
    store
        .outbox_due(ACCOUNT, far())
        .unwrap()
        .into_iter()
        .filter(|entry| matches!(entry.op, ProtoOp::Submit { .. }))
        .count()
}

fn outbox(store: &SqliteStore) -> usize {
    store.outbox_due(ACCOUNT, far()).unwrap().len()
}

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    rsx! { Reader { thread, shell } }
}

/// Let the dom's tasks run for `for_ms`, keeping every attribute the renders set.
async fn settle(dom: &mut VirtualDom, seen: &mut Seen, for_ms: u64) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_millis(for_ms);
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = tokio::time::timeout(left, dom.wait_for_work()).await;
        dom.render_immediate(seen);
    }
}

/// The reader on `thread`, once its lookups have landed.
async fn reader_on(store: Arc<SqliteStore>, thread: ThreadId) -> (VirtualDom, Seen, String) {
    dispatching();
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread }).with_root_context(store);
    let mut seen = rebuild_into(&mut dom);
    settle(&mut dom, &mut seen, 400).await;
    // The renderer escapes the apostrophe; the words are compared as they read.
    let markup = dioxus_ssr::render(&dom).replace("&#39;", "'");
    (dom, seen, markup)
}

fn ask(to: &str, return_path: ReturnPath) -> ReceiptState {
    ReceiptState::Pending(ReceiptAsk {
        to: vec![Address {
            name: None,
            email: to.to_owned(),
        }],
        return_path,
    })
}

#[test]
fn the_bar_says_who_asks_and_warns_when_the_receipt_goes_elsewhere() {
    let asking = |state| Standing {
        message: MessageId::generate(),
        sender: "Ada".to_owned(),
        state,
    };
    let sentence = "Ada asked to be told when you've read this.".to_owned();
    let cases = [
        (
            "agrees",
            ask("ada@example.test", ReturnPath::Agrees),
            Some(Line::Asking {
                sentence: sentence.clone(),
                warning: None,
            }),
        ),
        (
            "no return path",
            ask("ada@example.test", ReturnPath::Unknown),
            Some(Line::Asking {
                sentence: sentence.clone(),
                warning: None,
            }),
        ),
        (
            "differs",
            ask(
                "tracker@elsewhere.test",
                ReturnPath::Differs {
                    return_path: "ada@example.test".to_owned(),
                },
            ),
            Some(Line::Asking {
                sentence,
                warning: Some(
                    "The receipt would go to tracker@elsewhere.test, not to ada@example.test, \
                     where this message came from."
                        .to_owned(),
                ),
            }),
        ),
        (
            "sent",
            ReceiptState::Answered(ReceiptAnswer::Sent),
            Some(Line::Settled("Receipt sent")),
        ),
        (
            "declined",
            ReceiptState::Answered(ReceiptAnswer::Declined),
            Some(Line::Settled("Receipt declined")),
        ),
        ("not asked", ReceiptState::NotAsked, None),
        ("headers only", ReceiptState::Unknown, None),
    ];
    for (case, state, want) in cases {
        assert_eq!(line(&asking(state)), want, "{case}");
    }
}

#[tokio::test]
async fn a_message_that_asks_shows_the_bar_under_the_head() {
    let (store, _dir) = seeded();
    let (thread, _) = put(&store, ASKS, Held::Body);
    let (_, _, markup) = reader_on(store, thread).await;
    assert!(
        markup.contains("Ada asked to be told when you've read this."),
        "{markup}"
    );
    assert!(markup.contains("aria-label=\"Send receipt\""), "{markup}");
    assert!(markup.contains("aria-label=\"Don't send\""), "{markup}");
    assert!(!markup.contains("class=\"warn\""), "{markup}");
    // Under the head, not among the messages.
    let head_ends = markup.find("class=\"reader-body\"").unwrap();
    assert!(markup.find("class=\"receipt\"").unwrap() < head_ends);
}

#[tokio::test]
async fn a_request_to_another_domain_is_warned_about() {
    let (store, _dir) = seeded();
    let (thread, _) = put(&store, ASKS_ELSEWHERE, Held::Body);
    let (_, _, markup) = reader_on(store, thread).await;
    assert!(
        markup.contains(
            "The receipt would go to tracker@elsewhere.test, not to ada@example.test, \
             where this message came from."
        ),
        "{markup}"
    );
    assert!(markup.contains("aria-label=\"Send receipt\""), "{markup}");
}

#[tokio::test]
async fn an_answered_request_is_a_note_and_nothing_to_press() {
    let (store, _dir) = seeded();
    let (sent, sent_id) = put(&store, ASKS, Held::Body);
    let (declined, declined_id) = put(&store, ASKS, Held::Body);
    let now = chrono::Utc::now();
    crate::receipt::answer(&store, sent_id, ReceiptAnswer::Sent, now).unwrap();
    crate::receipt::answer(&store, declined_id, ReceiptAnswer::Declined, now).unwrap();
    for (thread, note) in [(sent, "Receipt sent"), (declined, "Receipt declined")] {
        let (_, _, markup) = reader_on(store.clone(), thread).await;
        assert!(
            markup.contains(&format!("class=\"receipt-note mono\">{note}</p>")),
            "{markup}"
        );
        assert!(!markup.contains("Send receipt"), "{markup}");
    }
}

#[tokio::test]
async fn a_message_that_does_not_ask_or_is_not_here_yet_shows_nothing() {
    let (store, _dir) = seeded();
    let (quiet, _) = put(&store, "", Held::Body);
    let (unfetched, _) = put(&store, ASKS, Held::HeadersOnly);
    for thread in [quiet, unfetched] {
        let (_, _, markup) = reader_on(store.clone(), thread).await;
        assert!(!markup.contains("receipt"), "{markup}");
    }
}

#[tokio::test]
async fn opening_a_message_that_asks_answers_nothing_and_queues_nothing() {
    let (store, _dir) = seeded();
    let (thread, message) = put(&store, ASKS, Held::Body);
    let queued = outbox(&store);
    let (_, _, markup) = reader_on(store.clone(), thread).await;
    // It was looked at and shown — the assertion below is about a reader that did its work.
    assert!(looked_at(message) && markup.contains("Send receipt"));
    assert_eq!(store.receipt_answer(message).unwrap(), None);
    assert_eq!(outbox(&store), queued, "opening queued something");
}

#[tokio::test]
async fn send_receipt_queues_exactly_one_receipt_and_settles_the_bar() {
    let (store, _dir) = seeded();
    let (thread, message) = put(&store, ASKS, Held::Body);
    let before = submissions(&store);
    let (mut dom, seen, _) = reader_on(store.clone(), thread).await;

    let mut after = click(&mut dom, seen.one("aria-label", "Send receipt"));
    settle(&mut dom, &mut after, 400).await;
    let markup = dioxus_ssr::render(&dom);

    assert_eq!(submissions(&store), before + 1, "one receipt, queued");
    assert_eq!(
        store.receipt_answer(message).unwrap(),
        Some(ReceiptAnswer::Sent)
    );
    assert!(
        markup.contains("class=\"receipt-note mono\">Receipt sent</p>"),
        "{markup}"
    );
    assert!(!markup.contains("aria-label=\"Send receipt\""), "{markup}");
}

#[tokio::test]
async fn dont_send_queues_no_receipt() {
    let (store, _dir) = seeded();
    let (thread, message) = put(&store, ASKS, Held::Body);
    let before = submissions(&store);
    let (mut dom, seen, _) = reader_on(store.clone(), thread).await;

    let mut after = click(&mut dom, seen.one("aria-label", "Don't send"));
    settle(&mut dom, &mut after, 400).await;

    assert_eq!(submissions(&store), before, "a declined receipt was queued");
    assert_eq!(
        store.receipt_answer(message).unwrap(),
        Some(ReceiptAnswer::Declined)
    );
    assert!(dioxus_ssr::render(&dom).contains("Receipt declined"));
}

#[test]
fn answering_twice_is_refused_and_queues_nothing_more() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, ASKS, Held::Body);
    let now = chrono::Utc::now();
    let said = super::answer(&store, message, ReceiptAnswer::Sent, now).unwrap();
    assert_eq!(said, "Queued a read receipt to ada@example.test");
    let queued = submissions(&store);
    assert!(super::answer(&store, message, ReceiptAnswer::Sent, now).is_err());
    assert_eq!(submissions(&store), queued);
}

#[tokio::test]
#[ignore = "writes target/receipt.html for a person or a headless browser to look at"]
async fn render_the_receipt_bar_to_a_file() {
    let (store, _dir) = seeded();
    let mut body = String::new();
    let (asking, _) = put(&store, ASKS, Held::Body);
    let (elsewhere, _) = put(&store, ASKS_ELSEWHERE, Held::Body);
    let (answered, answered_id) = put(&store, ASKS, Held::Body);
    crate::receipt::answer(&store, answered_id, ReceiptAnswer::Sent, chrono::Utc::now()).unwrap();
    for thread in [asking, elsewhere, answered] {
        let (_, _, reader) = reader_on(store.clone(), thread).await;
        body.push_str(&format!(
            "<section class=\"reader\" style=\"width:620px;height:250px;margin:16px\">{reader}</section>"
        ));
    }
    crate::ui::fixtures::dump("receipt", &body);
}

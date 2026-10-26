//! Read receipts in the window: answered by the setting, once, and only where it is safe.

use super::{Standing, answer_asked, automatic};
use crate::settings::{MailSettings, ReadReceipts};
use crate::ui::fixtures::settle_for;
use crate::ui::fixtures::{acct_account, dispatching, rebuild_into, seeded};
use crate::ui::reading::Reader;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_core::message::{Looks, Read};
use mail_core::receipt::ReceiptState;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use mail_mime::{ReceiptAsk, ReturnPath};
use std::sync::Arc;
use std::time::Duration;

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
    let raw = store.blobs().put(bytes.as_bytes()).unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: acct_account(),
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
            acct_account(),
            Ingest {
                mailbox: MailboxRef {
                    account: acct_account(),
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
                provider_threads: Vec::new(),
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
        .outbox_due(acct_account(), far())
        .unwrap()
        .into_iter()
        .filter(|entry| matches!(entry.op, ProtoOp::Submit { .. }))
        .count()
}

/// What is queued for the server beyond flags: opening a conversation marks it read, which is
/// not a receipt.
fn outbox(store: &SqliteStore) -> usize {
    store
        .outbox_due(acct_account(), far())
        .unwrap()
        .into_iter()
        .filter(|entry| !matches!(entry.op, ProtoOp::SetFlags { .. }))
        .count()
}

#[component]
fn Open(thread: ThreadId, policy: Option<ReadReceipts>) -> Element {
    let shell = use_signal(Shell::default);
    // A window's settings, when the case has one: a reader with none answers nothing.
    use_hook(|| {
        if let Some(policy) = policy {
            let mut settings = MailSettings::default();
            settings.reading.read_receipts = policy;
            provide_context(Signal::new(settings));
        }
    });
    rsx! { ds::prelude::Ds { appearance: ds::prelude::Appearance::default(), material: ds::prelude::Material::Window, Reader { thread, shell } } }
}

/// The reader on `thread` under `policy` (none: no window's settings), once its lookups have
/// landed, its markup, and the lookups it used.
async fn reader_under(
    store: Arc<SqliteStore>,
    thread: ThreadId,
    policy: Option<ReadReceipts>,
) -> (String, Arc<Looks>) {
    dispatching();
    let looks = Arc::new(Looks::new());
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread, policy })
        .with_root_context(store)
        .with_root_context(looks.clone());
    let mut seen = rebuild_into(&mut dom);
    settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
    (dioxus_ssr::render(&dom), looks)
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

/// The setting's whole table: only Always sends, and only for an unanswered request whose
/// receipt goes back where the message came from, outside Junk.
#[test]
fn the_setting_sends_only_where_it_is_safe() {
    let agrees = ask("ada@example.test", ReturnPath::Agrees);
    let differs = ask(
        "tracker@elsewhere.test",
        ReturnPath::Differs {
            return_path: "ada@example.test".to_owned(),
        },
    );
    let unknown = ask("ada@example.test", ReturnPath::Unknown);
    let sent = Some(ReceiptAnswer::Sent);
    let cases: Vec<(
        &str,
        ReadReceipts,
        ReceiptState,
        MailboxRole,
        Option<ReceiptAnswer>,
    )> = vec![
        (
            "always",
            ReadReceipts::Always,
            agrees.clone(),
            MailboxRole::Inbox,
            sent,
        ),
        (
            "always, archived",
            ReadReceipts::Always,
            agrees.clone(),
            MailboxRole::Archive,
            sent,
        ),
        (
            "never",
            ReadReceipts::Never,
            agrees.clone(),
            MailboxRole::Inbox,
            None,
        ),
        (
            "always, in junk",
            ReadReceipts::Always,
            agrees.clone(),
            MailboxRole::Spam,
            None,
        ),
        (
            "always, to another domain",
            ReadReceipts::Always,
            differs,
            MailboxRole::Inbox,
            None,
        ),
        (
            "always, no return path",
            ReadReceipts::Always,
            unknown,
            MailboxRole::Inbox,
            None,
        ),
        (
            "always, already sent",
            ReadReceipts::Always,
            ReceiptState::Answered(ReceiptAnswer::Sent),
            MailboxRole::Inbox,
            None,
        ),
        (
            "always, already declined",
            ReadReceipts::Always,
            ReceiptState::Answered(ReceiptAnswer::Declined),
            MailboxRole::Inbox,
            None,
        ),
        (
            "always, not asked",
            ReadReceipts::Always,
            ReceiptState::NotAsked,
            MailboxRole::Inbox,
            None,
        ),
        (
            "always, headers only",
            ReadReceipts::Always,
            ReceiptState::Unknown,
            MailboxRole::Inbox,
            None,
        ),
    ];
    for (case, policy, state, mailbox, want) in cases {
        assert_eq!(automatic(policy, &state, mailbox), want, "{case}");
    }
}

/// Under Never, or with no window's settings, opening a message that asks answers nothing,
/// queues nothing and reads nothing to find out; the reader says nothing about it either.
#[tokio::test]
async fn opening_under_never_answers_nothing_and_queues_nothing() {
    let (store, _dir) = seeded();
    let (thread, message) = put(&store, ASKS, Held::Body);
    let queued = outbox(&store);
    for policy in [None, Some(ReadReceipts::Never)] {
        let (markup, looks) = reader_under(store.clone(), thread, policy).await;
        assert_eq!(looks.reads(Read::Receipt), 0, "{policy:?}: looked it up");
        assert!(!markup.contains("receipt"), "{policy:?}: {markup}");
        assert_eq!(store.receipt_answer(message).unwrap(), None, "{policy:?}");
        assert_eq!(
            outbox(&store),
            queued,
            "{policy:?}: opening queued something"
        );
    }
}

/// Under Always, the first opening queues exactly one receipt and says nothing on the screen;
/// opening it again queues no second one.
#[tokio::test]
async fn opening_under_always_queues_one_receipt_once() {
    let (store, _dir) = seeded();
    let (thread, message) = put(&store, ASKS, Held::Body);
    let before = submissions(&store);
    let (markup, _) = reader_under(store.clone(), thread, Some(ReadReceipts::Always)).await;
    assert_eq!(submissions(&store), before + 1, "one receipt, queued");
    assert_eq!(
        store.receipt_answer(message).unwrap(),
        Some(ReceiptAnswer::Sent)
    );
    assert!(!markup.contains("receipt"), "{markup}");

    let _ = reader_under(store.clone(), thread, Some(ReadReceipts::Always)).await;
    assert_eq!(submissions(&store), before + 1, "a second receipt");
}

/// Under Always, a request whose receipt would go to another domain, and a message whose body
/// is not here yet, are left as they came.
#[tokio::test]
async fn always_leaves_another_domain_and_an_unfetched_body_alone() {
    let (store, _dir) = seeded();
    let (elsewhere, elsewhere_id) = put(&store, ASKS_ELSEWHERE, Held::Body);
    let (unfetched, unfetched_id) = put(&store, ASKS, Held::HeadersOnly);
    let queued = outbox(&store);
    for (thread, message) in [(elsewhere, elsewhere_id), (unfetched, unfetched_id)] {
        let _ = reader_under(store.clone(), thread, Some(ReadReceipts::Always)).await;
        assert_eq!(store.receipt_answer(message).unwrap(), None);
    }
    assert_eq!(outbox(&store), queued, "something was queued");
}

/// Answering the same standings twice queues one receipt: the store's answer, read again, is
/// what the second pass finds.
#[test]
fn answering_twice_queues_one_receipt() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, ASKS, Held::Body);
    let looks = Looks::new();
    let standings = vec![Standing {
        message,
        sender: "Ada".to_owned(),
        state: ask("ada@example.test", ReturnPath::Agrees),
    }];
    let now = chrono::Utc::now();
    let before = submissions(&store);
    assert_eq!(
        answer_asked(&looks, &store, &standings, ReadReceipts::Always, now),
        [message]
    );
    assert!(answer_asked(&looks, &store, &standings, ReadReceipts::Always, now).is_empty());
    assert_eq!(submissions(&store), before + 1);
}

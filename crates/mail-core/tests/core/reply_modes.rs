//! The other ways to answer or pass on a message: a reply quoting only the selection, a reply to
//! the list, a redirect, and a new message made from an old one (`compose::modes`).
//!
//! Each is driven against a store with one account that can send, and messages whose stored bytes
//! are real messages, since a list's address and a redirect both come out of those bytes.

use crate::support::account;
use chrono::{DateTime, TimeZone, Utc};
use mail_core::CoreError;
use mail_core::compose::{self, Answer, Quote};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ME: IdentityId = IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ALIAS: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

/// A store whose one account sends through `outgoing`, as `me@example.test` and, second, an
/// alias.
fn seeded(outgoing: Outgoing) -> SqliteStore {
    let store = mail_store::testing::in_memory();
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
    plan.outgoing = outgoing;
    mail_store::testing::seed_account_plan(&store, account(), "me@example.test", &plan, None);
    mail_store::testing::seed_identity_for(&store, ME, account(), "me@example.test", Some("Me"));
    mail_store::testing::seed_identity_for(&store, ALIAS, account(), "alias@example.test", None);
    store
}

fn smtp() -> Outgoing {
    Outgoing::Smtp {
        host: "smtp.example.test".to_owned(),
        port: 465,
        tls: Tls::Implicit,
    }
}

/// What a message is, for [`message`].
struct Stored<'a> {
    from: Address,
    to: Vec<Address>,
    cc: Vec<Address>,
    bcc: Vec<Address>,
    /// Header fields beyond the usual ones, each ending `\r\n`.
    extra: &'a str,
    text: &'a str,
    attachment: Option<&'a [u8]>,
}

impl Default for Stored<'_> {
    fn default() -> Self {
        Stored {
            from: addr(Some("Ada Lovelace"), "ada@example.test"),
            to: vec![addr(None, "me@example.test")],
            cc: vec![addr(None, "cara@example.test")],
            bcc: vec![],
            extra: "",
            text: "Shall we say one o'clock?\r\nOr two.",
            attachment: None,
        }
    }
}

/// Store `stored` in the inbox, its bytes a real message carrying its header fields.
fn message(store: &SqliteStore, stored: Stored<'_>) -> MessageId {
    let id = MessageId::generate();
    let rfc = format!("{}@example.test", uuid::Uuid::new_v4());
    let bytes = format!(
        "From: {}\r\nTo: {}\r\nCc: {}\r\nDate: Tue, 14 Nov 2023 22:13:20 +0000\r\n\
         Message-ID: <{rfc}>\r\nSubject: lunch on friday\r\n{}\r\n{}\r\n",
        Address::join(std::slice::from_ref(&stored.from)),
        Address::join(&stored.to),
        Address::join(&stored.cc),
        stored.extra,
        stored.text,
    );
    let raw = store.blobs().put(bytes.as_bytes()).unwrap();
    let attachments = stored
        .attachment
        .map(|bytes| Attachment {
            name: "plan.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            size: bytes.len() as u64,
            content: PartContent::Held(store.blobs().put(bytes).unwrap()),
            inline: Inline::Attached,
        })
        .into_iter()
        .collect();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: account(),
        key: MessageKey::Rfc(rfc.clone()),
        date: at(0),
        from: stored.from,
        reply_to: vec![],
        to: stored.to,
        cc: stored.cc,
        bcc: stored.bcc,
        subject: "lunch on friday".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(rfc),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(stored.text.to_owned()),
            raw,
        },
        attachments,
    };
    store
        .ingest(
            account(),
            Ingest {
                mailbox: MailboxRef {
                    account: account(),
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop {
                        uidl: uuid::Uuid::new_v4().to_string(),
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
    id
}

fn emails(list: &[Address]) -> Vec<&str> {
    list.iter().map(|a| a.email.as_str()).collect()
}

/// The quoted lines of a reply's text, without their `> `.
fn quoted(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.strip_prefix("> "))
        .collect()
}

#[test]
fn a_reply_to_a_selection_quotes_only_what_was_selected() {
    let store = seeded(smtp());
    let id = message(&store, Stored::default());
    let selected = compose::draft_answer_in(
        &store,
        id,
        Answer::Scope(ReplyScope::Sender),
        Quote::Selection("one o'clock\n"),
        "",
        at(10),
        &Utc,
    )
    .unwrap();
    assert_eq!(quoted(&selected.text), ["one o'clock"]);
    assert!(
        selected.text.contains(", Ada Lovelace wrote:"),
        "the attribution line stays: {}",
        selected.text
    );
    assert_eq!(emails(&selected.to), ["ada@example.test"]);
    assert_eq!(selected.in_reply_to, Some(id));

    // Nothing but space selected is no selection: the whole of it is quoted.
    let whole = compose::draft_answer_in(
        &store,
        id,
        Answer::Scope(ReplyScope::Sender),
        Quote::Selection(" \n "),
        "",
        at(10),
        &Utc,
    )
    .unwrap();
    assert_eq!(
        quoted(&whole.text),
        ["Shall we say one o'clock?", "Or two."]
    );
    assert_eq!(store.draft(selected.id).unwrap().text, selected.text);
}

#[test]
fn a_reply_to_the_list_goes_to_its_list_post_address_and_nobody_else() {
    let store = seeded(smtp());
    let id = message(
        &store,
        Stored {
            extra: "List-Id: Rust <rust.lists.example.test>\r\n\
                    List-Post: <mailto:rust@lists.example.test>\r\n",
            ..Stored::default()
        },
    );
    let draft = compose::draft_answer(&store, id, Answer::List, Quote::Whole, at(10)).unwrap();
    assert_eq!(emails(&draft.to), ["rust@lists.example.test"]);
    assert!(draft.cc.is_empty() && draft.bcc.is_empty());
    assert_eq!(draft.subject, "Re: lunch on friday");
    assert_eq!(draft.in_reply_to, Some(id), "it threads as a reply");
    assert_eq!(
        quoted(&draft.text),
        ["Shall we say one o'clock?", "Or two."]
    );

    let original = store.message(id).unwrap();
    assert_eq!(
        emails(&compose::list_post(&store, &original).unwrap()),
        ["rust@lists.example.test"]
    );

    for extra in ["", "List-Post: NO\r\n"] {
        let plain = message(
            &store,
            Stored {
                extra,
                ..Stored::default()
            },
        );
        assert!(
            matches!(
                compose::draft_answer(&store, plain, Answer::List, Quote::Whole, at(10)),
                Err(CoreError::NoListPost)
            ),
            "{extra:?}"
        );
    }
}

#[test]
fn edit_as_new_copies_recipients_subject_text_and_files_and_answers_nothing() {
    let store = seeded(smtp());
    // A message of the person's own, from the alias, as Sent keeps it: its blind copy named.
    let sent = message(
        &store,
        Stored {
            from: addr(None, "alias@example.test"),
            to: vec![addr(Some("Bea"), "bea@example.test")],
            cc: vec![addr(None, "cara@example.test")],
            bcc: vec![addr(None, "dee@example.test")],
            text: "The plan is attached.\r\n-- \r\nMe",
            attachment: Some(b"%PDF-1.4 plan"),
            ..Stored::default()
        },
    );
    let draft = compose::draft_again(&store, sent, at(10)).unwrap();
    assert_eq!(
        draft.identity, ALIAS,
        "it leaves from the address it left from"
    );
    assert_eq!(emails(&draft.to), ["bea@example.test"]);
    assert_eq!(emails(&draft.cc), ["cara@example.test"]);
    assert_eq!(emails(&draft.bcc), ["dee@example.test"]);
    assert_eq!(draft.subject, "lunch on friday", "no Re: or Fwd:");
    assert_eq!(
        draft.text, "The plan is attached.\r\n-- \r\nMe",
        "not signed a second time"
    );
    assert_eq!(draft.attachments.len(), 1);
    assert_eq!(draft.attachments[0].name, "plan.pdf");
    assert_eq!(
        store.blobs().get(draft.attachments[0].blob).unwrap(),
        b"%PDF-1.4 plan"
    );
    assert_eq!((draft.in_reply_to, draft.forward_of), (None, None));
    assert_eq!(draft.state, SendState::Editing);
    assert_eq!(store.draft(draft.id).unwrap(), draft, "it is saved");

    // Someone else's message leaves from the address it reached.
    let received = message(&store, Stored::default());
    assert_eq!(
        compose::draft_again(&store, received, at(10))
            .unwrap()
            .identity,
        ME
    );
}

#[test]
fn a_redirect_queues_the_original_unchanged_to_the_new_recipients() {
    let store = seeded(smtp());
    let id = message(
        &store,
        Stored {
            to: vec![addr(None, "alias@example.test")],
            ..Stored::default()
        },
    );
    let original = store.message(id).unwrap();
    let before = store.blobs().get(original.body.raw().unwrap()).unwrap();

    let to = [addr(Some("Bea"), "bea@example.test")];
    let went = compose::redirect(&store, id, &to, at(10)).unwrap();
    assert_eq!(went, ["bea@example.test"]);

    let due = store.outbox_due(account(), at(20)).unwrap();
    assert_eq!(due.len(), 1, "one submission");
    let ProtoOp::Submit {
        draft,
        raw,
        mail_from,
        rcpt_to,
        ..
    } = &due[0].op
    else {
        panic!("a submission, not {:?}", due[0].op);
    };
    assert_eq!(
        mail_from, "alias@example.test",
        "from the address it reached"
    );
    assert_eq!(rcpt_to, &["bea@example.test".to_owned()]);
    assert!(store.draft(*draft).is_err(), "no draft is made for it");
    let queued = store.blobs().get(*raw).unwrap();
    assert!(queued.ends_with(&before), "the original goes as it came");
    let text = String::from_utf8(queued).unwrap();
    assert!(text.starts_with("Resent-Date: "), "{text}");
    assert!(
        text.contains("Resent-From: <alias@example.test>\r\n"),
        "{text}"
    );
    assert!(
        text.contains("Resent-To: \"Bea\" <bea@example.test>\r\n"),
        "{text}"
    );
    assert!(text.contains(&format!("Resent-Message-ID: <{draft}@example.test>\r\n")));
}

#[test]
fn a_redirect_that_cannot_go_as_it_came_is_refused() {
    let to = [addr(None, "bea@example.test")];

    let graph = seeded(Outgoing::Graph);
    let id = message(&graph, Stored::default());
    assert!(matches!(
        compose::redirect(&graph, id, &to, at(10)),
        Err(CoreError::RedirectThroughGraph { .. })
    ));
    assert!(graph.outbox_due(account(), at(20)).unwrap().is_empty());

    let store = seeded(smtp());
    let id = message(&store, Stored::default());
    assert!(matches!(
        compose::redirect(&store, id, &[addr(None, " ")], at(10)),
        Err(CoreError::Mime(_))
    ));
    assert!(store.outbox_due(account(), at(20)).unwrap().is_empty());
}

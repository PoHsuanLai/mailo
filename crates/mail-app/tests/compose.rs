//! Replying and sending from the command line: `plan.md` phase 4's third verb.
//!
//! Phase 4 asks that "a tiny CLI can list, open and reply". The first two had tests from the
//! start. Reply had no implementation at all until this file's subject existed, because the
//! path it needs — a draft that persists, an identity to send as, an envelope that survives
//! into the outbox — was missing at every one of those three points.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

#[path = "../src/compose.rs"]
mod compose;
#[allow(dead_code)]
#[path = "../src/query.rs"]
mod query;
// Only the composer's half of `view` is used here; the rest belongs to the window, which this
// test deliberately does not build.
#[allow(dead_code)]
#[path = "../src/view.rs"]
mod view;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ORIGINAL: MessageId =
    MessageId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: ACCOUNT,
        from: Address {
            name: None,
            email: "me@example.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

/// A store with one account that can actually send, and one message to reply to.
fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();

    let mut plan = mail_domain::presets::preset_for("me@ntu.edu.tw", at(0))
        .expect("ntu is a known domain")
        .plan;
    plan.address = "me@example.test".to_owned();
    plan.identities = vec![identity()];

    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            rusqlite::params![IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }

    let raw = store
        .blobs()
        .put(&store.connection(), b"raw original")
        .unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: ORIGINAL,
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("original@example.test".to_owned()),
        date: at(0),
        from: Address {
            name: Some("Ada Lovelace".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![
            Address {
                name: None,
                email: "me@example.test".to_owned(),
            },
            Address {
                name: None,
                email: "bea@example.test".to_owned(),
            },
        ],
        cc: vec![Address {
            name: None,
            email: "cara@example.test".to_owned(),
        }],
        bcc: vec![],
        subject: "lunch on friday".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("original@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("Shall we say one o'clock?".to_owned()),
            raw,
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
                        uidl: "u1".to_owned(),
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
    (store, dir)
}

/// The draft a reply produced, read back from the store.
fn only_draft(store: &SqliteStore) -> Draft {
    let drafts = store.drafts(ACCOUNT).unwrap();
    assert_eq!(drafts.len(), 1, "expected exactly one draft");
    drafts.into_iter().next().unwrap()
}

#[test]
fn replying_creates_a_draft_addressed_to_the_sender() {
    let (store, _dir) = seeded();
    let out = compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .expect("reply succeeds");

    let draft = only_draft(&store);
    assert_eq!(draft.subject, "Re: lunch on friday");
    assert_eq!(draft.to.len(), 1);
    assert_eq!(draft.to[0].email, "ada@example.test");
    assert!(draft.cc.is_empty(), "a plain reply does not copy anyone");
    assert_eq!(draft.in_reply_to, Some(ORIGINAL));
    assert_eq!(draft.identity, IDENTITY);
    // The id is printed because the next command needs it.
    assert!(out.contains(&draft.id.to_string()), "{out}");
    assert!(out.contains("mailo send"), "{out}");
}

#[test]
fn replying_to_all_keeps_everyone_but_us() {
    let (store, _dir) = seeded();
    compose::reply(&store, ORIGINAL, ReplyScope::All, "sounds good", at(10)).unwrap();

    let draft = only_draft(&store);
    let to: Vec<&str> = draft.to.iter().map(|a| a.email.as_str()).collect();
    assert!(to.contains(&"ada@example.test"), "{to:?}");
    assert!(to.contains(&"bea@example.test"), "{to:?}");
    assert!(
        !to.contains(&"me@example.test") && !draft.cc.iter().any(|a| a.email == "me@example.test"),
        "a reply-all that mails ourselves: {draft:?}"
    );
    assert_eq!(
        draft
            .cc
            .iter()
            .map(|a| a.email.as_str())
            .collect::<Vec<_>>(),
        vec!["cara@example.test"],
        "the original Cc stays Cc"
    );
}

#[test]
fn the_reply_body_quotes_the_original_beneath_what_was_written() {
    let (store, _dir) = seeded();
    compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .unwrap();

    let text = only_draft(&store).text;
    let written = text
        .find("one o'clock suits")
        .expect("the new text is there");
    let quoted = text
        .find("> Shall we say one o'clock?")
        .expect("the original is quoted");
    assert!(
        written < quoted,
        "the quote must come after the reply:\n{text}"
    );
    assert!(
        text.contains("Ada Lovelace wrote:"),
        "no attribution line:\n{text}"
    );
}

#[test]
fn a_reply_with_nobody_to_send_it_to_says_so_rather_than_offering_to_send_it() {
    // Replying to something you sent yourself. `Draft::reply_to` drops your own address, which
    // is right, and the command then printed "send it with: mailo send <id>" for a draft whose
    // send fails with "cannot build a message with no recipients" — and the CLI has no command
    // that adds one, so the advice could not be followed at all.
    let (store, _dir) = seeded();
    let own = own_message(&store);

    let out = compose::reply(&store, own, ReplyScope::Sender, "to myself", at(10)).unwrap();

    assert!(out.contains("nobody to send this to"), "{out}");
    assert!(
        !out.contains("mailo send"),
        "it still offers a send that cannot work: {out}"
    );
}

#[test]
fn the_attribution_line_is_in_the_senders_zone_not_utc() {
    // This line leaves the machine. It is written into the body of a reply, so a wrong time is
    // wrong in the recipient's mailbox permanently and no later fix reaches it — and it was
    // wrong, because every date in this application was rendered straight off its `DateTime
    // <Utc>`. The original here is 22:13 UTC on Tuesday the 14th, which in Taipei is 06:13 on
    // Wednesday the 15th: a different hour, a different day and a different weekday, so an
    // assertion on it cannot pass by accident.
    let (store, _dir) = seeded();
    let taipei = chrono::FixedOffset::east_opt(8 * 3600).unwrap();
    compose::draft_reply_in(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
        &taipei,
    )
    .unwrap();

    let text = only_draft(&store).text;
    assert!(
        text.contains("On Wed, 15 Nov 2023 at 06:13, Ada Lovelace wrote:"),
        "the attribution line is not in the sender's zone:\n{text}"
    );

    // And the same reply written by someone on UTC says what UTC says, so this is a conversion
    // and not eight hours added somewhere.
    let (store, _dir) = seeded();
    compose::draft_reply_in(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
        &Utc,
    )
    .unwrap();
    assert!(
        only_draft(&store)
            .text
            .contains("On Tue, 14 Nov 2023 at 22:13, Ada Lovelace wrote:"),
        "{}",
        only_draft(&store).text
    );
}

/// A later message in the same conversation, so a thread really has two.
///
/// Carries `in_reply_to` and `References` like a real follow-up, but the `ThreadId` is assigned
/// here rather than derived: the store writes the thread the caller gives it, because JWZ
/// threading happens in `mail_runtime::assemble` on the way in. Minting a fresh id here would
/// silently produce a one-message thread and an assertion that proves nothing — which is what
/// the length check below exists to catch.
fn follow_up(store: &SqliteStore) -> MessageId {
    let id = MessageId::generate();
    let thread = store.message(ORIGINAL).unwrap().thread;
    let raw = store
        .blobs()
        .put(&store.connection(), b"raw reply")
        .unwrap();
    let message = Message {
        id,
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("reply@example.test".to_owned()),
        // Later than the original, which is what makes it the reply target.
        date: at(3600),
        from: Address {
            name: Some("Ada Lovelace".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "Re: lunch on friday".to_owned(),
        in_reply_to: Some("original@example.test".to_owned()),
        references: vec!["original@example.test".to_owned()],
        rfc_message_id: Some("reply@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("one o'clock works".to_owned()),
            raw,
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
                        uidl: "u3".to_owned(),
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

/// A message this account sent, so a reply to it has nowhere to go.
///
/// `From` is the identity's own address and there is no other recipient, which is the shape
/// `Draft::reply_to` deliberately empties: replying to yourself addresses nobody.
fn own_message(store: &SqliteStore) -> MessageId {
    let id = MessageId::generate();
    let raw = store
        .blobs()
        .put(&store.connection(), b"raw to self")
        .unwrap();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc("mine@example.test".to_owned()),
        date: at(7),
        from: Address {
            name: None,
            email: "me@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "a note to myself".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("mine@example.test".to_owned()),
        read: ReadState::Read,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("remember the milk".to_owned()),
            raw,
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
                        uidl: "u-self".to_owned(),
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

/// A second message, headers only — the normal mid-sync state.
fn headers_only(store: &SqliteStore) -> MessageId {
    let id = MessageId::generate();
    let raw = store.blobs().put(&store.connection(), b"raw two").unwrap();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc("second@example.test".to_owned()),
        date: at(5),
        from: Address {
            name: Some("Bob".to_owned()),
            email: "bob@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "no body yet".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("second@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
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
                        uidl: "u2".to_owned(),
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

#[test]
fn a_reply_to_a_message_with_no_body_quotes_nothing_rather_than_the_word_none() {
    // Mid-sync, headers arrive before bodies, and replying then is normal. A client that
    // renders the absent body into the quote sends that text to the recipient.
    let (store, _dir) = seeded();
    let id = headers_only(&store);
    compose::reply(&store, id, ReplyScope::Sender, "later", at(10)).unwrap();

    let text = only_draft(&store).text;
    assert!(text.contains("later"), "{text}");
    assert!(
        text.contains("Bob wrote:"),
        "the attribution still stands:\n{text}"
    );
    assert!(!text.to_lowercase().contains("none"), "{text}");
    assert!(!text.contains('>'), "nothing should be quoted:\n{text}");
}

#[test]
fn sending_queues_the_draft_without_touching_the_network() {
    let (store, _dir) = seeded();
    compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .unwrap();
    let draft = only_draft(&store);

    let out = compose::send(&store, draft.id, at(20)).expect("send queues");
    assert!(out.contains("mailo sync"), "{out}");

    // The submission is in the outbox, with its envelope frozen beside the bytes.
    let due = store.outbox_due(ACCOUNT, at(30)).unwrap();
    assert_eq!(due.len(), 1, "nothing was queued");
    match &due[0].op {
        ProtoOp::Submit {
            draft: queued,
            mail_from,
            rcpt_to,
            ..
        } => {
            assert_eq!(*queued, draft.id);
            assert_eq!(mail_from, "me@example.test");
            assert_eq!(rcpt_to, &vec!["ada@example.test".to_owned()]);
        }
        other => panic!("expected a submission, got {other:?}"),
    }
    assert_eq!(store.draft(draft.id).unwrap().state, SendState::Queued);
}

#[test]
fn the_queued_bytes_are_frozen_against_a_later_edit() {
    // The user pressed send on a particular version. A draft edited afterwards — by an autosave
    // that had not yet fired, or by a second window — must not change what goes out.
    let (store, _dir) = seeded();
    compose::reply(
        &store,
        ORIGINAL,
        ReplyScope::Sender,
        "one o'clock suits",
        at(10),
    )
    .unwrap();
    let mut draft = only_draft(&store);
    compose::send(&store, draft.id, at(20)).unwrap();

    draft.subject = "something else entirely".to_owned();
    draft.updated = at(25);
    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();

    let due = store.outbox_due(ACCOUNT, at(30)).unwrap();
    let ProtoOp::Submit { raw, .. } = &due[0].op else {
        panic!("expected a submission");
    };
    let bytes = store.blobs().get(&store.connection(), *raw).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("Re: lunch on friday"),
        "the frozen bytes changed under the edit:\n{text}"
    );
    assert!(!text.contains("something else entirely"), "{text}");
}

#[test]
fn sending_the_same_draft_twice_is_refused() {
    let (store, _dir) = seeded();
    compose::reply(&store, ORIGINAL, ReplyScope::Sender, "yes", at(10)).unwrap();
    let draft = only_draft(&store);
    compose::send(&store, draft.id, at(20)).unwrap();
    store
        .set_send_state(
            draft.id,
            &SendState::Sent {
                at: at(21),
                message: None,
            },
            at(21),
        )
        .unwrap();

    let err = compose::send(&store, draft.id, at(22)).expect_err("already sent");
    assert!(err.contains("already sent"), "{err}");
}

#[test]
fn an_account_with_no_identity_says_so_instead_of_inventing_a_sender() {
    // An account added before identities were created at setup has no row. Guessing a From
    // address is how mail goes out under an address the user does not own, so replying must
    // stop and say what is wrong.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let plan = mail_domain::presets::preset_for("me@ntu.edu.tw", at(0))
        .unwrap()
        .plan;
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();

    let err = compose::reply(&store, ORIGINAL, ReplyScope::Sender, "hi", at(10))
        .expect_err("no identity and no message");
    // The message does not exist in this bare store either; whichever check fires first, the
    // point is that nothing is invented and nothing panics.
    assert!(!err.is_empty(), "{err}");
}

#[test]
fn the_identity_comes_from_the_table_the_foreign_key_enforces() {
    // `drafts.identity` references `identities(id)`, so any draft that exists has a row. The
    // plan's copy of the same list is written once at account creation and can go stale;
    // reading it instead is how a send fails for an account that is perfectly well configured.
    let (store, _dir) = seeded();
    store
        .connection()
        .execute(
            "UPDATE accounts SET plan = json_set(plan, '$.identities', json('[]')) WHERE id = ?1",
            [ACCOUNT.to_string()],
        )
        .unwrap();

    compose::reply(&store, ORIGINAL, ReplyScope::Sender, "yes", at(10))
        .expect("the identity row is still there");
    let draft = only_draft(&store);
    let out = compose::send(&store, draft.id, at(20)).expect("and sending still works");
    assert!(out.contains("me@example.test"), "{out}");
}

#[test]
fn drafts_reports_what_is_waiting() {
    let (store, _dir) = seeded();
    assert!(compose::drafts(&store).unwrap().contains("no drafts"));

    compose::reply(&store, ORIGINAL, ReplyScope::Sender, "yes", at(10)).unwrap();
    let listed = compose::drafts(&store).unwrap();
    assert!(listed.contains("editing"), "{listed}");
    assert!(listed.contains("Re: lunch on friday"), "{listed}");

    let draft = only_draft(&store);
    compose::send(&store, draft.id, at(20)).unwrap();
    assert!(compose::drafts(&store).unwrap().contains("queued"));
}

/// The composer's round trip, without a window.
///
/// The shell's `Composer` component is thin by construction — every decision it makes lives in
/// `view.rs` and every write goes through `compose.rs`. These drive that pair the way the
/// component does, so the part of the shell that can be wrong is covered even though the
/// widgets are not.
mod composer {
    use super::*;
    use view::Composing;

    #[test]
    fn opening_a_reply_then_editing_and_saving_keeps_everything_unshown() {
        let (store, _dir) = seeded();
        let draft = compose::draft_reply(&store, ORIGINAL, ReplyScope::All, "", at(10)).unwrap();

        // What the composer shows, edited the way a user would.
        let mut editing = Composing::of(&draft);
        assert!(editing.to.contains("ada@example.test"), "{}", editing.to);
        editing.body = format!("one o'clock suits\r\n{}", editing.body);
        editing.subject = "Re: lunch on friday (moved)".to_owned();

        let base = store.draft(editing.draft).unwrap();
        let edited = editing.apply_to(&base, at(11)).unwrap();
        compose::save(&store, &edited).unwrap();

        let reloaded = store.draft(draft.id).unwrap();
        assert_eq!(reloaded.subject, "Re: lunch on friday (moved)");
        assert!(reloaded.text.starts_with("one o'clock suits"));
        // Not shown by the composer, and therefore the things an edit most easily destroys.
        assert_eq!(reloaded.identity, draft.identity);
        assert_eq!(reloaded.in_reply_to, Some(ORIGINAL));
        assert_eq!(
            store.drafts(ACCOUNT).unwrap().len(),
            1,
            "a second draft was made"
        );
    }

    #[test]
    fn a_composed_reply_can_be_sent_and_arrives_in_the_outbox() {
        let (store, _dir) = seeded();
        let draft = compose::draft_reply(&store, ORIGINAL, ReplyScope::Sender, "", at(10)).unwrap();
        let mut editing = Composing::of(&draft);
        editing.body = "yes".to_owned();
        // A recipient added by hand, the way the To box is actually used.
        editing.cc = "Bea <bea@example.test>".to_owned();

        let base = store.draft(editing.draft).unwrap();
        let edited = editing.apply_to(&base, at(11)).unwrap();
        compose::save(&store, &edited).unwrap();
        compose::send(&store, edited.id, at(12)).unwrap();

        let due = store.outbox_due(ACCOUNT, at(20)).unwrap();
        assert_eq!(due.len(), 1);
        let ProtoOp::Submit { rcpt_to, .. } = &due[0].op else {
            panic!("expected a submission");
        };
        let mut rcpt = rcpt_to.clone();
        rcpt.sort();
        assert_eq!(
            rcpt,
            vec!["ada@example.test".to_owned(), "bea@example.test".to_owned()],
            "the hand-typed Cc did not reach the envelope"
        );
    }

    #[test]
    fn a_mistyped_recipient_stops_the_save_instead_of_dropping_them() {
        let (store, _dir) = seeded();
        let draft = compose::draft_reply(&store, ORIGINAL, ReplyScope::Sender, "", at(10)).unwrap();
        let mut editing = Composing::of(&draft);
        editing.to = "ada@example.test, bea".to_owned();

        let base = store.draft(editing.draft).unwrap();
        let err = editing
            .apply_to(&base, at(11))
            .expect_err("\"bea\" is not an address");
        assert!(err.starts_with("To:"), "{err}");

        // And the stored draft is untouched, so nothing was half-written.
        assert_eq!(store.draft(draft.id).unwrap().to, draft.to);
    }

    #[test]
    fn the_reply_button_answers_the_newest_message_in_the_thread() {
        let (store, _dir) = seeded();
        let later = follow_up(&store);
        let thread = store.message(later).unwrap().thread;

        let messages: Vec<Message> = store
            .thread(thread)
            .unwrap()
            .messages
            .iter()
            .filter_map(|id| store.message(*id).ok())
            .collect();
        assert!(
            messages.len() >= 2,
            "this asserts nothing unless the thread really has two messages: {}",
            messages.len()
        );
        let target = view::reply_target(&messages).unwrap();
        assert_eq!(
            target.id, later,
            "replied to the wrong message in the thread"
        );
    }
}

/// Closing the composer must not cost the user what they typed.
mod closing {
    use super::*;
    use view::Composing;

    /// What the Close button does: save, and only then drop the widgets.
    fn close(store: &SqliteStore, editing: &Composing, now: DateTime<Utc>) -> Result<(), String> {
        let base = store.draft(editing.draft).map_err(|e| e.to_string())?;
        let edited = editing.apply_to(&base, now)?;
        compose::save(store, &edited)
    }

    #[test]
    fn closing_saves_what_was_typed() {
        // The bug: Close called close_composer directly and threw away everything since the
        // last Save, behind a comment saying that could not happen.
        let (store, _dir) = seeded();
        let draft = compose::draft_reply(&store, ORIGINAL, ReplyScope::Sender, "", at(10)).unwrap();
        let mut editing = Composing::of(&draft);
        editing.body = "a paragraph nobody clicked Save on".to_owned();

        close(&store, &editing, at(11)).expect("closing saves");
        assert!(
            store
                .draft(draft.id)
                .unwrap()
                .text
                .contains("a paragraph nobody clicked Save on"),
            "closing the composer lost the text"
        );
    }

    #[test]
    fn a_recipient_that_does_not_parse_refuses_the_close_rather_than_the_text() {
        // Staying open is the point: a typo in the To box must not cost the paragraph.
        let (store, _dir) = seeded();
        let draft = compose::draft_reply(&store, ORIGINAL, ReplyScope::Sender, "", at(10)).unwrap();
        let mut editing = Composing::of(&draft);
        editing.body = "worth keeping".to_owned();
        editing.to = "nonsense".to_owned();

        assert!(close(&store, &editing, at(11)).is_err());
        // Nothing was written, and the composer stays open holding the text.
        assert_eq!(store.draft(draft.id).unwrap().text, draft.text);
        assert_eq!(editing.body, "worth keeping");
    }
}

/// Discarding a draft, which nothing in the application could do.
///
/// The composer's "Discard" closed the pane without saving. For a draft that had never been
/// saved that is the same thing; for every other draft it is not, and `Composing`'s own doc
/// comment says which case is real: "the draft this edits — it already exists in the store
/// before the composer opens". A reply is written to the store the moment it is created and a
/// draft opened from the drafts list came off disk, so the discarded draft was still in Drafts
/// afterwards, for ever. Drafts could be made and never unmade.
mod discarding {
    use super::*;

    fn a_draft(store: &SqliteStore) -> DraftId {
        compose::draft_reply(store, ORIGINAL, ReplyScope::Sender, "never mind", at(10))
            .unwrap()
            .id
    }

    #[test]
    fn a_discarded_draft_is_gone_from_the_store() {
        let (store, _dir) = seeded();
        let id = a_draft(&store);
        assert_eq!(store.drafts(ACCOUNT).unwrap().len(), 1, "it was saved");

        let subject = compose::discard(&store, id).unwrap();

        assert_eq!(subject, "Re: lunch on friday", "it names what went");
        assert!(
            store.drafts(ACCOUNT).unwrap().is_empty(),
            "the draft outlived being discarded"
        );
        assert!(store.draft(id).is_err(), "and cannot be fetched by id");
    }

    #[test]
    fn a_draft_that_is_already_on_its_way_is_not_deleted_underneath_the_outbox() {
        // `Queued` means the next sync will pick it up, and `Sending` may already be on the
        // wire. Deleting either leaves the outbox draining something that is not there.
        for state in [SendState::Queued, SendState::Sending] {
            let (store, _dir) = seeded();
            let id = a_draft(&store);
            let mut draft = store.draft(id).unwrap();
            draft.state = state.clone();
            compose::save(&store, &draft).unwrap();

            let why = compose::discard(&store, id).unwrap_err();
            assert!(why.contains("queued for delivery"), "{why}");
            assert_eq!(
                store.drafts(ACCOUNT).unwrap().len(),
                1,
                "it was deleted anyway from {state:?}"
            );
        }
    }

    #[test]
    fn a_draft_that_was_already_sent_can_still_be_cleared_away() {
        // A `Sent` draft is a record of something that happened, not work in progress, and the
        // drafts list is the only place it shows up. Refusing to remove it would make the list
        // unclearable.
        let (store, _dir) = seeded();
        let id = a_draft(&store);
        let mut draft = store.draft(id).unwrap();
        draft.state = SendState::Sent {
            at: at(20),
            message: None,
        };
        compose::save(&store, &draft).unwrap();

        compose::discard(&store, id).unwrap();
        assert!(store.drafts(ACCOUNT).unwrap().is_empty());
    }

    #[test]
    fn discarding_something_that_is_not_there_is_an_error_not_a_panic() {
        let (store, _dir) = seeded();
        assert!(compose::discard(&store, DraftId::generate()).is_err());
    }
}

/// Forwarding, which the domain modelled and nothing could reach.
///
/// `Draft::forward_of` has existed and been tested in `mail-domain` since phase 1, and no surface
/// in the application called it. A mail client that cannot forward is not one.
mod forwarding {
    use super::*;

    fn taipei() -> chrono::FixedOffset {
        chrono::FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn to() -> Vec<Address> {
        vec![Address {
            name: Some("Bea".to_owned()),
            email: "bea@example.test".to_owned(),
        }]
    }

    #[test]
    fn a_forward_carries_the_original_beneath_a_header_block() {
        let (store, _dir) = seeded();
        compose::draft_forward_in(
            &store,
            ORIGINAL,
            &to(),
            "thought you should see this",
            at(10),
            &taipei(),
        )
        .unwrap();

        let draft = only_draft(&store);
        assert_eq!(draft.subject, "Fwd: lunch on friday");
        assert_eq!(draft.to, to(), "a forward goes where it is told");
        assert_eq!(draft.forward_of, Some(ORIGINAL));
        assert_eq!(draft.in_reply_to, None, "a forward answers nothing");

        let text = &draft.text;
        // What was written, then the block, then the message — in that order.
        let note = text.find("thought you should see this").expect("the note");
        let block = text
            .find("---------- Forwarded message ----------")
            .expect("block");
        let body = text
            .find("Shall we say one o'clock?")
            .expect("the original text");
        assert!(note < block && block < body, "out of order:\n{text}");

        // The headers a recipient needs to judge it, in the sender's zone: the original is
        // 22:13 on Tuesday the 14th in UTC and 06:13 on Wednesday the 15th in Taipei.
        assert!(
            text.contains("From: Ada Lovelace <ada@example.test>"),
            "{text}"
        );
        assert!(text.contains("Date: Wed, 15 Nov 2023 at 06:13"), "{text}");
        assert!(text.contains("Subject: lunch on friday"), "{text}");
        assert!(
            text.contains("To: me@example.test, bea@example.test"),
            "{text}"
        );
        assert!(text.contains("Cc: cara@example.test"), "{text}");
    }

    #[test]
    fn what_the_command_prints_names_the_draft_and_how_to_send_it() {
        // The CLI-facing half, which is also the one that decides the zone: `forward` defaults
        // to `Local` where `draft_forward_in` is told. Asserted on the text a person reads.
        let (store, _dir) = seeded();
        let out = compose::forward(&store, ORIGINAL, &to(), "fyi", at(10)).unwrap();

        assert!(out.contains("Fwd: lunch on friday"), "{out}");
        assert!(out.contains("Bea <bea@example.test>"), "{out}");
        let draft = only_draft(&store);
        assert!(
            out.contains(&format!("mailo send {}", draft.id)),
            "it does not say how to send it: {out}"
        );
        // No double-space check here, unlike the prose messages of F97: this output is an
        // aligned table — `  to      …` — and the runs of spaces are the alignment.
        assert!(
            out.contains("  to      "),
            "the columns are not aligned: {out:?}"
        );
    }

    #[test]
    fn the_original_is_carried_rather_than_quoted() {
        // A forward passes the message on; it does not answer it. Every client writes it
        // unmarked beneath a header block, and a recipient who sees "> " reads it as a reply.
        let (store, _dir) = seeded();
        compose::draft_forward_in(&store, ORIGINAL, &to(), "", at(10), &taipei()).unwrap();
        let text = only_draft(&store).text;
        assert!(
            !text.contains("> Shall we say one o'clock?"),
            "the original was quoted like a reply:\n{text}"
        );
        assert!(text.contains("Shall we say one o'clock?"), "{text}");
    }

    #[test]
    fn a_forward_with_no_body_yet_still_carries_the_message() {
        // What the shell's Forward button produces: a draft with no note and no recipients, for
        // the user to fill in. The message must already be in it, or there is nothing to send.
        let (store, _dir) = seeded();
        let draft =
            compose::draft_forward_in(&store, ORIGINAL, &[], "", at(10), &taipei()).unwrap();
        assert!(draft.to.is_empty());
        assert!(
            draft
                .text
                .contains("---------- Forwarded message ----------")
        );
        assert!(draft.text.contains("Shall we say one o'clock?"));
    }

    #[test]
    fn a_message_whose_body_never_arrived_forwards_its_headers_and_nothing_else() {
        // Mid-sync. Quoting the word "None" is what this avoids — the same case `quoted` has.
        let (store, _dir) = seeded();
        let id = headers_only(&store);
        let draft = compose::draft_forward_in(&store, id, &to(), "", at(10), &taipei()).unwrap();
        assert!(
            draft
                .text
                .contains("---------- Forwarded message ----------")
        );
        assert!(!draft.text.contains("None"), "{}", draft.text);
    }
}

/// Signatures, which were a column nothing read.
///
/// `Identity.signature` has existed since phase 1. No sending path appended one, no composer
/// showed one, and no command could set one — every message this client had ever sent went out
/// unsigned.
mod signatures {
    use super::*;

    fn set(store: &SqliteStore, text: Option<&str>) {
        compose::set_signature(store, ACCOUNT, text).unwrap();
    }

    #[test]
    fn a_reply_carries_the_signature_beneath_what_was_written() {
        let (store, _dir) = seeded();
        set(&store, Some("Ada Lovelace\nAnalytical Engines Ltd"));

        compose::reply(
            &store,
            ORIGINAL,
            ReplyScope::Sender,
            "one o'clock suits",
            at(10),
        )
        .unwrap();
        let text = only_draft(&store).text;

        let written = text.find("one o'clock suits").expect("what was written");
        let delimiter = text.find("\r\n-- \r\n").expect("the signature delimiter");
        let sig = text.find("Analytical Engines").expect("the signature");
        let quote = text.find("Ada Lovelace wrote:").expect("the attribution");
        assert!(written < delimiter, "the signature came first:\n{text}");
        assert!(delimiter < sig && sig < quote, "out of order:\n{text}");
    }

    #[test]
    fn the_delimiter_is_dash_dash_space_exactly() {
        // RFC 3676 §4.3 names that string, and every client that trims a signature when quoting
        // looks for it. `--` without the trailing space is a different line and gets quoted back
        // at people for the rest of the thread.
        let (store, _dir) = seeded();
        set(&store, Some("Ada"));
        compose::reply(&store, ORIGINAL, ReplyScope::Sender, "hi", at(10)).unwrap();

        let text = only_draft(&store).text;
        assert!(text.contains("\r\n-- \r\n"), "{text:?}");
        assert!(
            !text.contains("\r\n--\r\n"),
            "the trailing space is missing: {text:?}"
        );
    }

    #[test]
    fn a_forward_is_signed_too() {
        let (store, _dir) = seeded();
        set(&store, Some("Ada"));
        compose::draft_forward(
            &store,
            ORIGINAL,
            &[Address {
                name: None,
                email: "bea@example.test".to_owned(),
            }],
            "passing this on",
            at(10),
        )
        .unwrap();

        let text = only_draft(&store).text;
        let note = text.find("passing this on").unwrap();
        let delimiter = text.find("\r\n-- \r\n").unwrap();
        let block = text.find("---------- Forwarded message").unwrap();
        assert!(
            note < delimiter && delimiter < block,
            "out of order:\n{text}"
        );
    }

    #[test]
    fn no_signature_means_no_delimiter() {
        // An account that has not set one must not gain a bare `-- ` line, which other clients
        // read as "everything below is a signature" and hide.
        let (store, _dir) = seeded();
        compose::reply(&store, ORIGINAL, ReplyScope::Sender, "hi", at(10)).unwrap();
        assert!(
            !only_draft(&store).text.contains("-- "),
            "{}",
            only_draft(&store).text
        );
    }

    #[test]
    fn whitespace_is_not_a_signature() {
        // `mailo signature you@x < /dev/null` and a file of blank lines are both "take it off".
        let (store, _dir) = seeded();
        set(&store, Some("   \n\n  "));
        compose::reply(&store, ORIGINAL, ReplyScope::Sender, "hi", at(10)).unwrap();
        assert!(
            !only_draft(&store).text.contains("-- "),
            "{}",
            only_draft(&store).text
        );
    }

    #[test]
    fn clearing_it_takes_it_off_the_next_reply() {
        let (store, _dir) = seeded();
        set(&store, Some("Ada"));
        let out = compose::set_signature(&store, ACCOUNT, None).unwrap();
        assert!(out.contains("cleared"), "{out}");

        compose::reply(&store, ORIGINAL, ReplyScope::Sender, "hi", at(10)).unwrap();
        assert!(!only_draft(&store).text.contains("-- "));
    }
}

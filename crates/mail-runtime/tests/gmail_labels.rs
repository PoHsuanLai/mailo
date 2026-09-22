//! The seam between a Gmail survey and a search: labels off the wire, into SQLite, found again.
//!
//! `mail-proto`'s tests prove the parse against real capture shapes and `mail-store`'s prove the
//! rows; neither can prove they meet, because the boundary forbids `mail-proto` from knowing
//! what a store is. This is the only crate that may hold both.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{Backend, ImapAuth, ImapCommand, ImapSession, IoReady, Progress, ProtoOutcome};
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn gmail_caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::Supported,
        threads: ServerThreads::ProviderId,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::DropInbox,
        folders: FolderRoles::default(),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 5 },
        observed_at: now(),
    }
}

fn backend() -> ImapBackend {
    ImapBackend::new(
        ACCOUNT,
        gmail_caps(),
        Box::new(|auth: Authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.push(ImapCommand::AuthenticateXoauth2);
            }
            all.extend(commands);
            ImapSession::new(
                ImapAuth {
                    username: "ada@example.test".to_owned(),
                    credential: Credential::OAuth {
                        access: "ya29.token".to_owned(),
                        refresh: "1//refresh".to_owned(),
                        expires_at: Utc.timestamp_opt(2_000_000_000, 0).unwrap(),
                    },
                    sasl: vec![SaslMech::XOauth2],
                },
                all,
            )
        }),
    )
}

/// Run a survey against canned bytes and take the `Ingest` it produces.
///
/// No socket: the backend is sans-I/O, so "the server" is a slice of bytes handed back whenever
/// it asks to read. That is the whole point of the `Machine` shape.
fn survey(responses: &[&str]) -> Ingest {
    let mut backend = backend();
    let mut replies = responses.iter();
    let mut progress = backend.begin(ProtoOp::FetchEnvelopes {
        mailbox: MailboxRef {
            account: ACCOUNT,
            path: "INBOX".to_owned(),
        },
        since: FetchSince::Beginning,
    });
    loop {
        match progress {
            // Whatever it asked for, the answer is the next slice of the transcript: a write
            // needs no acknowledgement in this shape, and every read is the server's next turn.
            Progress::Need(_) => {
                let next = replies.next().expect("the transcript ran out");
                progress = backend.feed(IoReady::Bytes(next.as_bytes().to_vec()));
            }
            Progress::Done(ProtoOutcome::Ingested(ingest)) => return *ingest,
            other => panic!("{other:?}"),
        }
    }
}

fn store() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
    (store, dir)
}

#[test]
fn a_label_on_the_wire_becomes_a_label_you_can_search_for() {
    let (store, _dir) = store();

    // A message already held, as the header pass would have left it.
    let raw = store.blobs().put(&store.connection(), b"raw").unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc("m1@example.test".to_owned()),
        date: now(),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "the trip".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    let thread = message.thread;
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
                        uidvalidity: 1,
                        uid: 42,
                    },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();

    // Now the survey, speaking Gmail.
    let ingest = survey(&[
        "* OK Gimap ready\r\n",
        "a001 OK authenticated\r\n",
        "* OK [UIDVALIDITY 1] UIDs valid.\r\n* OK [UIDNEXT 43] next\r\na002 OK [READ-ONLY] done\r\n",
        "* 1 FETCH (UID 42 FLAGS (\\Seen) RFC822.SIZE 100 X-GM-LABELS (\\Inbox \"travel\"))\r\n\
         a003 OK Success\r\n",
    ]);
    assert_eq!(
        ingest.label_names.len(),
        1,
        "the survey carried no labels: {ingest:?}"
    );

    store.ingest(ACCOUNT, ingest).unwrap();

    // And it is findable by the thing the user typed.
    let travel = store
        .labels(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|l| l.name == "travel")
        .expect("the label reached the store");
    let found = store
        .threads(
            &Query {
                filter: Filter::HasLabel(travel.id),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 10,
                },
            },
            now(),
        )
        .unwrap();
    assert_eq!(found.items.len(), 1, "not findable by its label");
    assert_eq!(found.items[0].id, thread);

    // And `\Inbox` did not become one.
    assert_eq!(
        store.labels(ACCOUNT).unwrap().len(),
        1,
        "a system name became a label: {:?}",
        store.labels(ACCOUNT).unwrap()
    );
}

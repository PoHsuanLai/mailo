//! `plan.md` phase 3: "a CLI or unit test can ingest 20 fixture messages, archive one, label
//! one, search."
//!
//! Written as the plan's own acceptance criterion rather than as unit tests of the pieces,
//! because the pieces have passed individually for a while and that is not the same claim.
//! Everything here goes through the public `Store` surface a UI would use.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const WORK: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n * 3600, 0).unwrap()
}

fn now() -> DateTime<Utc> {
    at(1000)
}

struct Fixture {
    store: SqliteStore,
    threads: Vec<ThreadId>,
    messages: Vec<MessageId>,
    _dir: tempfile::TempDir,
}

/// Twenty messages across twenty threads, with searchable variety.
fn ingest_twenty() -> Fixture {
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

    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(Label {
                    id: WORK,
                    account: ACCOUNT,
                    name: "work".to_owned(),
                    color: None,
                    origin: LabelOrigin::User,
                })],
            },
        )
        .unwrap();

    let subjects = [
        "lunch on friday",
        "invoice 2024",
        "quarterly report",
        "café meeting",
        "Résumé for review",
        "standup notes",
        "travel booking",
        "server outage",
        "welcome aboard",
        "password reset",
        "lunch tomorrow",
        "invoice 2025",
        "design review",
        "holiday plans",
        "book club",
        "rent reminder",
        "conference talk",
        "photo dump",
        "weekly digest",
        "final notice",
    ];

    let mut threads = Vec::new();
    let mut messages = Vec::new();
    for (i, subject) in subjects.iter().enumerate() {
        let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7000 + i as u128));
        let id = MessageId::from_uuid(uuid::Uuid::from_u128(0x9000 + i as u128));
        let raw = store
            .blobs()
            .put(&store.connection(), format!("raw message {i}").as_bytes())
            .unwrap();
        let message = Message {
            id,
            thread,
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("m{i}@example.test")),
            date: at(i as i64),
            from: Address {
                name: Some(format!("Sender {i}")),
                email: format!("s{i}@example.test"),
            },
            reply_to: vec![],
            to: vec![Address {
                name: None,
                email: "me@example.test".into(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: (*subject).to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("m{i}@example.test")),
            read: if i % 3 == 0 {
                ReadState::Read
            } else {
                ReadState::Unread
            },
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some(format!("body of {subject}, message number {i}")),
                raw,
            },
            attachments: vec![],
        };
        store
            .apply(
                ACCOUNT,
                &Patch {
                    id: ChangeId::generate(),
                    changes: vec![Change::MessageUpsert(Box::new(message))],
                },
            )
            .unwrap();
        threads.push(thread);
        messages.push(id);
    }
    Fixture {
        store,
        threads,
        messages,
        _dir: dir,
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::Supported,
        threads: ServerThreads::Jwz,
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

fn query(filter: Filter) -> Query {
    Query {
        filter,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 100,
        },
    }
}

#[test]
fn ingest_twenty_archive_one_label_one_and_search() {
    let f = ingest_twenty();

    // --- twenty threads, all in the inbox --------------------------------------------
    let inbox = f
        .store
        .threads(&query(Filter::InMailbox(MailboxRole::Inbox)), now())
        .unwrap();
    assert_eq!(inbox.items.len(), 20, "all twenty should be in the inbox");
    assert_eq!(
        f.store
            .count(&Filter::InMailbox(MailboxRole::Inbox), now())
            .unwrap(),
        20,
        "count must agree with the rows it counts"
    );

    // --- archive one ------------------------------------------------------------------
    let target = f.threads[3];
    let thread = f.store.thread(target).unwrap();
    let messages: Vec<Message> = thread
        .messages
        .iter()
        .map(|id| f.store.message(*id).unwrap())
        .collect();
    let applied = Op::Archive.apply(
        &Target::Threads(vec![target]),
        &thread,
        &messages,
        &caps(),
        now(),
    );
    f.store.apply(ACCOUNT, &applied.forward).unwrap();

    assert_eq!(
        f.store
            .count(&Filter::InMailbox(MailboxRole::Inbox), now())
            .unwrap(),
        19,
        "the archived thread must leave the inbox"
    );
    assert_eq!(
        f.store
            .count(&Filter::InMailbox(MailboxRole::Archive), now())
            .unwrap(),
        1
    );

    // The undo the op computed must actually undo it — the property the proptest checks
    // abstractly, here against real storage.
    f.store.apply(ACCOUNT, &applied.inverse).unwrap();
    assert_eq!(
        f.store
            .count(&Filter::InMailbox(MailboxRole::Inbox), now())
            .unwrap(),
        20,
        "applying the inverse must restore the inbox exactly"
    );
    f.store.apply(ACCOUNT, &applied.forward).unwrap();

    // --- label one --------------------------------------------------------------------
    let labelled = f.threads[7];
    let thread = f.store.thread(labelled).unwrap();
    let messages: Vec<Message> = thread
        .messages
        .iter()
        .map(|id| f.store.message(*id).unwrap())
        .collect();
    let applied = Op::Label(WORK, Membership::In).apply(
        &Target::Threads(vec![labelled]),
        &thread,
        &messages,
        &caps(),
        now(),
    );
    f.store.apply(ACCOUNT, &applied.forward).unwrap();

    let tagged = f
        .store
        .threads(&query(Filter::HasLabel(WORK)), now())
        .unwrap();
    assert_eq!(tagged.items.len(), 1);
    assert_eq!(tagged.items[0].id, labelled);

    // Under ServerLabels::Supported this is real remote work, not a local-only change.
    assert!(
        applied.remote.is_some(),
        "a server-side label change must be queued for the server"
    );

    // --- search ------------------------------------------------------------------------
    // Substring, through LIKE: matches inside a word.
    let by_subject = f
        .store
        .threads(
            &query(Filter::Subject(TextMatch::Contains("invoice".into()))),
            now(),
        )
        .unwrap();
    assert_eq!(by_subject.items.len(), 2, "invoice 2024 and invoice 2025");

    // Full text, through FTS5: whole words, and it folds diacritics where LIKE does not.
    let folded = f
        .store
        .threads(
            &query(Filter::Text(TextMatch::Contains("resume".into()))),
            now(),
        )
        .unwrap();
    assert_eq!(
        folded.items.len(),
        1,
        "an unaccented needle must find 'Résumé' through FTS5"
    );
    let unfolded = f
        .store
        .threads(
            &query(Filter::Subject(TextMatch::Contains("resume".into()))),
            now(),
        )
        .unwrap();
    assert_eq!(
        unfolded.items.len(),
        0,
        "the same needle must NOT match through LIKE, which is ASCII-only"
    );

    // Body text is searchable, and only through the full-text path.
    let by_body = f
        .store
        .threads(
            &query(Filter::Text(TextMatch::Contains("standup".into()))),
            now(),
        )
        .unwrap();
    assert_eq!(by_body.items.len(), 1);

    // Composition: unread AND in the inbox.
    let unread_inbox = f
        .store
        .count(
            &Filter::And(vec![
                Filter::InMailbox(MailboxRole::Inbox),
                Filter::Read(ReadState::Unread),
            ]),
            now(),
        )
        .unwrap();
    assert!(unread_inbox > 0 && unread_inbox < 20, "got {unread_inbox}");

    // --- pagination --------------------------------------------------------------------
    let mut page = Query {
        filter: Filter::All,
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 7,
        },
    };
    let mut seen = Vec::new();
    loop {
        let got = f.store.threads(&page, now()).unwrap();
        seen.extend(got.items.iter().map(|s| s.id));
        match got.next {
            Some(cursor) => page.page.after = Some(cursor),
            None => break,
        }
    }
    assert_eq!(
        seen.len(),
        20,
        "keyset pagination must visit every row once"
    );
    let unique: std::collections::BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 20, "and must not repeat one");

    assert_eq!(f.messages.len(), 20);
}

//! Twenty messages ingested, one archived and its undo applied, one labelled, then searched and
//! paged through.
//!
//! One path end to end rather than unit tests of the pieces, because the pieces passing one at a
//! time is not the same claim. Everything here goes through the public `Store` surface a UI
//! would use.

use crate::support::*;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const WORK: LabelId = LabelId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));

fn now() -> chrono::DateTime<chrono::Utc> {
    at(3_600_000)
}

/// Twenty messages across twenty threads, an hour apart, with searchable variety; a third of
/// them read.
fn ingest_twenty() -> (SqliteStore, Vec<ThreadId>) {
    let store = sqlite();
    let work = Label {
        id: WORK,
        account: acct_account(),
        name: "work".to_owned(),
        color: None,
        origin: LabelOrigin::User,
    };
    store
        .apply(acct_account(), &patch(vec![Change::LabelUpsert(work)]))
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
    for (i, subject) in subjects.iter().enumerate() {
        let raw = store
            .blobs()
            .put(format!("raw message {i}").as_bytes())
            .unwrap();
        let message = Message {
            date: at(i as i64 * 3600),
            subject: (*subject).to_owned(),
            read: if i % 3 == 0 {
                ReadState::Read
            } else {
                ReadState::Unread
            },
            body: Body::Present {
                text: Some(format!("body of {subject}, message number {i}")),
                raw,
            },
            ..message(0x9000 + i as u128)
        };
        threads.push(message.thread);
        store
            .apply(
                acct_account(),
                &patch(vec![Change::MessageUpsert(Box::new(message))]),
            )
            .unwrap();
    }
    (store, threads)
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

/// `op` applied to the whole of `thread`, as the window applies it.
fn apply_to(store: &SqliteStore, op: Op, thread: ThreadId) -> Applied {
    let held = store.thread(thread).unwrap();
    let messages: Vec<Message> = held
        .messages
        .iter()
        .map(|id| store.message(*id).unwrap())
        .collect();
    op.apply(
        &Target::Threads(vec![thread]),
        &held,
        &messages,
        &caps(),
        now(),
    )
}

#[test]
fn twenty_messages_are_archived_labelled_searched_and_paged() {
    let (store, threads) = ingest_twenty();
    let count = |filter: Filter| store.count(&filter, now()).unwrap();
    let listed = |filter: Filter| -> Vec<ThreadId> {
        let page = store.threads(&query(filter, 100), now()).unwrap();
        page.items.into_iter().map(|s| s.id).collect()
    };
    let inbox = Filter::InMailbox(MailboxRole::Inbox);
    assert_eq!(
        listed(inbox.clone()).len(),
        20,
        "all twenty are in the inbox"
    );
    assert_eq!(
        count(inbox.clone()),
        20,
        "count agrees with the rows it counts"
    );

    // Archive one, and the undo the op computed undoes it against real storage.
    let archived = apply_to(&store, Op::Archive, threads[3]);
    store.apply(acct_account(), &archived.forward).unwrap();
    assert_eq!(
        count(inbox.clone()),
        19,
        "the archived thread leaves the inbox"
    );
    assert_eq!(count(Filter::InMailbox(MailboxRole::Archive)), 1);
    store.apply(acct_account(), &archived.inverse).unwrap();
    assert_eq!(
        count(inbox.clone()),
        20,
        "the inverse restores the inbox exactly"
    );
    store.apply(acct_account(), &archived.forward).unwrap();

    // Label one. Under ServerLabels::Supported this is remote work, not a local-only change.
    let labelled = apply_to(&store, Op::Label(WORK, Membership::In), threads[7]);
    store.apply(acct_account(), &labelled.forward).unwrap();
    assert_eq!(listed(Filter::HasLabel(WORK)), [threads[7]]);
    assert!(labelled.remote.is_some(), "a server label change is queued");

    // Subject goes through LIKE: a substring inside a word, and ASCII-only. Text goes through
    // FTS5: whole words, diacritics folded, and bodies included.
    let subject = |s: &str| Filter::Subject(TextMatch::Contains(s.into()));
    let text = |s: &str| Filter::Text(TextMatch::Contains(s.into()));
    assert_eq!(listed(subject("invoice")).len(), 2, "invoice 2024 and 2025");
    assert_eq!(listed(text("resume")).len(), 1, "'Résumé' through FTS5");
    assert_eq!(listed(subject("resume")).len(), 0, "but not through LIKE");
    assert_eq!(listed(text("standup")).len(), 1);

    // Composition, checked against the other path to the same question: a range would be met
    // by almost any wrong answer, and `count` and `threads` agreeing cannot be by accident.
    let unread_inbox = Filter::And(vec![inbox, Filter::Read(ReadState::Unread)]);
    let counted = count(unread_inbox.clone());
    assert!(counted > 0, "the fixture has unread inbox mail");
    assert_eq!(counted as usize, listed(unread_inbox).len());

    // Keyset pagination visits every row once.
    let mut page = query(Filter::All, 7);
    let mut seen = Vec::new();
    loop {
        let got = store.threads(&page, now()).unwrap();
        seen.extend(got.items.iter().map(|s| s.id));
        match got.next {
            Some(cursor) => page.page.after = Some(cursor),
            None => break,
        }
    }
    assert_eq!(seen.len(), 20);
    let unique: std::collections::BTreeSet<_> = seen.iter().collect();
    assert_eq!(unique.len(), 20, "and repeats none");
}

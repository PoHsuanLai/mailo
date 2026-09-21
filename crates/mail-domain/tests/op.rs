//! `Op::apply`, `Op::kind`, and the thread rollup they rebuild.
//!
//! The load-bearing property is that `Applied::inverse` restores the state that existed
//! *before* the op, rather than being a mechanical mirror of the op. Every hand-written case
//! below is a witness for one way those two differ; `inverse_restores_the_prior_state` is the
//! general statement.

use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountId, ArchiveMeans, Attachments, BlobId, Body, Change, Condstore,
    FolderRoles, LabelId, MailboxRole, MailboxSet, Membership, Message, MessageId, MessageKey,
    MoveExt, Op, OpKind, Patch, Pin, ReadState, RemoteIntent, ServerLabels, ServerThreads, Snooze,
    Star, Target, Thread, ThreadId, ThreadSummary, WatchMode,
};
use mail_domain::{Address, Attachment, Inline};
use proptest::prelude::*;
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------
// Fixtures. Every instant is fixed: CONVENTIONS.md section 6 forbids `Utc::now()` in tests
// too, so that a failure reproduces.
// ---------------------------------------------------------------------------------------

const fn tid(n: u128) -> ThreadId {
    ThreadId::from_uuid(Uuid::from_u128(n))
}
const fn mid(n: u128) -> MessageId {
    MessageId::from_uuid(Uuid::from_u128(n))
}
const fn lid(n: u128) -> LabelId {
    LabelId::from_uuid(Uuid::from_u128(n))
}

const THREAD: ThreadId = tid(0x7001);
const OTHER_THREAD: ThreadId = tid(0x7002);
const ACCOUNT: AccountId = AccountId::from_uuid(Uuid::from_u128(0xacc0));
const LABEL_A: LabelId = lid(0xa1);
const LABEL_B: LabelId = lid(0xb2);
const LABEL_C: LabelId = lid(0xc3);

/// A fixed instant, `secs` seconds after the epoch.
fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("a small positive timestamp is always in range")
}

/// The `now` handed to every `apply` call. Nothing in the domain reads it today.
fn now() -> DateTime<Utc> {
    at(1_000_000)
}

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

/// A message with every flag at its quietest setting; tests mutate the fields they care about.
fn message(n: u128, date: i64) -> Message {
    Message {
        id: mid(n),
        thread: THREAD,
        account: ACCOUNT,
        key: MessageKey::Rfc(format!("m{n}@example.test")),
        date: at(date),
        from: addr(None, "ada@example.test"),
        reply_to: Vec::new(),
        to: vec![addr(None, "bob@example.test")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: format!("Subject {n}"),
        in_reply_to: None,
        references: Vec::new(),
        rfc_message_id: Some(format!("m{n}@example.test")),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: Vec::new(),
        body: Body::Present {
            text: None,
            raw: BlobId::from_uuid(Uuid::from_u128(0xb000 + n)),
        },
        attachments: Vec::new(),
    }
}

fn attachment(name: &str) -> Attachment {
    Attachment {
        name: name.to_owned(),
        mime: "text/plain".to_owned(),
        size: 3,
        blob: BlobId::from_uuid(Uuid::from_u128(0xf00d)),
        inline: Inline::Attached,
    }
}

fn thread_of(messages: &[Message], snooze: Snooze, pin: Pin) -> Thread {
    Thread {
        summary: ThreadSummary::derive(THREAD, messages, snooze, pin),
        messages: messages.iter().map(|m| m.id).collect(),
    }
}

fn account_caps(archive: ArchiveMeans, labels: ServerLabels) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: Duration::from_secs(300),
        },
        archive,
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        observed_at: at(0),
    }
}

fn server_caps() -> AccountCaps {
    account_caps(ArchiveMeans::DropInbox, ServerLabels::Supported)
}

/// The thread every hand-written `apply` case runs against: three messages that disagree about
/// everything an op can touch.
///
/// | message | date | mailbox | read   | star      | labels |
/// |---------|------|---------|--------|-----------|--------|
/// | m1      | 100  | Inbox   | Unread | Unstarred | A      |
/// | m2      | 200  | Archive | Read   | Starred   | -      |
/// | m3      | 300  | Trash   | Unread | Unstarred | A, B   |
fn mixed_thread() -> Vec<Message> {
    let mut m1 = message(1, 100);
    m1.labels = vec![LABEL_A];

    let mut m2 = message(2, 200);
    m2.mailbox = MailboxRole::Archive;
    m2.read = ReadState::Read;
    m2.star = Star::Starred;

    let mut m3 = message(3, 300);
    m3.mailbox = MailboxRole::Trash;
    m3.labels = vec![LABEL_A, LABEL_B];

    vec![m1, m2, m3]
}

// ---------------------------------------------------------------------------------------
// Op::kind
// ---------------------------------------------------------------------------------------

#[test]
fn kind_strips_the_payload() {
    // `SetRead`, `SetStar` and `Label` each map to two kinds, chosen by their payload.
    const CASES: &[(Op, OpKind)] = &[
        (Op::Archive, OpKind::Archive),
        (Op::Trash, OpKind::Trash),
        (Op::Restore, OpKind::Restore),
        (Op::Spam, OpKind::Spam),
        (Op::SetRead(ReadState::Read), OpKind::MarkRead),
        (Op::SetRead(ReadState::Unread), OpKind::MarkUnread),
        (Op::SetStar(Star::Starred), OpKind::Star),
        (Op::SetStar(Star::Unstarred), OpKind::Unstar),
        (Op::Label(LABEL_A, Membership::In), OpKind::AddLabel),
        (Op::Label(LABEL_A, Membership::Out), OpKind::RemoveLabel),
        (Op::SetSnooze(Snooze::Inactive), OpKind::Snooze),
        (Op::SetPin(Pin::Unpinned), OpKind::Pin),
        (Op::SetPin(Pin::Rank(7)), OpKind::Pin),
    ];

    for (op, expected) in CASES {
        assert_eq!(op.kind(), *expected, "kind of {op:?}");
    }

    assert_eq!(
        Op::SetSnooze(Snooze::Until(at(500))).kind(),
        OpKind::Snooze,
        "a snoozed-until payload is still the Snooze kind"
    );
}

// ---------------------------------------------------------------------------------------
// ThreadSummary::derive
// ---------------------------------------------------------------------------------------

#[test]
fn derive_rolls_up_the_thread() {
    let mut m1 = message(1, 300); // deliberately not in date order
    m1.subject = "Original subject".to_owned();
    m1.from = addr(Some("Ada"), "Ada@Example.test");
    m1.mailbox = MailboxRole::Inbox;
    m1.labels = vec![LABEL_B, LABEL_A];
    m1.attachments = vec![attachment("one.txt")];

    let mut m2 = message(2, 100);
    m2.subject = "Re: Original subject".to_owned();
    m2.from = addr(Some("Bob"), "bob@example.test");
    m2.mailbox = MailboxRole::Sent;
    m2.read = ReadState::Read;
    m2.labels = vec![LABEL_A];

    let mut m3 = message(3, 500);
    m3.subject = "Re: Re: Original subject".to_owned();
    // Same address as m1 in a different case: one participant, first spelling wins.
    m3.from = addr(Some("Ada Lovelace"), "ada@example.test");
    m3.mailbox = MailboxRole::Archive;
    m3.read = ReadState::Read;
    m3.star = Star::Starred;
    m3.labels = vec![LABEL_C];
    m3.attachments = vec![attachment("two.txt"), attachment("three.txt")];
    m3.body = Body::Present {
        text: Some("  the   newest\n\nbody  ".to_owned()),
        raw: m3.body.raw().expect("fixture has a body"),
    };

    let s = ThreadSummary::derive(THREAD, &[m1, m2, m3], Snooze::Until(at(900)), Pin::Rank(4));

    assert_eq!(s.id, THREAD);
    assert_eq!(s.account, ACCOUNT);
    // Oldest is m2 (date 100); its reply prefix stays exactly as it arrived.
    assert_eq!(s.subject, "Re: Original subject");
    // Newest is m3 (date 500).
    assert_eq!(s.snippet, "the newest body");
    assert_eq!(s.from, addr(Some("Ada Lovelace"), "ada@example.test"));
    assert_eq!(s.last_date, at(500));
    assert_eq!(
        s.participants,
        vec![
            addr(Some("Bob"), "bob@example.test"),
            addr(Some("Ada"), "Ada@Example.test"),
        ],
        "oldest first, deduplicated case-insensitively by email"
    );
    assert_eq!(s.message_count, 3);
    assert_eq!(s.read, ReadState::Unread, "m1 is unread, so the thread is");
    assert_eq!(s.star, Star::Starred, "m3 is starred, so the thread is");
    assert_eq!(
        s.mailboxes,
        MailboxSet::empty()
            .with(MailboxRole::Inbox)
            .with(MailboxRole::Sent)
            .with(MailboxRole::Archive)
    );
    assert_eq!(
        s.labels,
        vec![LABEL_A, LABEL_B, LABEL_C],
        "union in the order first seen, scanning oldest first"
    );
    assert_eq!(s.attachments, Attachments::Present { count: 3 });
    assert_eq!(
        s.snooze,
        Snooze::Until(at(900)),
        "thread state, carried through"
    );
    assert_eq!(s.pin, Pin::Rank(4), "thread state, carried through");
}

#[test]
fn derive_rolls_read_and_star_over_every_message() {
    struct Case {
        name: &'static str,
        flags: &'static [(ReadState, Star)],
        read: ReadState,
        star: Star,
    }
    const CASES: &[Case] = &[
        Case {
            name: "all read, none starred",
            flags: &[
                (ReadState::Read, Star::Unstarred),
                (ReadState::Read, Star::Unstarred),
            ],
            read: ReadState::Read,
            star: Star::Unstarred,
        },
        Case {
            name: "one unread wins",
            flags: &[
                (ReadState::Read, Star::Unstarred),
                (ReadState::Unread, Star::Unstarred),
            ],
            read: ReadState::Unread,
            star: Star::Unstarred,
        },
        Case {
            name: "one starred wins",
            flags: &[
                (ReadState::Read, Star::Unstarred),
                (ReadState::Read, Star::Starred),
            ],
            read: ReadState::Read,
            star: Star::Starred,
        },
        Case {
            name: "a single message decides both",
            flags: &[(ReadState::Unread, Star::Starred)],
            read: ReadState::Unread,
            star: Star::Starred,
        },
    ];

    for case in CASES {
        let messages: Vec<Message> = case
            .flags
            .iter()
            .enumerate()
            .map(|(i, (read, star))| {
                let mut m = message(i as u128, 100 * (i as i64 + 1));
                m.read = *read;
                m.star = *star;
                m
            })
            .collect();
        let s = ThreadSummary::derive(THREAD, &messages, Snooze::Inactive, Pin::Unpinned);
        assert_eq!(s.read, case.read, "{}: read", case.name);
        assert_eq!(s.star, case.star, "{}: star", case.name);
    }
}

#[test]
fn derive_builds_the_snippet_from_the_newest_body() {
    let cases: Vec<(&str, Option<String>, String)> = vec![
        ("no text part at all", None, String::new()),
        ("empty text part", Some(String::new()), String::new()),
        (
            "whitespace only",
            Some("  \n\t  ".to_owned()),
            String::new(),
        ),
        (
            "every run of whitespace collapses",
            Some("Hi  there,\r\n\r\n\tlong time.".to_owned()),
            "Hi there, long time.".to_owned(),
        ),
        (
            "truncated to 140 characters",
            Some("a".repeat(200)),
            "a".repeat(140),
        ),
        (
            "counted in characters, not bytes",
            Some("é".repeat(200)),
            "é".repeat(140),
        ),
        (
            "no trailing separator when the budget runs out mid-gap",
            // 139 'a's, then a space, then a word: the space would be character 140, so the
            // snippet stops before it rather than ending in one.
            Some(format!("{} tail", "a".repeat(139))),
            "a".repeat(139),
        ),
    ];

    for (name, text, expected) in cases {
        let mut newest = message(2, 200);
        newest.body = Body::Present {
            text,
            raw: newest.body.raw().expect("fixture has a body"),
        };
        let older = message(1, 100); // has no text part; must not be consulted
        let s = ThreadSummary::derive(THREAD, &[older, newest], Snooze::Inactive, Pin::Unpinned);
        assert_eq!(s.snippet, expected, "{name}");
        assert!(s.snippet.chars().count() <= 140, "{name}: length bound");
    }
}

#[test]
fn derive_breaks_date_ties_by_position() {
    let mut first = message(1, 100);
    first.subject = "first".to_owned();
    first.from = addr(None, "first@example.test");
    let mut second = message(2, 100);
    second.subject = "second".to_owned();
    second.from = addr(None, "second@example.test");

    let s = ThreadSummary::derive(THREAD, &[first, second], Snooze::Inactive, Pin::Unpinned);
    assert_eq!(
        s.subject, "first",
        "the earlier position is the older message"
    );
    assert_eq!(s.from, addr(None, "second@example.test"));
}

#[test]
#[should_panic(expected = "at least one message")]
fn derive_rejects_an_empty_thread() {
    // Documented caller invariant: a thread with no messages is programmer error, and there is
    // no honest summary to return for it.
    let _ = ThreadSummary::derive(THREAD, &[], Snooze::Inactive, Pin::Unpinned);
}

// ---------------------------------------------------------------------------------------
// Op::apply -- forward and inverse
// ---------------------------------------------------------------------------------------

struct ApplyCase {
    name: &'static str,
    op: Op,
    forward: Vec<Change>,
    inverse: Vec<Change>,
}

#[test]
fn apply_over_a_whole_thread() {
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let target = Target::Threads(vec![THREAD]);
    let snoozed = Snooze::Until(at(900));

    let cases = vec![
        ApplyCase {
            // m2 is already archived, so it is not touched, and undoing does not move it.
            // m1 and m3 came from *different* mailboxes and each goes back to its own.
            name: "archive a thread spanning three mailboxes",
            op: Op::Archive,
            forward: vec![
                Change::MessageMailbox(mid(1), MailboxRole::Archive),
                Change::MessageMailbox(mid(3), MailboxRole::Archive),
            ],
            inverse: vec![
                Change::MessageMailbox(mid(1), MailboxRole::Inbox),
                Change::MessageMailbox(mid(3), MailboxRole::Trash),
            ],
        },
        ApplyCase {
            name: "trash skips what is already trashed",
            op: Op::Trash,
            forward: vec![
                Change::MessageMailbox(mid(1), MailboxRole::Trash),
                Change::MessageMailbox(mid(2), MailboxRole::Trash),
            ],
            inverse: vec![
                Change::MessageMailbox(mid(1), MailboxRole::Inbox),
                Change::MessageMailbox(mid(2), MailboxRole::Archive),
            ],
        },
        ApplyCase {
            name: "spam moves everything",
            op: Op::Spam,
            forward: vec![
                Change::MessageMailbox(mid(1), MailboxRole::Spam),
                Change::MessageMailbox(mid(2), MailboxRole::Spam),
                Change::MessageMailbox(mid(3), MailboxRole::Spam),
            ],
            inverse: vec![
                Change::MessageMailbox(mid(1), MailboxRole::Inbox),
                Change::MessageMailbox(mid(2), MailboxRole::Archive),
                Change::MessageMailbox(mid(3), MailboxRole::Trash),
            ],
        },
        ApplyCase {
            name: "restore lifts archived and trashed messages only",
            op: Op::Restore,
            forward: vec![
                Change::MessageMailbox(mid(2), MailboxRole::Inbox),
                Change::MessageMailbox(mid(3), MailboxRole::Inbox),
            ],
            inverse: vec![
                Change::MessageMailbox(mid(2), MailboxRole::Archive),
                Change::MessageMailbox(mid(3), MailboxRole::Trash),
            ],
        },
        ApplyCase {
            // The mixed read/unread case: m2 was already read and stays out of both patches,
            // so undoing restores the mix rather than marking the whole thread unread.
            name: "mark read over a half-read thread",
            op: Op::SetRead(ReadState::Read),
            forward: vec![
                Change::MessageRead(mid(1), ReadState::Read),
                Change::MessageRead(mid(3), ReadState::Read),
            ],
            inverse: vec![
                Change::MessageRead(mid(1), ReadState::Unread),
                Change::MessageRead(mid(3), ReadState::Unread),
            ],
        },
        ApplyCase {
            name: "mark unread over a half-read thread",
            op: Op::SetRead(ReadState::Unread),
            forward: vec![Change::MessageRead(mid(2), ReadState::Unread)],
            inverse: vec![Change::MessageRead(mid(2), ReadState::Read)],
        },
        ApplyCase {
            name: "star over a half-starred thread",
            op: Op::SetStar(Star::Starred),
            forward: vec![
                Change::MessageStar(mid(1), Star::Starred),
                Change::MessageStar(mid(3), Star::Starred),
            ],
            inverse: vec![
                Change::MessageStar(mid(1), Star::Unstarred),
                Change::MessageStar(mid(3), Star::Unstarred),
            ],
        },
        ApplyCase {
            name: "unstar over a half-starred thread",
            op: Op::SetStar(Star::Unstarred),
            forward: vec![Change::MessageStar(mid(2), Star::Unstarred)],
            inverse: vec![Change::MessageStar(mid(2), Star::Starred)],
        },
        ApplyCase {
            // The crux: m1 and m3 already carry A. They contribute nothing to `forward` and
            // nothing to `inverse`, so undoing does not strip a label the user already had.
            name: "add a label two messages already carry",
            op: Op::Label(LABEL_A, Membership::In),
            forward: vec![Change::MessageLabel(mid(2), LABEL_A, Membership::In)],
            inverse: vec![Change::MessageLabel(mid(2), LABEL_A, Membership::Out)],
        },
        ApplyCase {
            name: "remove a label only two messages carry",
            op: Op::Label(LABEL_A, Membership::Out),
            forward: vec![
                Change::MessageLabel(mid(1), LABEL_A, Membership::Out),
                Change::MessageLabel(mid(3), LABEL_A, Membership::Out),
            ],
            inverse: vec![
                Change::MessageLabel(mid(1), LABEL_A, Membership::In),
                Change::MessageLabel(mid(3), LABEL_A, Membership::In),
            ],
        },
        ApplyCase {
            name: "add a label nobody carries",
            op: Op::Label(LABEL_C, Membership::In),
            forward: vec![
                Change::MessageLabel(mid(1), LABEL_C, Membership::In),
                Change::MessageLabel(mid(2), LABEL_C, Membership::In),
                Change::MessageLabel(mid(3), LABEL_C, Membership::In),
            ],
            inverse: vec![
                Change::MessageLabel(mid(1), LABEL_C, Membership::Out),
                Change::MessageLabel(mid(2), LABEL_C, Membership::Out),
                Change::MessageLabel(mid(3), LABEL_C, Membership::Out),
            ],
        },
        ApplyCase {
            name: "remove a label nobody carries",
            op: Op::Label(LABEL_C, Membership::Out),
            forward: Vec::new(),
            inverse: Vec::new(),
        },
        ApplyCase {
            name: "snooze the thread",
            op: Op::SetSnooze(snoozed),
            forward: vec![Change::ThreadSnooze(THREAD, snoozed)],
            inverse: vec![Change::ThreadSnooze(THREAD, Snooze::Inactive)],
        },
        ApplyCase {
            name: "snooze to the value it already has",
            op: Op::SetSnooze(Snooze::Inactive),
            forward: Vec::new(),
            inverse: Vec::new(),
        },
        ApplyCase {
            name: "pin the thread",
            op: Op::SetPin(Pin::Rank(3)),
            forward: vec![Change::ThreadPin(THREAD, Pin::Rank(3))],
            inverse: vec![Change::ThreadPin(THREAD, Pin::Unpinned)],
        },
        ApplyCase {
            name: "pin to the value it already has",
            op: Op::SetPin(Pin::Unpinned),
            forward: Vec::new(),
            inverse: Vec::new(),
        },
    ];

    for case in cases {
        let applied = case
            .op
            .apply(&target, &thread, &messages, &server_caps(), now());
        assert_eq!(
            applied.forward.changes, case.forward,
            "{}: forward",
            case.name
        );
        assert_eq!(
            applied.inverse.changes, case.inverse,
            "{}: inverse",
            case.name
        );
        assert_ne!(
            applied.forward.id, applied.inverse.id,
            "{}: a patch id names one application event, so an undo gets its own",
            case.name
        );
    }
}

#[test]
fn target_messages_acts_on_a_subset_of_the_thread() {
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);

    let target = Target::Messages(vec![mid(1)]);
    let applied =
        Op::SetRead(ReadState::Read).apply(&target, &thread, &messages, &server_caps(), now());
    assert_eq!(
        applied.forward.changes,
        vec![Change::MessageRead(mid(1), ReadState::Read)],
        "only the named message moves, though m3 is also unread"
    );

    // Two of the three, and only the one that is not already archived is touched.
    let target = Target::Messages(vec![mid(2), mid(3)]);
    let applied = Op::Archive.apply(&target, &thread, &messages, &server_caps(), now());
    assert_eq!(
        applied.forward.changes,
        vec![Change::MessageMailbox(mid(3), MailboxRole::Archive)]
    );
    assert_eq!(
        applied.inverse.changes,
        vec![Change::MessageMailbox(mid(3), MailboxRole::Trash)]
    );

    // A message of some other thread names nothing here.
    let target = Target::Messages(vec![mid(99)]);
    let applied = Op::Archive.apply(&target, &thread, &messages, &server_caps(), now());
    assert!(applied.forward.changes.is_empty());
    assert!(applied.inverse.changes.is_empty());
}

#[test]
fn a_thread_target_that_names_another_thread_changes_nothing() {
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let target = Target::Threads(vec![OTHER_THREAD]);

    for op in [
        Op::Archive,
        Op::SetRead(ReadState::Read),
        Op::SetSnooze(Snooze::Until(at(900))),
        Op::SetPin(Pin::Rank(1)),
    ] {
        let applied = op.apply(&target, &thread, &messages, &server_caps(), now());
        assert!(applied.forward.changes.is_empty(), "{op:?}: forward");
        assert!(applied.inverse.changes.is_empty(), "{op:?}: inverse");
    }
}

#[test]
fn a_message_target_snoozes_and_pins_the_whole_thread() {
    // There is no such thing as snoozing half a conversation: naming one of its messages
    // targets the thread.
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let target = Target::Messages(vec![mid(2)]);

    let applied =
        Op::SetPin(Pin::Rank(9)).apply(&target, &thread, &messages, &server_caps(), now());
    assert_eq!(
        applied.forward.changes,
        vec![Change::ThreadPin(THREAD, Pin::Rank(9))]
    );
}

// ---------------------------------------------------------------------------------------
// Op::apply -- remote work
// ---------------------------------------------------------------------------------------

#[test]
fn local_only_capabilities_queue_no_remote_work() {
    // These discriminate for real since F12: `Applied::remote` is `Some(RemoteIntent)` under
    // server-backed capabilities, so a `None` here means the capability was honoured rather
    // than that the domain could not address the server. The positive half of the contract is
    // `server_capabilities_queue_remote_intent` below.
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let target = Target::Threads(vec![THREAD]);

    let cases: Vec<(&str, Op, AccountCaps)> = vec![
        (
            "archive under ArchiveMeans::LocalOnly",
            Op::Archive,
            account_caps(ArchiveMeans::LocalOnly, ServerLabels::Supported),
        ),
        (
            "restore under ArchiveMeans::LocalOnly",
            Op::Restore,
            account_caps(ArchiveMeans::LocalOnly, ServerLabels::Supported),
        ),
        (
            "add a label under ServerLabels::LocalOnly",
            Op::Label(LABEL_C, Membership::In),
            account_caps(ArchiveMeans::DropInbox, ServerLabels::LocalOnly),
        ),
        (
            "remove a label under ServerLabels::LocalOnly",
            Op::Label(LABEL_A, Membership::Out),
            account_caps(ArchiveMeans::DropInbox, ServerLabels::LocalOnly),
        ),
    ];

    for (name, op, caps) in cases {
        let applied = op.apply(&target, &thread, &messages, &caps, now());
        assert!(applied.remote.is_none(), "{name}");
        assert!(
            !applied.forward.changes.is_empty(),
            "{name}: local-only still means local *work*, not a no-op"
        );
    }
}

#[test]
fn snooze_and_pin_never_reach_a_server() {
    // These two have no server representation at all, under any capabilities.
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let target = Target::Threads(vec![THREAD]);

    for op in [
        Op::SetSnooze(Snooze::Until(at(900))),
        Op::SetPin(Pin::Rank(2)),
    ] {
        let applied = op.apply(&target, &thread, &messages, &server_caps(), now());
        assert!(applied.remote.is_none(), "{op:?}");
        assert!(!applied.forward.changes.is_empty(), "{op:?}: local work");
    }
}

// ---------------------------------------------------------------------------------------
// The property: inverse restores the prior state.
// ---------------------------------------------------------------------------------------

/// Everything an [`Op`] can mutate. The summary is derived, so it is not part of the state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    messages: Vec<Message>,
    snooze: Snooze,
    pin: Pin,
}

fn find(messages: &mut [Message], id: MessageId) -> &mut Message {
    messages
        .iter_mut()
        .find(|m| m.id == id)
        .expect("a patch from Op::apply only ever names messages it was given")
}

/// Apply a [`Patch`] to a [`State`], the way `mail-store` will.
fn replay(mut state: State, patch: &Patch) -> State {
    for change in &patch.changes {
        match change {
            Change::MessageRead(id, read) => find(&mut state.messages, *id).read = *read,
            Change::MessageStar(id, star) => find(&mut state.messages, *id).star = *star,
            Change::MessageMailbox(id, role) => find(&mut state.messages, *id).mailbox = *role,
            Change::MessageLabel(id, label, Membership::In) => {
                let m = find(&mut state.messages, *id);
                if !m.labels.contains(label) {
                    m.labels.push(*label);
                }
            }
            Change::MessageLabel(id, label, Membership::Out) => {
                find(&mut state.messages, *id).labels.retain(|l| l != label);
            }
            Change::ThreadSnooze(_, snooze) => state.snooze = *snooze,
            Change::ThreadPin(_, pin) => state.pin = *pin,
            other => panic!("an Op never produces {other:?}"),
        }
    }
    state
}

/// A message's labels are a *set*: `Change::MessageLabel` carries membership, not position, so
/// re-adding a removed label may land it at the end. Compare membership, not order.
fn canonical(mut state: State) -> State {
    for m in &mut state.messages {
        m.labels.sort_unstable();
    }
    state
}

fn arb_read() -> impl Strategy<Value = ReadState> {
    prop_oneof![Just(ReadState::Unread), Just(ReadState::Read)]
}

fn arb_star() -> impl Strategy<Value = Star> {
    prop_oneof![Just(Star::Unstarred), Just(Star::Starred)]
}

fn arb_mailbox() -> impl Strategy<Value = MailboxRole> {
    prop_oneof![
        Just(MailboxRole::Inbox),
        Just(MailboxRole::Archive),
        Just(MailboxRole::Sent),
        Just(MailboxRole::Drafts),
        Just(MailboxRole::Trash),
        Just(MailboxRole::Spam),
    ]
}

fn arb_label() -> impl Strategy<Value = LabelId> {
    prop_oneof![Just(LABEL_A), Just(LABEL_B), Just(LABEL_C)]
}

fn arb_labels() -> impl Strategy<Value = Vec<LabelId>> {
    proptest::collection::vec(arb_label(), 0..4).prop_map(|mut v| {
        v.sort_unstable();
        v.dedup();
        v
    })
}

fn arb_snooze() -> impl Strategy<Value = Snooze> {
    prop_oneof![
        Just(Snooze::Inactive),
        (0i64..2_000).prop_map(|s| Snooze::Until(at(s))),
    ]
}

fn arb_pin() -> impl Strategy<Value = Pin> {
    prop_oneof![Just(Pin::Unpinned), (-4i64..4).prop_map(Pin::Rank)]
}

fn arb_message(index: u128) -> impl Strategy<Value = Message> {
    (
        1i64..1_000,
        arb_read(),
        arb_star(),
        arb_mailbox(),
        arb_labels(),
    )
        .prop_map(move |(date, read, star, mailbox, labels)| {
            let mut m = message(index, date);
            m.read = read;
            m.star = star;
            m.mailbox = mailbox;
            m.labels = labels;
            m
        })
}

fn arb_messages() -> impl Strategy<Value = Vec<Message>> {
    (1usize..5).prop_flat_map(|n| (0..n).map(|i| arb_message(i as u128)).collect::<Vec<_>>())
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        Just(Op::Archive),
        Just(Op::Trash),
        Just(Op::Restore),
        Just(Op::Spam),
        arb_read().prop_map(Op::SetRead),
        arb_star().prop_map(Op::SetStar),
        (
            arb_label(),
            prop_oneof![Just(Membership::In), Just(Membership::Out)]
        )
            .prop_map(|(l, m)| Op::Label(l, m)),
        arb_snooze().prop_map(Op::SetSnooze),
        arb_pin().prop_map(Op::SetPin),
    ]
}

/// A thread target that hits, a thread target that misses, or any subset of the messages.
fn arb_target(ids: Vec<MessageId>) -> impl Strategy<Value = Target> {
    let n = ids.len();
    prop_oneof![
        Just(Target::Threads(vec![THREAD])),
        Just(Target::Threads(vec![OTHER_THREAD, THREAD])),
        Just(Target::Threads(vec![OTHER_THREAD])),
        proptest::sample::subsequence(ids, 0..=n).prop_map(Target::Messages),
    ]
}

fn arb_state_and_target() -> impl Strategy<Value = (Vec<Message>, Target)> {
    arb_messages().prop_flat_map(|messages| {
        let ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
        (Just(messages), arb_target(ids))
    })
}

proptest! {
    /// Apply an arbitrary op to an arbitrary thread, then apply its inverse: the state that
    /// comes back must be the state we started from. This is the only thing that proves
    /// `inverse` is computed from the prior state rather than by mirroring the op.
    #[test]
    fn inverse_restores_the_prior_state(
        (messages, target) in arb_state_and_target(),
        snooze in arb_snooze(),
        pin in arb_pin(),
        op in arb_op(),
        server in any::<bool>(),
    ) {
        let state = State { messages, snooze, pin };
        let thread = thread_of(&state.messages, state.snooze, state.pin);
        let caps = if server {
            server_caps()
        } else {
            account_caps(ArchiveMeans::LocalOnly, ServerLabels::LocalOnly)
        };

        let applied = op.apply(&target, &thread, &state.messages, &caps, now());

        let forward = replay(state.clone(), &applied.forward);
        let back = replay(forward.clone(), &applied.inverse);

        prop_assert_eq!(canonical(back), canonical(state.clone()));
        prop_assert_ne!(applied.forward.id, applied.inverse.id);
        prop_assert_eq!(
            applied.forward.changes.is_empty(),
            applied.inverse.changes.is_empty(),
            "a patch and its undo are empty together"
        );
        // An op that changes nothing must not have moved the state either.
        if applied.forward.changes.is_empty() {
            prop_assert_eq!(canonical(forward), canonical(state));
        }
    }
}

/// The other half of `local_only_capabilities_queue_no_remote_work`: when the server *can*
/// hold the state, the intent must actually be produced, name only the messages that changed,
/// and stay addressed in local ids.
#[test]
fn server_capabilities_queue_remote_intent() {
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let target = Target::Threads(vec![THREAD]);
    let caps = account_caps(ArchiveMeans::DropInbox, ServerLabels::Supported);

    let archive = Op::Archive.apply(&target, &thread, &messages, &caps, now());
    match archive.remote {
        Some(RemoteIntent::SetMailbox { messages: m, role }) => {
            assert_eq!(role, MailboxRole::Archive);
            // Only what actually moved: a message already in Archive is not re-sent.
            let moved: Vec<MessageId> = archive
                .forward
                .changes
                .iter()
                .filter_map(|c| match c {
                    Change::MessageMailbox(id, _) => Some(*id),
                    _ => None,
                })
                .collect();
            assert_eq!(m, moved, "intent must name exactly the changed messages");
            assert!(!m.is_empty());
        }
        other => panic!("expected SetMailbox intent, got {other:?}"),
    }

    let star = Op::SetStar(Star::Starred).apply(&target, &thread, &messages, &caps, now());
    assert!(
        matches!(
            star.remote,
            Some(RemoteIntent::SetFlags {
                star: Some(Star::Starred),
                read: None,
                ..
            })
        ),
        "star must travel as a flag, with read untouched: {:?}",
        star.remote
    );

    let label = Op::Label(LABEL_C, Membership::In).apply(&target, &thread, &messages, &caps, now());
    match label.remote {
        Some(RemoteIntent::SetLabels { add, remove, .. }) => {
            assert_eq!(add, vec![LABEL_C]);
            assert!(remove.is_empty());
        }
        other => panic!("expected SetLabels intent, got {other:?}"),
    }
}

/// A local no-op must not become a remote round trip telling the server what it already knows.
#[test]
fn an_op_that_changes_nothing_queues_nothing() {
    let messages = mixed_thread();
    let thread = thread_of(&messages, Snooze::Inactive, Pin::Unpinned);
    let caps = account_caps(ArchiveMeans::DropInbox, ServerLabels::Supported);
    // A target naming no message of this thread selects nothing, so nothing changes.
    let applied = Op::SetStar(Star::Starred).apply(
        &Target::Messages(vec![MessageId::generate()]),
        &thread,
        &messages,
        &caps,
        now(),
    );
    assert!(applied.forward.changes.is_empty());
    assert!(
        applied.remote.is_none(),
        "no local change means nothing to tell the server"
    );
}

/// `recipients` is a union over To and Cc, deduplicated case-insensitively. Bcc must never
/// appear: a blind copy is not a fact the thread's other readers share.
#[test]
fn derive_rolls_up_recipients_without_bcc() {
    let mut a = message(1, 100);
    a.to = vec![addr(None, "Ada@Example.test")];
    a.cc = vec![addr(None, "cc@example.test")];
    a.bcc = vec![addr(None, "blind@example.test")];
    let mut b = message(2, 200);
    // Same person, different spelling: one entry, first spelling wins.
    b.to = vec![addr(None, "ada@example.test")];
    b.cc = vec![];
    b.bcc = vec![];

    let summary = ThreadSummary::derive(THREAD, &[a, b], Snooze::Inactive, Pin::Unpinned);
    let emails: Vec<&str> = summary
        .recipients
        .iter()
        .map(|a| a.email.as_str())
        .collect();
    assert_eq!(emails, vec!["Ada@Example.test", "cc@example.test"]);
    assert!(
        !summary.recipients.iter().any(|a| a.email.contains("blind")),
        "Bcc must not surface in a list row"
    );
}

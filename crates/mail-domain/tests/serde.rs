//! The schema safety net (`CONVENTIONS.md` §3).
//!
//! The serde form of a domain type is a **persisted schema**: `AccountPlan`, `RemoteRef`,
//! `ProtoOp`, `SyncCursor`, `Patch`, `View` and the rest round-trip through SQLite. Changing
//! a representation is a migration, not a refactor, and the failure mode of getting it wrong
//! is silent — rows written by yesterday's build stop loading, or load as something else.
//!
//! Three groups of tests, and they do different jobs:
//!
//! 1. **Round trips.** `value -> JSON -> value` for every persisted type. Catches a
//!    representation that cannot read its own output (an internally-tagged newtype variant
//!    over a non-map, a `skip_serializing` on a field that has no default).
//! 2. **Shape assertions.** A round trip is happy with *any* stable encoding, including a
//!    bitmask. `MailboxSet` must persist as an array of role names, and `Credential`'s
//!    `Debug` must not print secrets; both are asserted directly.
//! 3. **Frozen fixtures.** JSON written by an earlier build, in `tests/fixtures/`, that must
//!    still deserialize.
//!
//! ## The fixtures are APPEND-ONLY
//!
//! **Never edit a fixture to make a test pass.** A fixture that no longer deserializes *is*
//! the finding: it means a change to a persisted type just broke every row already on a
//! user's disk. That is precisely the silent break these files exist to catch, and editing
//! the file deletes the evidence and ships the bug. Add a new fixture beside the old one
//! when a representation gains a variant or a field; leave what is there alone.
//!
//! Every time is a fixed instant. Nothing here calls `Utc::now()` — a test that moves with
//! the clock cannot be re-run against a failure.

use mail_domain::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt::Debug;
use std::time::Duration;
use uuid::Uuid;

// ---------------------------------------------------------------------------------------
// Fixed, reproducible building blocks.
// ---------------------------------------------------------------------------------------

/// A deterministic UUID. Fixtures must be byte-stable between runs, so no `generate()`.
fn uuid(n: u8) -> Uuid {
    Uuid::from_bytes([n; 16])
}

fn at(day: u32) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(&format!("2026-01-{day:02}T04:05:06Z"))
        .expect("literal is valid RFC 3339")
        .with_timezone(&chrono::Utc)
}

fn account() -> AccountId {
    AccountId::from_uuid(uuid(1))
}

fn address(email: &str) -> Address {
    Address {
        name: Some("A Name".to_owned()),
        email: email.to_owned(),
    }
}

fn identity() -> Identity {
    Identity {
        id: IdentityId::from_uuid(uuid(2)),
        account: account(),
        from: address("me@example.test"),
        reply_to: Some(address("alias@example.test")),
        signature: Some("-- me".to_owned()),
        default: IsDefault::Default,
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::Supported,
        threads: ServerThreads::ProviderId,
        watch: WatchMode::Idle,
        archive: ArchiveMeans::DropInbox,
        folders: FolderRoles(vec![
            ("INBOX".to_owned(), MailboxRole::Inbox),
            ("[Gmail]/All Mail".to_owned(), MailboxRole::Archive),
        ]),
        condstore: Condstore::Supported,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: at(3),
    }
}

fn label() -> Label {
    Label {
        id: LabelId::from_uuid(uuid(3)),
        account: account(),
        name: "Receipts".to_owned(),
        color: Some("#ff8800".to_owned()),
        origin: LabelOrigin::User,
    }
}

fn attachment() -> Attachment {
    Attachment {
        name: "invoice.pdf".to_owned(),
        mime: "application/pdf".to_owned(),
        size: 12_345,
        content: PartContent::Held(BlobId::from_uuid(uuid(4))),
        inline: Inline::Embedded {
            cid: "part1@example.test".to_owned(),
        },
    }
}

fn message() -> Message {
    Message {
        id: MessageId::from_uuid(uuid(5)),
        thread: ThreadId::from_uuid(uuid(6)),
        account: account(),
        key: MessageKey::Rfc("abc@example.test".to_owned()),
        date: at(4),
        from: address("sender@example.test"),
        reply_to: Vec::new(),
        to: vec![address("me@example.test")],
        cc: vec![address("cc@example.test")],
        bcc: vec![],
        subject: "Lunch".to_owned(),
        in_reply_to: Some("older@example.test".to_owned()),
        references: vec!["oldest@example.test".to_owned()],
        rfc_message_id: Some("abc@example.test".to_owned()),
        read: ReadState::Read,
        star: Star::Starred,
        mailbox: MailboxRole::Inbox,
        labels: vec![LabelId::from_uuid(uuid(3))],
        body: Body::Present {
            text: Some("hello".to_owned()),
            raw: BlobId::from_uuid(uuid(7)),
        },
        attachments: vec![attachment()],
    }
}

/// Built literally, not via `ThreadSummary::derive` — that is another agent's wave-1 work and
/// is still `todo!()`. A schema test must not depend on someone else's body.
fn summary() -> ThreadSummary {
    ThreadSummary {
        id: ThreadId::from_uuid(uuid(6)),
        account: account(),
        subject: "Lunch".to_owned(),
        snippet: "hello".to_owned(),
        from: address("sender@example.test"),
        participants: vec![address("sender@example.test"), address("me@example.test")],
        recipients: Vec::new(),
        last_date: at(4),
        message_count: 2,
        read: ReadState::Unread,
        star: Star::Starred,
        mailboxes: MailboxSet::only(MailboxRole::Inbox).with(MailboxRole::Sent),
        labels: vec![LabelId::from_uuid(uuid(3))],
        attachments: Attachments::Present { count: 1 },
        snooze: Snooze::Until(at(9)),
        pin: Pin::Rank(1_000),
    }
}

fn thread() -> Thread {
    Thread {
        summary: summary(),
        messages: vec![MessageId::from_uuid(uuid(5)), MessageId::from_uuid(uuid(8))],
    }
}

fn draft() -> Draft {
    Draft {
        id: DraftId::from_uuid(uuid(9)),
        account: account(),
        identity: IdentityId::from_uuid(uuid(2)),
        to: vec![address("you@example.test")],
        cc: vec![],
        bcc: vec![address("secret@example.test")],
        subject: "Re: Lunch".to_owned(),
        in_reply_to: Some(MessageId::from_uuid(uuid(5))),
        forward_of: None,
        text: "yes".to_owned(),
        html: Some("<p>yes</p>".to_owned()),
        attachments: vec![PendingAttachment {
            name: "map.png".to_owned(),
            mime: "image/png".to_owned(),
            blob: BlobId::from_uuid(uuid(10)),
        }],
        receipt: ReceiptRequest::Unrequested,
        state: SendState::Failed {
            reason: "550 rejected".to_owned(),
            retry: Retry::After(Duration::from_secs(90)),
        },
        updated: at(5),
    }
}

fn template() -> Template {
    Template {
        id: TemplateId::from_uuid(uuid(13)),
        account: account(),
        identity: IdentityId::from_uuid(uuid(2)),
        name: "weekly".to_owned(),
        to: vec![address("team@example.test")],
        cc: vec![],
        bcc: vec![address("archive@example.test")],
        subject: "Weekly report".to_owned(),
        text: "This week:".to_owned(),
        html: None,
        attachments: vec![PendingAttachment {
            name: "plan.pdf".to_owned(),
            mime: "application/pdf".to_owned(),
            blob: BlobId::from_uuid(uuid(10)),
        }],
        receipt: ReceiptRequest::Requested,
        updated: at(5),
    }
}

fn imap_ref() -> RemoteRef {
    RemoteRef::Imap {
        mailbox: "[Gmail]/All Mail".to_owned(),
        uidvalidity: 12,
        uid: 4_294_967_295,
    }
}

fn mailbox_ref() -> MailboxRef {
    MailboxRef {
        account: account(),
        path: "[Gmail]/All Mail".to_owned(),
    }
}

fn graph_ref() -> RemoteRef {
    RemoteRef::Graph {
        mailbox: "INBOX".to_owned(),
        id: "AAMkAGI2TAAA=".to_owned(),
    }
}

fn graph_cursor() -> SyncCursor {
    SyncCursor::Graph {
        delta_link:
            "https://graph.microsoft.com/v1.0/me/mailFolders/inbox/messages/delta?$deltatoken=abc"
                .to_owned(),
    }
}

fn imap_cursor() -> SyncCursor {
    SyncCursor::Imap {
        uidvalidity: 12,
        uidnext: 900,
        modseq: Some(4_294_967_296),
    }
}

fn proto_ops() -> Vec<ProtoOp> {
    vec![
        ProtoOp::FetchCaps,
        ProtoOp::ListFolders,
        ProtoOp::FetchEnvelopes {
            mailbox: mailbox_ref(),
            since: FetchSince::Beginning,
        },
        ProtoOp::FetchEnvelopes {
            mailbox: mailbox_ref(),
            since: FetchSince::After {
                cursor: imap_cursor(),
            },
        },
        ProtoOp::FetchBody {
            remotes: vec![imap_ref()],
        },
        ProtoOp::SetFlags {
            remotes: vec![imap_ref()],
            read: Some(ReadState::Read),
            star: None,
        },
        ProtoOp::SetMailbox {
            remotes: vec![imap_ref()],
            role: MailboxRole::Trash,
        },
        ProtoOp::SetLabels {
            remotes: vec![imap_ref()],
            add: vec!["Receipts".to_owned()],
            remove: vec!["\\Inbox".to_owned()],
        },
        ProtoOp::Append {
            mailbox: mailbox_ref(),
            flags: vec![SystemFlag::Draft, SystemFlag::Seen],
            date: None,
            raw: BlobId::from_uuid(uuid(7)),
        },
        ProtoOp::Submit {
            draft: DraftId::from_uuid(uuid(9)),
            raw: BlobId::from_uuid(uuid(7)),
            mail_from: "me@example.test".to_owned(),
            rcpt_to: vec![
                "to@example.test".to_owned(),
                "blind@example.test".to_owned(),
            ],
        },
        ProtoOp::Expunge {
            remotes: vec![RemoteRef::Pop {
                uidl: "UID-1".to_owned(),
            }],
        },
        ProtoOp::Watch {
            mailbox: mailbox_ref(),
            uidnext: None,
        },
        ProtoOp::AddKeyword {
            remotes: vec![imap_ref()],
            keyword: Keyword::MdnSent,
        },
    ]
}

/// Every [`Filter`] variant, once.
fn folders() -> Vec<Folder> {
    vec![
        Folder {
            account: account(),
            path: "[Gmail]/Sent Mail".to_owned(),
            delimiter: Some('/'),
            special: Some(SpecialUse::Sent),
            subscription: Subscription::Subscribed,
            holds: Holds::Mail,
        },
        Folder {
            account: account(),
            path: "[Gmail]".to_owned(),
            delimiter: Some('/'),
            special: None,
            subscription: Subscription::Unsubscribed,
            holds: Holds::FoldersOnly,
        },
        Folder {
            account: account(),
            path: "日本語".to_owned(),
            delimiter: None,
            special: None,
            subscription: Subscription::Subscribed,
            holds: Holds::Mail,
        },
    ]
}

/// Every shape of folder work, as the outbox stores it.
fn folder_ops() -> Vec<ProtoOp> {
    vec![
        ProtoOp::Folder(FolderWork::Create {
            path: "Receipts".to_owned(),
        }),
        ProtoOp::Folder(FolderWork::Rename {
            from: "Work".to_owned(),
            to: "Work/2026".to_owned(),
        }),
        ProtoOp::Folder(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Refuse,
        }),
        ProtoOp::Folder(FolderWork::Delete {
            path: "Old".to_owned(),
            non_empty: NonEmpty::Allow,
        }),
        ProtoOp::Folder(FolderWork::Subscribe {
            path: "Lists".to_owned(),
            subscription: Subscription::Unsubscribed,
        }),
    ]
}

/// The undo of folder work, as the outbox stores it.
fn folder_patch() -> Patch {
    Patch {
        id: ChangeId::from_uuid(uuid(8)),
        changes: vec![
            Change::FolderUpsert(folders().remove(0)),
            Change::FolderRemove(mailbox_ref()),
            Change::FolderRename {
                from: mailbox_ref(),
                to: "Archive/Old".to_owned(),
                delimiter: Some('/'),
            },
            Change::LabelRemove(LabelId::from_uuid(uuid(4))),
        ],
    }
}

fn filters() -> Vec<Filter> {
    vec![
        Filter::All,
        Filter::Nothing,
        Filter::And(vec![Filter::All, Filter::Pinned]),
        Filter::Or(vec![Filter::Nothing]),
        Filter::Not(Box::new(Filter::All)),
        Filter::Account(account()),
        Filter::InMailbox(MailboxRole::Spam),
        Filter::Read(ReadState::Unread),
        Filter::Starred(Star::Starred),
        Filter::HasLabel(LabelId::from_uuid(uuid(3))),
        Filter::From(TextMatch::Contains("sender".to_owned())),
        Filter::To(TextMatch::Exact("me@example.test".to_owned())),
        Filter::Subject(TextMatch::Contains("lunch".to_owned())),
        Filter::Text(TextMatch::Contains("hello".to_owned())),
        Filter::Date(DateRange {
            from: Some(at(1)),
            to: None,
        }),
        Filter::HasAttachment,
        Filter::Snoozed,
        Filter::SnoozeDue,
        Filter::Pinned,
    ]
}

fn rules() -> Vec<Rule> {
    vec![
        Rule {
            id: RuleId::from_uuid(uuid(20)),
            account: account(),
            name: "Bills".to_owned(),
            position: 1,
            state: RuleState::Enabled,
            filter: Filter::And(vec![
                Filter::From(TextMatch::Contains("bank.example".to_owned())),
                Filter::Not(Box::new(Filter::Read(ReadState::Read))),
            ]),
            actions: vec![
                RuleAction::Label("Money".to_owned()),
                RuleAction::File("Money/Bills".to_owned()),
                RuleAction::MarkRead,
                RuleAction::Star,
            ],
            after: AfterMatch::Stop,
        },
        Rule {
            id: RuleId::from_uuid(uuid(21)),
            account: account(),
            name: "Tidy".to_owned(),
            position: 2,
            state: RuleState::Disabled,
            filter: Filter::All,
            actions: vec![RuleAction::Archive, RuleAction::Trash, RuleAction::Spam],
            after: AfterMatch::Continue,
        },
    ]
}

fn vacation() -> Vacation {
    Vacation {
        account: account(),
        subject: "Away".to_owned(),
        body: "Back on the 8th.".to_owned(),
        days: Vacation::DEFAULT_DAYS,
        addresses: vec!["me@example.test".to_owned()],
        from: Some("me@example.test".to_owned()),
        during: DateRange {
            from: Some(at(1)),
            to: Some(at(8)),
        },
    }
}

#[test]
fn rule_types_round_trip() {
    round_trip_each("Rule", rules());
    round_trip("Vacation", vacation());
    round_trip("Op::File", Op::File(LabelId::from_uuid(uuid(3))));
}

fn view() -> View {
    View {
        id: ViewId::from_uuid(uuid(11)),
        name: "Unread receipts".to_owned(),
        kind: ViewKind::PlaceLabel {
            label: LabelId::from_uuid(uuid(3)),
        },
        filter: Filter::And(filters()),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        group_by: Some(GroupKey::Property(Property::From)),
        threading: Threading::Threaded,
        shown: vec![Property::Date, Property::Subject, Property::Attachments],
        hover: vec![OpKind::Archive, OpKind::Reply, OpKind::Forward],
    }
}

fn query() -> Query {
    Query {
        filter: Filter::All,
        sort: Sort {
            property: Property::Subject,
            dir: SortDir::Asc,
        },
        page: PageReq {
            after: Some(Cursor("eyJkIjoxfQ".to_owned())),
            limit: 50,
        },
    }
}

/// Every [`Change`] variant, once.
fn patch() -> Patch {
    Patch {
        id: ChangeId::from_uuid(uuid(12)),
        changes: vec![
            Change::MessageRead(MessageId::from_uuid(uuid(5)), ReadState::Read),
            Change::MessageStar(MessageId::from_uuid(uuid(5)), Star::Unstarred),
            Change::MessageMailbox(MessageId::from_uuid(uuid(5)), MailboxRole::Archive),
            Change::MessageLabel(
                MessageId::from_uuid(uuid(5)),
                LabelId::from_uuid(uuid(3)),
                Membership::In,
            ),
            Change::ThreadSnooze(ThreadId::from_uuid(uuid(6)), Snooze::Inactive),
            Change::ThreadPin(ThreadId::from_uuid(uuid(6)), Pin::Unpinned),
            Change::MessageUpsert(Box::new(message())),
            Change::MessageDelete(MessageId::from_uuid(uuid(8))),
            Change::LabelUpsert(label()),
            Change::DraftUpsert(Box::new(draft())),
            Change::DraftDelete(DraftId::from_uuid(uuid(9))),
        ],
    }
}

fn ingest() -> Ingest {
    Ingest {
        mailbox: mailbox_ref(),
        validity: UidValidity::Reset,
        cursor: Some(imap_cursor()),
        messages: vec![Fetched {
            remote: imap_ref(),
            key: MessageKey::Gmail(18_446_744_073_709_551_615),
            raw: BlobId::from_uuid(uuid(7)),
            message: message(),
        }],
        flags: vec![(imap_ref(), ReadState::Unread, Star::Starred)],
        labels: vec![label()],
        // Non-empty, so the round trip covers a shape a Gmail survey actually produces.
        label_names: vec![(imap_ref(), vec!["travel".to_owned(), "家人".to_owned()])],
        gone: vec![RemoteRef::Pop {
            uidl: "UID-2".to_owned(),
        }],
    }
}

fn actions() -> Vec<Action> {
    let ops = [
        Op::Archive,
        Op::Trash,
        Op::Restore,
        Op::Spam,
        Op::SetRead(ReadState::Read),
        Op::SetStar(Star::Starred),
        Op::Label(LabelId::from_uuid(uuid(3)), Membership::Out),
        Op::SetSnooze(Snooze::Until(at(9))),
        Op::SetPin(Pin::Rank(-3)),
    ];
    ops.into_iter()
        .map(|op| Action {
            target: Target::Threads(vec![ThreadId::from_uuid(uuid(6))]),
            op,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Group 1 — round trips.
// ---------------------------------------------------------------------------------------

#[track_caller]
fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(label: &str, value: T) {
    let json = serde_json::to_string(&value).unwrap_or_else(|e| panic!("{label}: serialize: {e}"));
    let back: T = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("{label}: deserialize failed on its own output {json}: {e}"));
    assert_eq!(
        value, back,
        "{label}: round trip changed the value ({json})"
    );
}

/// Round-trip each element of a list of variants, naming the index that failed.
#[track_caller]
fn round_trip_each<T: Serialize + DeserializeOwned + PartialEq + Debug>(
    label: &str,
    values: Vec<T>,
) {
    for (i, value) in values.into_iter().enumerate() {
        round_trip(&format!("{label}[{i}]"), value);
    }
}

/// An attachment still on the server round-trips, and so does one stored.
#[test]
fn attachment_content_round_trips() {
    round_trip("held", attachment());
    round_trip(
        "remote",
        Attachment {
            content: PartContent::Remote {
                section: "1.3".to_owned(),
            },
            ..attachment()
        },
    );
}

/// Rows written before parts could be remote carry a bare `blob`, and are every attachment
/// already on disk. They must read as held, unchanged.
#[test]
fn an_attachment_row_from_before_remote_parts_still_reads() {
    let old = r#"{"name":"invoice.pdf","mime":"application/pdf","size":12345,
        "blob":"04040404-0404-0404-0404-040404040404",
        "inline":{"kind":"attached"}}"#;
    let read: Attachment = serde_json::from_str(old).expect("an old row must still parse");
    assert_eq!(read.content, PartContent::Held(BlobId::from_uuid(uuid(4))));
    assert_eq!(
        serde_json::to_value(&read).unwrap()["blob"],
        "04040404-0404-0404-0404-040404040404",
        "and a held part is still written the old way"
    );
}

#[test]
fn account_types_round_trip() {
    round_trip_each(
        "Tls",
        vec![Tls::Implicit, Tls::StartTlsRequired, Tls::Plaintext],
    );
    round_trip_each(
        "LeaveOnServer",
        vec![LeaveOnServer::Keep, LeaveOnServer::DeleteAfterFetch],
    );
    round_trip_each(
        "Incoming",
        vec![
            Incoming::Imap {
                host: "imap.example.test".to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            Incoming::Pop3 {
                host: "pop.example.test".to_owned(),
                port: 995,
                tls: Tls::StartTlsRequired,
                leave: LeaveOnServer::Keep,
            },
            Incoming::Graph,
        ],
    );
    round_trip(
        "Outgoing",
        Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
    );
    round_trip_each(
        "Username",
        vec![
            Username::SameAsAddress,
            Username::LocalPart,
            Username::Literal("login-name".to_owned()),
        ],
    );
    round_trip_each(
        "SaslMech",
        vec![SaslMech::Plain, SaslMech::Login, SaslMech::XOauth2],
    );
    round_trip("OAuthIssuer", OAuthIssuer::Google);
    round_trip_each(
        "AuthPlan",
        vec![
            AuthPlan::OAuth {
                issuer: OAuthIssuer::Google,
                scopes: vec!["https://mail.google.com/".to_owned()],
            },
            AuthPlan::Password {
                username: Username::LocalPart,
                sasl: vec![SaslMech::Login, SaslMech::Plain],
            },
        ],
    );
    round_trip("Identity", identity());
    round_trip(
        "AccountPlan",
        AccountPlan {
            address: "me@example.test".to_owned(),
            incoming: Incoming::Imap {
                host: "imap.example.test".to_owned(),
                port: 993,
                tls: Tls::Implicit,
            },
            outgoing: Outgoing::Smtp {
                host: "smtp.example.test".to_owned(),
                port: 465,
                tls: Tls::Implicit,
            },
            auth: AuthPlan::Password {
                username: Username::SameAsAddress,
                sasl: vec![SaslMech::Plain],
            },
            identities: vec![identity()],
        },
    );

    round_trip_each(
        "ServerLabels",
        vec![ServerLabels::Supported, ServerLabels::LocalOnly],
    );
    round_trip_each(
        "ServerThreads",
        vec![ServerThreads::ProviderId, ServerThreads::Jwz],
    );
    round_trip_each(
        "WatchMode",
        vec![
            WatchMode::Idle,
            WatchMode::Poll {
                every: Duration::from_secs(300),
            },
        ],
    );
    round_trip_each(
        "ArchiveMeans",
        vec![
            ArchiveMeans::DropInbox,
            ArchiveMeans::MoveToFolder("Archive".to_owned()),
            ArchiveMeans::LocalOnly,
        ],
    );
    round_trip(
        "FolderRoles",
        FolderRoles(vec![("INBOX".to_owned(), MailboxRole::Inbox)]),
    );
    round_trip("FolderRoles/empty", FolderRoles::default());
    round_trip_each(
        "Condstore",
        vec![Condstore::Supported, Condstore::Qresync, Condstore::Absent],
    );
    round_trip_each("MoveExt", vec![MoveExt::Supported, MoveExt::Absent]);
    round_trip("AccountCaps", caps());

    round_trip_each(
        "SecretPurpose",
        vec![
            SecretPurpose::IncomingPassword,
            SecretPurpose::OutgoingPassword,
            SecretPurpose::OAuthRefresh,
        ],
    );
    round_trip(
        "SecretKey",
        SecretKey {
            account: account(),
            purpose: SecretPurpose::OAuthRefresh,
        },
    );
    round_trip_each(
        "Credential",
        vec![
            Credential::Password("hunter2".to_owned()),
            Credential::OAuth {
                access: "ya29.access".to_owned(),
                refresh: "1//refresh".to_owned(),
                expires_at: at(6),
            },
        ],
    );
}

#[test]
fn remote_types_round_trip() {
    round_trip_each(
        "RemoteRef",
        vec![
            imap_ref(),
            RemoteRef::Pop {
                uidl: "UID-1".to_owned(),
            },
            graph_ref(),
        ],
    );
    round_trip("MailboxRef", mailbox_ref());
    round_trip_each(
        "SyncCursor",
        vec![
            imap_cursor(),
            SyncCursor::Imap {
                uidvalidity: 1,
                uidnext: 2,
                modseq: None,
            },
            SyncCursor::Pop,
            graph_cursor(),
        ],
    );
    round_trip_each("UidValidity", vec![UidValidity::Same, UidValidity::Reset]);
    round_trip_each(
        "FetchSince",
        vec![
            FetchSince::Beginning,
            FetchSince::After {
                cursor: SyncCursor::Pop,
            },
        ],
    );
    round_trip_each("ProtoOp", proto_ops());
    round_trip_each(
        "Retry",
        vec![
            Retry::Now,
            Retry::After(Duration::from_millis(1_500)),
            Retry::NeedsReauth,
            Retry::Fatal("message gone".to_owned()),
        ],
    );
}

#[test]
fn filter_and_view_types_round_trip() {
    round_trip_each(
        "TextMatch",
        vec![
            TextMatch::Contains("a".to_owned()),
            TextMatch::Exact("b".to_owned()),
        ],
    );
    round_trip_each(
        "DateRange",
        vec![
            DateRange::default(),
            DateRange {
                from: Some(at(1)),
                to: Some(at(2)),
            },
        ],
    );
    round_trip_each("Filter", filters());
    // Nested, so the recursive variants are exercised inside a container too.
    round_trip("Filter/nested", Filter::And(filters()));
    round_trip_each(
        "ViewKind",
        vec![
            ViewKind::Place {
                mailbox: MailboxRole::Inbox,
            },
            ViewKind::PlaceLabel {
                label: LabelId::from_uuid(uuid(3)),
            },
            ViewKind::Query,
        ],
    );
    round_trip_each(
        "Property",
        vec![
            Property::Date,
            Property::Subject,
            Property::From,
            Property::Sender,
            Property::Size,
            Property::Attachments,
            Property::Pin,
        ],
    );
    round_trip_each("SortDir", vec![SortDir::Asc, SortDir::Desc]);
    round_trip(
        "Sort",
        Sort {
            property: Property::Size,
            dir: SortDir::Asc,
        },
    );
    round_trip("View", view());
    round_trip("Cursor", Cursor("opaque".to_owned()));
    round_trip_each(
        "PageReq",
        vec![
            PageReq {
                after: None,
                limit: 25,
            },
            PageReq {
                after: Some(Cursor("opaque".to_owned())),
                limit: 0,
            },
        ],
    );
    round_trip("Query", query());
    round_trip(
        "Page",
        Page {
            items: vec![summary()],
            next: Some(Cursor("opaque".to_owned())),
        },
    );
}

#[test]
fn op_types_round_trip() {
    round_trip_each(
        "Target",
        vec![
            Target::Threads(vec![ThreadId::from_uuid(uuid(6))]),
            Target::Messages(vec![MessageId::from_uuid(uuid(5))]),
        ],
    );
    round_trip_each("Action", actions());
    round_trip_each(
        "OpKind",
        vec![
            OpKind::Archive,
            OpKind::Trash,
            OpKind::Restore,
            OpKind::Spam,
            OpKind::MarkRead,
            OpKind::MarkUnread,
            OpKind::Star,
            OpKind::Unstar,
            OpKind::AddLabel,
            OpKind::RemoveLabel,
            OpKind::Snooze,
            OpKind::Pin,
            OpKind::Reply,
            OpKind::ReplyAll,
            OpKind::Forward,
        ],
    );
    // `Patch` carries one of every `Change` variant.
    round_trip("Patch", patch());
    round_trip_each("Change", patch().changes);
}

#[test]
fn content_and_message_types_round_trip() {
    round_trip_each(
        "Address",
        vec![
            address("me@example.test"),
            Address {
                name: None,
                email: "bare@example.test".to_owned(),
            },
        ],
    );
    round_trip_each(
        "Body",
        vec![
            Body::Present {
                text: Some("hi".to_owned()),
                raw: BlobId::from_uuid(uuid(7)),
            },
            Body::Present {
                text: None,
                raw: BlobId::from_uuid(uuid(7)),
            },
        ],
    );
    round_trip_each(
        "Inline",
        vec![
            Inline::Attached,
            Inline::Embedded {
                cid: "cid@example.test".to_owned(),
            },
        ],
    );
    round_trip("Attachment", attachment());
    round_trip("Label", label());
    round_trip_each(
        "MessageKey",
        vec![
            MessageKey::Rfc("abc@example.test".to_owned()),
            MessageKey::Gmail(1),
            MessageKey::Synthetic([7u8; 32]),
        ],
    );
    round_trip("Message", message());
    round_trip("ThreadSummary", summary());
    round_trip("Thread", thread());
    round_trip("Ingest", ingest());
    round_trip_each("Fetched", ingest().messages);
}

#[test]
fn draft_types_round_trip() {
    round_trip(
        "PendingAttachment",
        PendingAttachment {
            name: "map.png".to_owned(),
            mime: "image/png".to_owned(),
            blob: BlobId::from_uuid(uuid(10)),
        },
    );
    round_trip_each(
        "SendState",
        vec![
            SendState::Editing,
            SendState::Queued,
            SendState::Sending,
            SendState::Scheduled { at: at(8) },
            SendState::Failed {
                reason: "550".to_owned(),
                retry: Retry::NeedsReauth,
            },
            SendState::Sent {
                at: at(7),
                message: Some(MessageId::from_uuid(uuid(5))),
            },
            SendState::Sent {
                at: at(7),
                message: None,
            },
        ],
    );
    round_trip_each("ReplyScope", vec![ReplyScope::Sender, ReplyScope::All]);
    round_trip("Draft", draft());
    round_trip("Template", template());
    round_trip_each(
        "Attendance",
        vec![
            Attendance::Accepted,
            Attendance::Tentative,
            Attendance::Declined,
        ],
    );
    round_trip(
        "InviteAnswer",
        InviteAnswer {
            message: MessageId::from_uuid(uuid(8)),
            attendance: Attendance::Declined,
            sequence: 0,
            comment: None,
            answered_at: at(9),
        },
    );
}

#[test]
fn state_types_round_trip() {
    round_trip_each("MailboxRole", MailboxRole::ALL.to_vec());
    round_trip_each(
        "MailboxSet",
        vec![
            MailboxSet::empty(),
            MailboxSet::only(MailboxRole::Inbox),
            MailboxRole::ALL.into_iter().collect::<MailboxSet>(),
        ],
    );
    round_trip_each("ReadState", vec![ReadState::Unread, ReadState::Read]);
    round_trip_each("Star", vec![Star::Unstarred, Star::Starred]);
    round_trip_each("Membership", vec![Membership::In, Membership::Out]);
    round_trip_each(
        "Attachments",
        vec![Attachments::None, Attachments::Present { count: 3 }],
    );
    round_trip_each("Pin", vec![Pin::Unpinned, Pin::Rank(-1)]);
    round_trip_each("Snooze", vec![Snooze::Inactive, Snooze::Until(at(9))]);
    round_trip_each(
        "LabelOrigin",
        vec![LabelOrigin::User, LabelOrigin::Provider],
    );
    round_trip_each("IsDefault", vec![IsDefault::Default, IsDefault::Alternate]);
    round_trip_each("Threading", vec![Threading::Threaded, Threading::Single]);
}

#[test]
fn id_types_round_trip() {
    round_trip("AccountId", account());
    round_trip("ThreadId", ThreadId::from_uuid(uuid(6)));
    round_trip("MessageId", MessageId::from_uuid(uuid(5)));
    round_trip("DraftId", DraftId::from_uuid(uuid(9)));
    round_trip("TemplateId", TemplateId::from_uuid(uuid(13)));
    round_trip("LabelId", LabelId::from_uuid(uuid(3)));
    round_trip("ViewId", ViewId::from_uuid(uuid(11)));
    round_trip("BlobId", BlobId::from_uuid(uuid(7)));
    round_trip("IdentityId", IdentityId::from_uuid(uuid(2)));
    round_trip("ChangeId", ChangeId::from_uuid(uuid(12)));
    round_trip("OutboxId", OutboxId::from_i64(i64::MAX));

    // `#[serde(transparent)]`: an id is a bare string, not `{"0": "..."}`. A wrapper object
    // would make every id column in SQLite unqueryable.
    assert_eq!(
        serde_json::to_value(account()).expect("serialize"),
        serde_json::json!("01010101-0101-0101-0101-010101010101")
    );
    assert_eq!(
        serde_json::to_value(OutboxId::from_i64(7)).expect("serialize"),
        serde_json::json!(7)
    );
}

// ---------------------------------------------------------------------------------------
// Group 2 — shape assertions. A round trip alone would accept any stable encoding.
// ---------------------------------------------------------------------------------------

#[test]
fn mailbox_set_persists_as_role_names() {
    let set = MailboxSet::only(MailboxRole::Inbox).with(MailboxRole::Sent);
    let json = serde_json::to_value(set).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!(["inbox", "sent"]),
        "MailboxSet must persist as an array of role names. The bitmask is an implementation \
         detail: a bare `5` in a SQLite column is unreadable, unqueryable, and silently \
         changes meaning the day a role is added to MailboxRole::ALL."
    );
    assert_eq!(
        serde_json::to_value(MailboxSet::empty()).expect("serialize"),
        serde_json::json!([])
    );
    // Order is `MailboxRole::ALL` order, not insertion order, so the encoding is canonical.
    let other: MailboxSet = [MailboxRole::Sent, MailboxRole::Inbox]
        .into_iter()
        .collect();
    assert_eq!(
        serde_json::to_value(other).expect("serialize"),
        serde_json::json!(["inbox", "sent"])
    );
    // And it reads names back, including a duplicate a hand-written row might carry.
    let parsed: MailboxSet =
        serde_json::from_str(r#"["sent","inbox","sent"]"#).expect("deserialize");
    assert_eq!(parsed, set);
}

#[test]
fn adjacently_tagged_enums_keep_their_tag() {
    // The `tag`/`content` pair is the persisted shape; internal tagging cannot represent
    // `RemoteRef::Pop(..)`-style newtype variants over non-maps, which is why §3 mandates it.
    assert_eq!(
        serde_json::to_value(RemoteRef::Pop {
            uidl: "UID-1".to_owned()
        })
        .expect("serialize"),
        serde_json::json!({"kind": "pop", "v": {"uidl": "UID-1"}})
    );
    assert_eq!(
        serde_json::to_value(Retry::Fatal("gone".to_owned())).expect("serialize"),
        serde_json::json!({"kind": "fatal", "v": "gone"})
    );
    assert_eq!(
        serde_json::to_value(ProtoOp::FetchCaps).expect("serialize"),
        serde_json::json!({"kind": "fetch_caps"})
    );
    // Fieldless enums are bare snake_case strings.
    assert_eq!(
        serde_json::to_value(Tls::StartTlsRequired).expect("serialize"),
        serde_json::json!("start_tls_required")
    );
}

#[test]
fn persisted_types_tolerate_unknown_fields() {
    // No `deny_unknown_fields` anywhere: a field added by a newer build must not turn a
    // downgrade into a hard startup failure (`CONVENTIONS.md` §3).
    let json = r#"{"account":"01010101-0101-0101-0101-010101010101",
                   "path":"INBOX","future_field":42}"#;
    let parsed: MailboxRef = serde_json::from_str(json).expect("unknown field must be ignored");
    assert_eq!(parsed.path, "INBOX");
}

#[test]
fn credential_debug_redacts_every_secret() {
    // A security property, not a formatting preference: a derived `Debug` puts the password
    // into every log line, panic message and error chain that ever formats this value.
    let password = Credential::Password("hunter2".to_owned());
    let shown = format!("{password:?}");
    assert!(
        !shown.contains("hunter2"),
        "Credential::Password Debug leaked the password: {shown}"
    );
    assert!(shown.contains("redacted"), "{shown}");

    let oauth = Credential::OAuth {
        access: "ya29.access-token".to_owned(),
        refresh: "1//refresh-token".to_owned(),
        expires_at: at(6),
    };
    let shown = format!("{oauth:?}");
    for secret in ["ya29.access-token", "1//refresh-token"] {
        assert!(
            !shown.contains(secret),
            "Credential::OAuth Debug leaked {secret}: {shown}"
        );
    }
    // The non-secret field survives, so the value is still worth logging.
    assert!(shown.contains("2026-01-06"), "{shown}");

    // Nesting must not reopen the hole: a `Debug` on a container that holds a `Credential`
    // formats it through the same hand-written impl.
    let nested = format!("{:?}", vec![password]);
    assert!(!nested.contains("hunter2"), "{nested}");
    let nested = format!("{:?}", Some(&oauth));
    assert!(!nested.contains("ya29.access-token"), "{nested}");
}

#[test]
fn presets_round_trip_and_carry_the_given_instant() {
    // A preset's output is persisted verbatim, so it is part of this suite.
    let gmail = mail_domain::presets::preset_for("someone@gmail.com", at(3)).expect("gmail preset");
    round_trip("preset/gmail/plan", gmail.plan);
    round_trip("preset/gmail/caps", gmail.expected_caps.clone());
    assert_eq!(gmail.expected_caps.observed_at, at(3));

    let manual = mail_domain::presets::manual_pop3(
        "s1234567@example.edu",
        &mail_domain::presets::ManualPop3 {
            pop3_host: "pop.example.edu".to_owned(),
            pop3_port: 995,
            smtp_host: "smtp.example.edu".to_owned(),
            smtp_port: 465,
            login: Some("s1234567".to_owned()),
        },
        at(3),
    );
    round_trip("preset/manual_pop3/plan", manual.plan);
    round_trip("preset/manual_pop3/caps", manual.expected_caps);
}

// ---------------------------------------------------------------------------------------
// Group 3 — frozen fixtures.
//
// Each file below was written by an earlier build and must keep deserializing forever.
// APPEND-ONLY: if one fails, that is the finding — a persisted type just changed shape and
// every row already on disk stops loading. Add a new fixture; never edit an old one.
// ---------------------------------------------------------------------------------------

macro_rules! fixtures {
    ($($file:literal => $ty:ty = $value:expr),* $(,)?) => {
        /// Every fixture still parses as the type it was written from.
        #[test]
        fn frozen_fixtures_still_deserialize() {
            let dir = fixture_dir();
            let mut expected: Vec<&str> = Vec::new();
            $({
                expected.push($file);
                let path = dir.join($file);
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let _parsed: $ty = serde_json::from_str(&text).unwrap_or_else(|e| panic!(
                    "{}: no longer deserializes as {}: {e}\n\
                     This is a schema break, not a test to fix. Do NOT edit the fixture.",
                    path.display(),
                    stringify!($ty),
                ));
            })*

            // A fixture nobody reads pins nothing, so the directory and the table must agree.
            let mut found: Vec<String> = std::fs::read_dir(&dir)
                .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                .map(|entry| entry.expect("dir entry").file_name().to_string_lossy().into_owned())
                .filter(|name| name.ends_with(".json"))
                .collect();
            found.sort();
            let mut expected: Vec<String> = expected.into_iter().map(str::to_owned).collect();
            expected.sort();
            assert_eq!(found, expected, "every fixture must be listed in `fixtures!`");
        }

        /// Writes fixtures that do not exist yet, and **never** touches one that does.
        ///
        /// Run with `cargo test -p mail-domain --test serde -- --ignored write_fixtures`
        /// after adding a new entry to `fixtures!`. Refusing to overwrite is what keeps the
        /// corpus append-only even when it is generated.
        #[test]
        #[ignore = "writes new fixture files; run by hand after adding an entry"]
        fn write_fixtures() {
            let dir = fixture_dir();
            std::fs::create_dir_all(&dir).expect("create fixtures dir");
            $({
                let path = dir.join($file);
                if path.exists() {
                    println!("keep  {}", path.display());
                } else {
                    let value: $ty = $value;
                    let json = serde_json::to_string_pretty(&value).expect("serialize fixture");
                    std::fs::write(&path, format!("{json}\n")).expect("write fixture");
                    println!("write {}", path.display());
                }
            })*
        }
    };
}

fn fixture_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fixtures! {
    "account_plan_gmail.json" => AccountPlan = presets::preset_for("someone@gmail.com", at(3))
        .expect("gmail preset").plan,
    // A Microsoft 365 account that sends through Graph rather than SMTP.
    "account_plan_graph.json" => AccountPlan = presets::send_through_graph(
        presets::microsoft_preset("me@contoso.example", at(3))
    ).plan,
    // A POP3 account that logs in with the local part of its address.
    "account_plan_local_part.json" => AccountPlan = AccountPlan {
        address: "s1234567@example.edu".to_owned(),
        incoming: Incoming::Pop3 {
            host: "pop.example.edu".to_owned(),
            port: 995,
            tls: Tls::Implicit,
            leave: LeaveOnServer::Keep,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.edu".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::LocalPart,
            sasl: vec![SaslMech::Login, SaslMech::Plain],
        },
        identities: Vec::new(),
    },
    "account_plan_manual.json" => AccountPlan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Pop3 {
            host: "pop.example.test".to_owned(),
            port: 110,
            tls: Tls::StartTlsRequired,
            leave: LeaveOnServer::DeleteAfterFetch,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 587,
            tls: Tls::StartTlsRequired,
        },
        auth: AuthPlan::Password {
            username: Username::Literal("login-name".to_owned()),
            sasl: vec![SaslMech::Plain],
        },
        identities: vec![identity()],
    },
    "account_caps_imap.json" => AccountCaps = caps(),
    "account_caps_pop3.json" => AccountCaps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll { every: Duration::from_secs(300) },
        archive: ArchiveMeans::MoveToFolder("Archive".to_owned()),
        folders: FolderRoles::default(),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget::default(),
        observed_at: at(3),
    },
    "identity.json" => Identity = identity(),
    "secret_key.json" => SecretKey = SecretKey {
        account: account(),
        purpose: SecretPurpose::OAuthRefresh,
    },
    "credentials.json" => Vec<Credential> = vec![
        Credential::Password("hunter2".to_owned()),
        Credential::OAuth {
            access: "ya29.access".to_owned(),
            refresh: "1//refresh".to_owned(),
            expires_at: at(6),
        },
    ],
    "remote_refs.json" => Vec<RemoteRef> = vec![
        imap_ref(),
        RemoteRef::Pop { uidl: "UID-1".to_owned() },
    ],
    "mailbox_ref.json" => MailboxRef = mailbox_ref(),
    "sync_cursors.json" => Vec<SyncCursor> = vec![imap_cursor(), SyncCursor::Pop],
    "uid_validity.json" => Vec<UidValidity> = vec![UidValidity::Same, UidValidity::Reset],
    "fetch_since.json" => Vec<FetchSince> = vec![
        FetchSince::Beginning,
        FetchSince::After { cursor: imap_cursor() },
    ],
    // Regenerated once, when `ProtoOp::Submit` gained `mail_from` and `rcpt_to`. The corpus is
    // append-only precisely so that a break like this cannot pass unnoticed, so the argument
    // for retiring the old file is recorded rather than assumed: the only writer of a stored
    // `ProtoOp` is `SqliteStore::queue`, fed by `resolve_intent`, which until that same commit
    // matched `SetFlags | SetMailbox | SetLabels` and nothing else. No `Submit` had ever been
    // written to any outbox, so no row of the old shape exists to be read back. The retired
    // content remains in git history. See FINDINGS F37.
    "proto_ops.json" => Vec<ProtoOp> = proto_ops(),
    "retries.json" => Vec<Retry> = vec![
        Retry::Now,
        Retry::After(Duration::from_secs(90)),
        Retry::NeedsReauth,
        Retry::Fatal("message gone".to_owned()),
    ],
    "filters.json" => Vec<Filter> = filters(),
    "view.json" => View = view(),
    "query.json" => Query = query(),
    "patch.json" => Patch = patch(),
    "actions.json" => Vec<Action> = actions(),
    "ingest.json" => Ingest = ingest(),
    "message.json" => Message = message(),
    "thread.json" => Thread = thread(),
    "label.json" => Label = label(),
    "draft.json" => Draft = draft(),
    // Written when drafts gained `receipt`. `draft.json` above predates the field and must keep
    // loading as `Unrequested`; this one pins the field's own spelling.
    "draft_receipt.json" => Draft = Draft { receipt: ReceiptRequest::Requested, ..draft() },
    "receipt_answers.json" => Vec<ReceiptAnswer> = vec![ReceiptAnswer::Sent, ReceiptAnswer::Declined],
    // The keyword op, added after `proto_ops.json` was frozen.
    "proto_ops_keywords.json" => Vec<ProtoOp> = vec![ProtoOp::AddKeyword {
        remotes: vec![imap_ref()],
        keyword: Keyword::MdnSent,
    }],
    "send_states.json" => Vec<SendState> = vec![
        SendState::Editing,
        SendState::Queued,
        SendState::Sending,
        SendState::Failed { reason: "550 rejected".to_owned(), retry: Retry::NeedsReauth },
        SendState::Sent { at: at(7), message: Some(MessageId::from_uuid(uuid(5))) },
        SendState::Sent { at: at(7), message: None },
    ],
    // Send later (`plan.md` 10.8): a send held in the outbox until a time the user chose.
    "send_states_scheduled.json" => Vec<SendState> = vec![SendState::Scheduled { at: at(8) }],
    "template.json" => Template = template(),
    "mailbox_sets.json" => Vec<MailboxSet> = vec![
        MailboxSet::empty(),
        MailboxSet::only(MailboxRole::Inbox),
        MailboxRole::ALL.into_iter().collect(),
    ],
    "folders.json" => Vec<Folder> = folders(),
    "proto_ops_folders.json" => Vec<ProtoOp> = folder_ops(),
    "patch_folders.json" => Patch = folder_patch(),
    // `Append` carried a `MailboxRole` until import needed flags no role implies; the old
    // shape is still in `proto_ops.json` and must keep reading. This is the new one.
    "proto_ops_append.json" => Vec<ProtoOp> = vec![ProtoOp::Append {
        mailbox: mailbox_ref(),
        flags: vec![SystemFlag::Seen, SystemFlag::Flagged],
        date: Some(at(4)),
        raw: BlobId::from_uuid(uuid(7)),
    }],
    // The local-only account imported mail lands in.
    "account_plan_local.json" => AccountPlan = presets::local_folders(at(3)).plan,
    // Rules and vacation replies (`plan.md` 10.11), and the op that files into a folder.
    "rules.json" => Vec<Rule> = rules(),
    "vacation.json" => Vacation = vacation(),
    "proto_ops_file.json" => Vec<ProtoOp> = vec![ProtoOp::File {
        remotes: vec![imap_ref()],
        folder: "Money/Bills".to_owned(),
    }],
    "message_keys.json" => Vec<MessageKey> = vec![
        MessageKey::Rfc("abc@example.test".to_owned()),
        MessageKey::Gmail(1),
        MessageKey::Synthetic([7u8; 32]),
    ],
    // Calendar invitations (`plan.md` 10.12): the answer the store keeps per message.
    "attendances.json" => Vec<Attendance> = vec![
        Attendance::Accepted,
        Attendance::Tentative,
        Attendance::Declined,
    ],
    "invite_answer.json" => InviteAnswer = InviteAnswer {
        message: MessageId::from_uuid(uuid(8)),
        attendance: Attendance::Tentative,
        sequence: 2,
        comment: Some("might be late".to_owned()),
        answered_at: at(9),
    },
    // Microsoft Graph as an incoming protocol (`plan.md` 10.18).
    "graph_incoming.json" => Vec<Incoming> = vec![Incoming::Graph],
    "graph_remote_refs.json" => Vec<RemoteRef> = vec![graph_ref()],
    "graph_sync_cursors.json" => Vec<SyncCursor> = vec![graph_cursor()],
}

/// `RemoteIntent` is not persisted today — `Store::enqueue` resolves it to a `ProtoOp` before
/// anything reaches SQLite — but it is `Serialize`, so its shape is pinned here to keep the
/// adjacent-tagging convention honest if that ever changes.
#[test]
fn remote_intent_round_trips() {
    let m = MessageId::generate();
    let l = LabelId::generate();
    for value in [
        RemoteIntent::SetFlags {
            messages: vec![m],
            read: Some(ReadState::Read),
            star: None,
        },
        RemoteIntent::SetMailbox {
            messages: vec![m],
            role: MailboxRole::Archive,
        },
        RemoteIntent::SetLabels {
            messages: vec![m],
            add: vec![l],
            remove: Vec::new(),
        },
        RemoteIntent::AddKeyword {
            messages: vec![m],
            keyword: Keyword::MdnSent,
        },
        RemoteIntent::File {
            messages: vec![m],
            label: l,
        },
    ] {
        let json = serde_json::to_value(&value).expect("serialize");
        assert!(
            json.get("kind").is_some() && json.get("v").is_some(),
            "adjacent tagging: {json}"
        );
        let back: RemoteIntent = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, value);
    }
}

/// Grouping is a different axis from columns, and the type has to say so.
///
/// `group_by: Option<Property>` could not express "group by read state" or "group by label" —
/// two of the groupings a mail client most obviously wants — because the field was typed as the
/// thing we had rather than the thing we needed.
#[test]
fn group_key_round_trips_every_variant() {
    for value in [
        GroupKey::Property(Property::Date),
        GroupKey::Read,
        GroupKey::Star,
        GroupKey::Label(LabelId::generate()),
        GroupKey::Mailbox,
    ] {
        let json = serde_json::to_value(&value).expect("serialize");
        assert!(
            json.get("kind").is_some(),
            "adjacent tagging, like every other persisted enum: {json}"
        );
        let back: GroupKey = serde_json::from_value(json).expect("deserialize");
        assert_eq!(back, value);
    }
}

#[test]
fn folder_types_round_trip() {
    round_trip_each("Folder", folders());
    round_trip_each("ProtoOp::Folder", folder_ops());
    round_trip("folder Patch", folder_patch());
    round_trip(
        "RemoteIntent::Folder",
        RemoteIntent::Folder(FolderWork::Create {
            path: "Receipts".to_owned(),
        }),
    );
}

/// A draft written before receipts existed did not ask for one, and still loads saying so.
#[test]
fn a_draft_from_before_receipts_asks_for_none() {
    let text = std::fs::read_to_string(fixture_dir().join("draft.json")).expect("fixture");
    assert!(
        !text.contains("receipt"),
        "the fixture must predate the field"
    );
    let draft: Draft = serde_json::from_str(&text).expect("still deserializes");
    assert_eq!(draft.receipt, ReceiptRequest::Unrequested);
}

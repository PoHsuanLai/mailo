use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

pub(in crate::ui) const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

/// A store with one account, one message and one draft, so the panes have something to draw.
/// What was actually observed against the user's Gmail account.
pub(in crate::ui) fn gmail_caps() -> AccountCaps {
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
        pipelining: Supported::Yes,
        connections: ConnectionBudget::default(),
        observed_at: chrono::Utc::now(),
    }
}

pub(in crate::ui) fn seeded() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let identity = IdentityId::generate();
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
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [identity.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
        // Capabilities, because `account add` always writes them and a fixture without them
        // is a state the application cannot reach — F133's lesson. Gmail's own, as recorded
        // against the real account: archiving means dropping the inbox label, and labels
        // are the server's. This is what decides whether an operation performed in the
        // window has a server half at all.
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at)
             VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![
                ACCOUNT.to_string(),
                serde_json::to_string(&gmail_caps()).unwrap()
            ],
        )
        .unwrap();
    }

    let raw = store
        .blobs()
        .put(
            &store.connection(),
            b"From: ada@example.test\r\nSubject: hi\r\n\r\nbody\r\n",
        )
        .unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc("m1@example.test".to_owned()),
        date: chrono::Utc::now(),
        from: Address {
            name: Some("Ada".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "hi".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("m1@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("body".to_owned()),
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

    store
        .apply(
            ACCOUNT,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(Draft {
                    id: DraftId::generate(),
                    account: ACCOUNT,
                    identity,
                    to: vec![Address {
                        name: None,
                        email: "ada@example.test".to_owned(),
                    }],
                    cc: vec![],
                    bcc: vec![],
                    subject: "Re: hi".to_owned(),
                    in_reply_to: None,
                    forward_of: None,
                    text: "typing".to_owned(),
                    html: None,
                    attachments: vec![],
                    state: SendState::Editing,
                    updated: chrono::Utc::now(),
                }))],
            },
        )
        .unwrap();
    (store, dir)
}

/// A mailbox shaped like a real one, for looking at the layout rather than exercising it.
///
/// The fixture above holds one message from "Ada" with the subject "hi", which is a size
/// nothing can be wrong at. Real mail has long subjects, long display names, CJK text that
/// has no spaces to break at, and enough rows to fill the pane.
pub(in crate::ui) fn realistic() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let (store, dir) = seeded();
    let rows: [(&str, &str, &str, bool); 6] = [
        (
            "校園資訊網路中心",
            "noreply@example.edu",
            "【重要】校園郵件信箱系統維護通知：本週六 02:00 至 06:00 暫停服務",
            false,
        ),
        (
            "GitHub",
            "notifications@github.test",
            "[rust-lang/rust] Re: Tracking issue for `let`-chains stabilisation (#53667)",
            false,
        ),
        (
            "Dr. Wolfgang Amadeus Pemberton-Featherstonehaugh",
            "w.pemberton@example.test",
            "Re: Re: Re: Fwd: supervision meeting — moved to Thursday",
            true,
        ),
        ("Mum", "mum@example.test", "dinner?", false),
        (
            "Stripe",
            "receipts@stripe.test",
            "Your receipt from Anthropic, PBC #2847-1932",
            true,
        ),
        (
            "arXiv cs.PL",
            "no-reply@arxiv.test",
            "New submissions in cs.PL: 14 papers",
            true,
        ),
    ];
    for (n, (name, email, subject, read)) in rows.iter().enumerate() {
        // Real bytes for the first one, so the reader has an HTML part to sanitize and
        // draw rather than falling back to text for every message in the fixture.
        let bytes = if n == 1 {
            format!(
                "From: {name} <{email}>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
                 Content-Type: text/html; charset=utf-8\r\n\r\n\
                 <h2>Tracking issue for <code>let</code>-chains</h2>\
                 <p>There are <b>3</b> new comments on this issue.</p>\
                 <blockquote>Stabilisation report is up; please review.</blockquote>\
                 <p><a href=\"https://example.test/issues/53667\">View it on GitHub</a></p>\r\n"
            )
            .into_bytes()
        } else if n == 4 {
            // A receipt with a tracking pixel, which is what a receipt actually is. This is
            // the one message in the fixture that has something to load.
            format!(
                "From: {name} <{email}>\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
                 Content-Type: text/html; charset=utf-8\r\n\r\n\
                 <p>Thanks for your payment.</p>\
                 <img src=\"https://track.stripe.test/open/2847-1932.gif\" width=\"1\" \
                 height=\"1\">\r\n"
            )
            .into_bytes()
        } else {
            subject.as_bytes().to_vec()
        };
        let raw = store.blobs().put(&store.connection(), &bytes).unwrap();
        let message = Message {
            id: MessageId::generate(),
            thread: ThreadId::generate(),
            account: ACCOUNT,
            key: MessageKey::Rfc(format!("real{n}@example.test")),
            // Spread over days, so the date column has more than one shape in it.
            date: chrono::Utc::now() - chrono::TimeDelta::try_hours(n as i64 * 19).unwrap(),
            from: Address {
                name: Some((*name).to_owned()),
                email: (*email).to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: (*subject).to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("real{n}@example.test")),
            read: if *read {
                ReadState::Read
            } else {
                ReadState::Unread
            },
            star: Star::Unstarred,
            mailbox: MailboxRole::Inbox,
            labels: vec![],
            body: Body::Present {
                text: Some((*subject).to_owned()),
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
                            uidl: format!("real{n}"),
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
    }
    (store, dir)
}

/// Write the shell to `target/shell.html`, stylesheet and all, so it can be looked at.
///
/// `#[ignore]`d because it is a tool, not an assertion. It exists because the shell's layout
/// had never been seen: the desktop window is a WebView surface owned by the compositor, and
/// under rootless XWayland an X11 grab of it returns `BadMatch`, so there was no way from
/// here to a picture of it. This produces the same markup and the same stylesheet as a
/// static page, which any browser will render and screenshot headlessly:
///
/// ```text
/// cargo test -p mail-app --bins -- --ignored render_the_shell_to_a_file
/// google-chrome --headless --screenshot=shell.png --window-size=1200,800 target/shell.html
/// ```
///
/// It is the markup and the CSS, not the running application: nothing here clicks, and a
/// WebView is not a browser. It is still the difference between looking and guessing.
/// A database with nothing in it: no account, no mail, no drafts.
///
/// The first thing anyone sees, and the one state the fixtures never covered because every
/// one of them seeds an account before rendering.
pub(in crate::ui) fn empty() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    (store, dir)
}

pub(in crate::ui) fn inbox_query() -> Query {
    Query {
        filter: Filter::InMailbox(MailboxRole::Inbox),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    }
}

//! The Work Space from the frame mockup, as mail the store can actually hold.
//!
//! Three accounts (Google, Microsoft, Fastmail hosts), the same senders, subjects
//! and snippets, and the labels `spec` and `release`. Nothing here is a real
//! institution or a real person's mailbox.

use super::store::gmail_caps;
use crate::appearance::WindowDirs;
use crate::space::{self, Pinned, Scope, Space, Spaces};
use crate::today::{self, Today};
use crate::view::{Motion, Theme};
use chrono::Datelike;
use ds::{CardAccent, Dot, Grain, SpaceLook};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::collections::BTreeMap;
use std::sync::Arc;

pub(in crate::ui) struct Work {
    pub store: Arc<SqliteStore>,
    pub root: tempfile::TempDir,
    pub dirs: WindowDirs,
    pub dana: ThreadId,
    pub sam: ThreadId,
}

const GOOGLE: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const MICROSOFT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));
const FASTMAIL: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b3"));

struct Mail {
    account: AccountId,
    name: &'static str,
    email: &'static str,
    subject: &'static str,
    body: &'static str,
    days: i64,
    hour: u32,
    min: u32,
    read: bool,
    star: bool,
    labels: &'static [&'static str],
    file: bool,
}

/// The Work Space, its file, and a state directory for Today.
pub(in crate::ui) fn work() -> Work {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("config");
    let state = root.path().join("state");
    let store = Arc::new(SqliteStore::in_memory(root.path()).unwrap());
    let monday = {
        let today = chrono::Local::now().weekday().num_days_from_monday() as i64;
        if today == 0 { 7 } else { today }
    };
    let mail = [
        Mail {
            account: GOOGLE,
            name: "Dana Okafor",
            email: "dana@example.com",
            subject: "Re: UIDL stability across a UIDVALIDITY change",
            body: "Treat UIDL as a per-session handle, and dedupe on something derived from the message itself.",
            days: 0,
            hour: 9,
            min: 41,
            read: false,
            star: false,
            labels: &["spec"],
            file: false,
        },
        Mail {
            account: GOOGLE,
            name: "Release list",
            email: "release@example.com",
            subject: "0.9 cut on Thursday: what is still open",
            body: "Three blockers left, two of them in the outbox retry path.",
            days: 0,
            hour: 8,
            min: 2,
            read: false,
            star: false,
            labels: &["release"],
            file: false,
        },
        Mail {
            account: MICROSOFT,
            name: "Google Workspace",
            email: "no-reply@g00gle-security.xyz",
            subject: "Unusual sign-in attempt blocked",
            body: "We blocked a sign-in to your account. Verify it was you within 24 hours to keep access.",
            days: 0,
            hour: 7,
            min: 12,
            read: false,
            star: false,
            labels: &[],
            file: false,
        },
        Mail {
            account: MICROSOFT,
            name: "Build bot",
            email: "ci@example.com",
            subject: "main is green again",
            body: "The flaky reader test passed 50 of 50 runs after the cache key change.",
            days: 1,
            hour: 18,
            min: 5,
            read: false,
            star: false,
            labels: &[],
            file: false,
        },
        Mail {
            account: GOOGLE,
            name: "Sam Lindqvist",
            email: "sam@example.com",
            subject: "Notes from the sync review",
            body: "Keyset cursors, not offsets. The list must not jump when mail arrives.",
            days: monday,
            hour: 16,
            min: 40,
            read: true,
            star: true,
            labels: &["spec"],
            file: false,
        },
        Mail {
            account: FASTMAIL,
            name: "This Week in Rust",
            email: "newsletter@thisweekinrust.example",
            subject: "This Week in Rust 566",
            body: "Hello and welcome to another issue of This Week in Rust! Crate of the week, 412 PRs merged.",
            days: monday,
            hour: 11,
            min: 2,
            read: false,
            star: false,
            labels: &["rust"],
            file: false,
        },
        Mail {
            account: FASTMAIL,
            name: "Priya Raman",
            email: "priya@rust-users.example",
            subject: "[rust-users] Re: matching FETCH responses to UIDs",
            body: "Don't zip them. The server may answer in any order, and it may send unsolicited FETCHes.",
            days: monday,
            hour: 10,
            min: 15,
            read: false,
            star: false,
            labels: &[],
            file: false,
        },
        Mail {
            account: GOOGLE,
            name: "Sam Lindqvist",
            email: "sam@example.com",
            subject: "Re: Re: Keyset cursors, not offsets",
            body: "Agreed on the cursor. One wrinkle: two threads with the same timestamp.",
            days: monday,
            hour: 9,
            min: 15,
            read: false,
            star: false,
            labels: &["spec"],
            file: false,
        },
        Mail {
            account: GOOGLE,
            name: "Dana Okafor",
            email: "dana@example.com",
            subject: "Invitation: Design review, Thursday 14:00",
            body: "Thursday 25 September, 14:00-15:00. Room 4 and a video link.",
            days: monday,
            hour: 8,
            min: 2,
            read: false,
            star: false,
            labels: &[],
            file: true,
        },
        Mail {
            account: GOOGLE,
            name: "Calendar",
            email: "calendar@example.com",
            subject: "Accepted: Design review, Thursday 14:00",
            body: "Dana Okafor accepted this invitation.",
            days: monday,
            hour: 7,
            min: 30,
            read: true,
            star: false,
            labels: &[],
            file: false,
        },
    ];
    for (id, address, host, graph) in [
        (GOOGLE, "poh@acme.example", "imap.gmail.com", false),
        (
            MICROSOFT,
            "p.lai@corp.example",
            "outlook.office365.com",
            true,
        ),
        (
            FASTMAIL,
            "lists@fastmail.example",
            "imap.fastmail.com",
            false,
        ),
    ] {
        insert_account(&store, id, address, host, graph);
    }
    let mut dana = None;
    let mut sam = None;
    for account in [GOOGLE, MICROSOFT, FASTMAIL] {
        let batch: Vec<&Mail> = mail.iter().filter(|item| item.account == account).collect();
        let built = build(&store, &batch);
        dana = dana.or(built.dana);
        sam = sam.or(built.sam);
        ingest(&store, account, built.messages, built.names);
    }
    let mut colors = BTreeMap::new();
    colors.insert(GOOGLE, "#5B4FC4".to_owned());
    colors.insert(MICROSOFT, "#2F7F6E".to_owned());
    colors.insert(FASTMAIL, "#B0662E".to_owned());
    let spaces = Spaces {
        current: 0,
        recall: BTreeMap::new(),
        spaces: vec![Space {
            name: "Work".to_owned(),
            look: SpaceLook {
                dots: vec![
                    Dot {
                        hue: 268.0,
                        chroma: 0.72,
                    },
                    Dot {
                        hue: 318.0,
                        chroma: 0.55,
                    },
                ],
                grain: Grain(35),
                theme: Theme::System,
                card_accent: CardAccent::SpaceHue,
            },
            motion: Motion::Standard,
            scope: Scope::All,
            pins: vec![
                Pinned::Person {
                    name: "Dana Okafor".to_owned(),
                    email: "dana@example.com".to_owned(),
                },
                Pinned::Person {
                    name: "Release list".to_owned(),
                    email: "release@example.com".to_owned(),
                },
                Pinned::Search {
                    name: "Unread".to_owned(),
                    query: "is:unread".to_owned(),
                },
                Pinned::Person {
                    name: "Build bot".to_owned(),
                    email: "ci@example.com".to_owned(),
                },
            ],
            colors,
        }],
    };
    space::save(&config, &spaces).unwrap();
    let mut today = Today::default();
    today.opened(
        0,
        sam.expect("sam"),
        chrono::Utc::now() - chrono::TimeDelta::minutes(30),
    );
    today.opened(0, dana.expect("dana"), chrono::Utc::now());
    today::save(&state, &today).unwrap();
    Work {
        store,
        dana: dana.expect("dana"),
        sam: sam.expect("sam"),
        dirs: WindowDirs { config, state },
        root,
    }
}

fn insert_account(store: &SqliteStore, id: AccountId, address: &str, host: &str, graph: bool) {
    let plan = AccountPlan {
        address: address.to_owned(),
        incoming: Incoming::Imap {
            host: host.to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: if graph {
            Outgoing::Graph
        } else {
            Outgoing::Smtp {
                host: host.replacen("imap", "smtp", 1),
                port: 465,
                tls: Tls::Implicit,
            }
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    let db = store.connection();
    db.execute(
        "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, datetime('now'))",
        rusqlite::params![
            id.to_string(),
            address,
            serde_json::to_string(&plan).unwrap()
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, datetime('now'))",
        rusqlite::params![
            id.to_string(),
            serde_json::to_string(&gmail_caps()).unwrap()
        ],
    )
    .unwrap();
}

fn at(days: i64, hour: u32, min: u32) -> chrono::DateTime<chrono::Utc> {
    let day = chrono::Local::now().date_naive() - chrono::TimeDelta::days(days);
    let naive = day.and_hms_opt(hour, min, 0).expect("that hour exists");
    naive
        .and_local_timezone(chrono::Local)
        .earliest()
        .expect("that local time exists")
        .with_timezone(&chrono::Utc)
}

fn ensure_label(store: &SqliteStore, account: AccountId, name: &str) -> LabelId {
    if let Ok(labels) = store.labels(account)
        && let Some(found) = labels.into_iter().find(|label| label.name == name)
    {
        return found.id;
    }
    let id = LabelId::generate();
    store
        .apply(
            account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::LabelUpsert(Label {
                    id,
                    account,
                    name: name.to_owned(),
                    color: None,
                    origin: LabelOrigin::User,
                })],
            },
        )
        .unwrap();
    id
}

struct BuiltMail {
    messages: Vec<Fetched>,
    names: Vec<(RemoteRef, Vec<String>)>,
    dana: Option<ThreadId>,
    sam: Option<ThreadId>,
}

fn build(store: &SqliteStore, batch: &[&Mail]) -> BuiltMail {
    let mut messages = Vec::new();
    let mut names = Vec::new();
    let mut dana = None;
    let mut sam = None;
    for (n, item) in batch.iter().enumerate() {
        let bytes = format!(
            "From: {} <{}>\r\nSubject: {}\r\n\r\n{}\r\n",
            item.name, item.email, item.subject, item.body
        );
        let raw = store
            .blobs()
            .put(&store.connection(), bytes.as_bytes())
            .unwrap();
        let remote = RemoteRef::Pop {
            uidl: format!("w-{n}-{}", item.account),
        };
        let thread = ThreadId::generate();
        if item.subject.starts_with("Re: UIDL") {
            dana = Some(thread);
        }
        if item.subject == "Notes from the sync review" {
            sam = Some(thread);
        }
        let message = Message {
            id: MessageId::generate(),
            thread,
            account: item.account,
            key: MessageKey::Rfc(format!("w-{n}-{}@example.test", item.account)),
            date: at(item.days, item.hour, item.min),
            from: Address {
                name: Some(item.name.to_owned()),
                email: item.email.to_owned(),
            },
            reply_to: vec![],
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: item.subject.to_owned(),
            in_reply_to: None,
            references: vec![],
            rfc_message_id: Some(format!("w-{n}@example.test")),
            read: if item.read {
                ReadState::Read
            } else {
                ReadState::Unread
            },
            star: if item.star {
                Star::Starred
            } else {
                Star::Unstarred
            },
            mailbox: MailboxRole::Inbox,
            labels: item
                .labels
                .iter()
                .map(|name| ensure_label(store, item.account, name))
                .collect(),
            body: Body::Present {
                text: Some(item.body.to_owned()),
                raw,
            },
            attachments: if item.file {
                vec![Attachment {
                    name: "invite.ics".to_owned(),
                    mime: "text/calendar".to_owned(),
                    size: 2048,
                    content: PartContent::Remote {
                        section: "2".to_owned(),
                    },
                    inline: Inline::Attached,
                }]
            } else {
                vec![]
            },
        };
        names.push((
            remote.clone(),
            item.labels
                .iter()
                .map(|label| (*label).to_owned())
                .collect(),
        ));
        messages.push(Fetched {
            remote,
            key: message.key.clone(),
            raw,
            message,
        });
    }
    BuiltMail {
        messages,
        names,
        dana,
        sam,
    }
}

fn ingest(
    store: &SqliteStore,
    account: AccountId,
    messages: Vec<Fetched>,
    names: Vec<(RemoteRef, Vec<String>)>,
) {
    store
        .ingest(
            account,
            Ingest {
                mailbox: MailboxRef {
                    account,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages,
                flags: vec![],
                labels: vec![],
                label_names: names,
                gone: vec![],
            },
        )
        .unwrap();
}

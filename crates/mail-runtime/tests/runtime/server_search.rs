//! Searching an IMAP server over a real socket: the engine asks, keeps what is new as headers
//! through the sync's own header path, marks it as found there, and brings nothing twice.
//!
//! The server is a listener in this process that plays Gmail's shape — one `\All` mailbox — with
//! and without `ESEARCH`. It answers `UID SEARCH` from what it was asked for, and refuses any
//! fetch of a body, so a search that fetched one would fail here.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::{AccountEngine, AccountSecrets, Arrival, Searched};
use mail_store::{SqliteStore, Store};
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
const PASSWORD: &str = "s3cr3t";
const ALL_MAIL: &str = "[Gmail]/All Mail";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_790_000_000, 0).unwrap()
}

/// What All Mail holds: UID, and the message. 503 is also in the inbox, where a sync found it.
fn all_mail() -> Vec<(u32, String)> {
    let message = |id: &str, from: &str, subject: &str, date: &str| {
        format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\nDate: {date}\r\n\
             Message-ID: <{id}@example.test>\r\n\r\nThe body of {subject}, never fetched.\r\n"
        )
    };
    vec![
        (
            501,
            message(
                "old-lunch",
                "Ada <ada@example.test>",
                "lunch in 2019",
                "Tue, 5 Mar 2019 12:00:00 +0000",
            ),
        ),
        (
            502,
            message(
                "budget",
                "Bob <bob@example.test>",
                "the budget",
                "Wed, 6 Mar 2019 12:00:00 +0000",
            ),
        ),
        (
            503,
            message(
                "new-lunch",
                "Ada <ada@example.test>",
                "lunch on friday",
                "Mon, 21 Sep 2026 12:00:00 +0000",
            ),
        ),
    ]
}

type Seen = Arc<Mutex<Vec<String>>>;

async fn serve(esearch: bool) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let recording = seen.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(session(sock, recording.clone(), esearch));
        }
    });
    (port, seen)
}

async fn session(mut sock: tokio::net::TcpStream, seen: Seen, esearch: bool) {
    if sock
        .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        .await
        .is_err()
    {
        return;
    }
    let mail = all_mail();
    let mut buf = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let read = match sock.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        buf.extend_from_slice(&chunk[..read]);
        while let Some(at) = buf.windows(2).position(|w| w == b"\r\n") {
            let line = String::from_utf8_lossy(&buf[..at]).to_string();
            buf.drain(..at + 2);
            let (tag, rest) = line.split_once(' ').unwrap_or(("*", ""));
            seen.lock().unwrap().push(rest.to_owned());
            let upper = rest.to_ascii_uppercase();
            let reply = if upper.starts_with("LOGIN") {
                format!("{tag} OK logged in\r\n")
            } else if upper.starts_with("CAPABILITY") {
                let extra = if esearch { " ESEARCH" } else { "" };
                format!("* CAPABILITY IMAP4rev1 X-GM-EXT-1{extra}\r\n{tag} OK done\r\n")
            } else if upper.starts_with("EXAMINE") {
                format!("* 3 EXISTS\r\n* OK [UIDVALIDITY 11] ok\r\n{tag} OK [READ-ONLY] done\r\n")
            } else if upper.starts_with("UID SEARCH") {
                // The words asked for, each in some field of the message.
                let wanted: Vec<u32> = mail
                    .iter()
                    .filter(|(_, raw)| {
                        quoted(rest)
                            .iter()
                            .all(|w| raw.to_lowercase().contains(&w.to_lowercase()))
                    })
                    .map(|(uid, _)| *uid)
                    .collect();
                let list: Vec<String> = wanted.iter().map(u32::to_string).collect();
                if esearch && upper.contains("RETURN (COUNT ALL)") {
                    let all = if list.is_empty() {
                        String::new()
                    } else {
                        format!(" ALL {}", list.join(","))
                    };
                    format!(
                        "* ESEARCH (TAG \"{tag}\") UID COUNT {}{all}\r\n{tag} OK done\r\n",
                        list.len()
                    )
                } else {
                    format!("* SEARCH {}\r\n{tag} OK done\r\n", list.join(" "))
                }
            } else if upper.starts_with("UID FETCH") && upper.contains("BODY.PEEK[HEADER]") {
                let set = rest.split_whitespace().nth(2).unwrap_or("");
                let mut out = String::new();
                for uid in set.split(',').filter_map(|u| u.parse::<u32>().ok()) {
                    let Some((_, raw)) = mail.iter().find(|(u, _)| *u == uid) else {
                        continue;
                    };
                    let head = raw.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n\r\n";
                    out.push_str(&format!(
                        "* 1 FETCH (UID {uid} FLAGS (\\Seen) BODY[HEADER] {{{}}}\r\n{head})\r\n",
                        head.len()
                    ));
                }
                out + &format!("{tag} OK done\r\n")
            } else if upper.starts_with("LOGOUT") {
                format!("* BYE\r\n{tag} OK bye\r\n")
            } else {
                // A body, or anything else a search has no business asking for.
                format!("{tag} BAD not for a search\r\n")
            };
            if sock.write_all(reply.as_bytes()).await.is_err() {
                return;
            }
        }
    }
}

/// The distinct quoted strings in a command line.
fn quoted(line: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (n, part) in line.split('"').enumerate() {
        if n % 2 == 1 && !out.iter().any(|o| o == part) {
            out.push(part.to_owned());
        }
    }
    out
}

fn plan(port: u16) -> AccountPlan {
    AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: 1,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

fn caps() -> AccountCaps {
    AccountCaps {
        labels: ServerLabels::Supported,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::DropInbox,
        folders: FolderRoles(vec![(ALL_MAIL.to_owned(), MailboxRole::Archive)]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Supported,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    }
}

struct Fixture {
    store: Arc<SqliteStore>,
    engine: AccountEngine<ImapBackend>,
    _dir: tempfile::TempDir,
}

fn fixture(port: u16) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
    store
        .put_folders(
            acct_account(),
            vec![
                Folder {
                    account: acct_account(),
                    path: "INBOX".to_owned(),
                    delimiter: Some('/'),
                    special: Some(SpecialUse::Inbox),
                    subscription: Subscription::Subscribed,
                    holds: Holds::Mail,
                },
                Folder {
                    account: acct_account(),
                    path: ALL_MAIL.to_owned(),
                    delimiter: Some('/'),
                    special: Some(SpecialUse::All),
                    subscription: Subscription::Subscribed,
                    holds: Holds::Mail,
                },
            ],
        )
        .unwrap();
    // 503 was synced from the inbox, at its inbox UID.
    let (_, raw) = all_mail().remove(2);
    let head = raw.split("\r\n\r\n").next().unwrap().to_owned() + "\r\n\r\n";
    mail_runtime::absorb(
        &store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        None,
        vec![Arrival {
            remote: RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 3,
                uid: 7,
            },
            raw: head.into_bytes(),
        }],
        true,
        now(),
    )
    .unwrap();

    let secrets = MemorySecrets::default();
    mail_runtime::block_on(secrets.put(
        &SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new(PASSWORD.to_owned())),
    ))
    .unwrap();
    let auth = ImapAuth {
        username: "me@example.test".to_owned(),
        credential: Credential::Password(SecretText::new(PASSWORD.to_owned())),
        sasl: vec![SaslMech::Plain],
    };
    let backend = ImapBackend::new(
        acct_account(),
        caps(),
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if authenticate == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(auth.clone(), all)
        }),
    );
    let engine = AccountEngine::new(
        acct_account(),
        plan(port),
        backend,
        store.clone(),
        Arc::new(secrets),
    );
    Fixture {
        store,
        engine,
        _dir: dir,
    }
}

fn subjects(store: &SqliteStore) -> Vec<String> {
    let query = Query {
        filter: Filter::Account(acct_account()),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 50,
        },
    };
    store
        .threads(&query, now())
        .unwrap()
        .items
        .into_iter()
        .map(|t| t.subject)
        .collect()
}

fn lunch() -> Filter {
    Filter::Text(TextMatch::Contains("lunch".to_owned()))
}

async fn searches_all_mail_and_keeps_only_what_is_new(esearch: bool) {
    let (port, seen) = serve(esearch).await;
    let mut f = fixture(port);
    let (_tx, mut cancel) = watch::channel(false);
    assert_eq!(subjects(&f.store), vec!["lunch on friday"]);

    let Searched::Found(hits) = f
        .engine
        .search_imap(&lunch(), &[], &mut cancel, now())
        .await
        .unwrap()
    else {
        panic!("the search was not asked");
    };
    assert_eq!(hits.messages.len(), 2, "both lunches: {hits:?}");
    assert_eq!(hits.fetched, 1, "only the 2019 lunch is new here");
    assert_eq!(hits.more, 0);
    assert_eq!(
        subjects(&f.store),
        vec!["lunch on friday", "lunch in 2019"],
        "the hit is listed like any other message, and nothing twice"
    );
    let old = f
        .store
        .threads(
            &Query {
                filter: Filter::Subject(TextMatch::Contains("2019".to_owned())),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 5,
                },
            },
            now(),
        )
        .unwrap()
        .items;
    let every: Vec<ThreadId> = f
        .store
        .threads(
            &Query {
                filter: Filter::All,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 50,
                },
            },
            now(),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(
        f.store.found_in(&every).unwrap(),
        vec![old[0].id],
        "only what the search brought is from the server"
    );
    let message = f.store.message(hits.messages[1]).unwrap();
    assert_eq!(message.body, Body::Absent, "headers only");
    assert_eq!(
        message.read,
        ReadState::Read,
        "the server's flags came with them"
    );

    // Again: everything it finds is held now, so nothing is fetched.
    let Searched::Found(again) = f
        .engine
        .search_imap(&lunch(), &[], &mut cancel, now())
        .await
        .unwrap()
    else {
        panic!("the search was not asked");
    };
    assert_eq!(again.messages, hits.messages);
    assert_eq!(again.fetched, 0);
    assert_eq!(subjects(&f.store).len(), 2, "no duplicate");

    let seen = seen.lock().unwrap().clone();
    let searches: Vec<&String> = seen
        .iter()
        .filter(|c| c.starts_with("UID SEARCH"))
        .collect();
    assert_eq!(searches.len(), 2);
    let expected = if esearch {
        "UID SEARCH RETURN (COUNT ALL) OR"
    } else {
        "UID SEARCH OR"
    };
    assert!(searches[0].starts_with(expected), "{searches:?}");
    assert!(
        seen.iter().any(|c| c == &format!("EXAMINE \"{ALL_MAIL}\"")),
        "All Mail, read-only: {seen:?}"
    );
    let fetches: Vec<&String> = seen.iter().filter(|c| c.starts_with("UID FETCH")).collect();
    assert_eq!(
        fetches,
        vec!["UID FETCH 503,501 (UID FLAGS BODY.PEEK[HEADER])"],
        "one header fetch, the first time, of what was not held at those UIDs"
    );
}

/// The same search against a server without ESEARCH and one with it.
#[tokio::test]
async fn searches_all_mail_with_and_without_esearch() {
    for esearch in [false, true] {
        // Captured, and printed beside a failure, so the failure names the row.
        eprintln!("row: esearch = {esearch}");
        searches_all_mail_and_keeps_only_what_is_new(esearch).await;
    }
}

#[tokio::test]
async fn a_query_that_cannot_be_asked_sends_nothing() {
    let (port, seen) = serve(true).await;
    let mut f = fixture(port);
    let (_tx, mut cancel) = watch::channel(false);
    let pinned = Filter::And(vec![Filter::Pinned, lunch()]);
    let said = f
        .engine
        .search_imap(&pinned, &[], &mut cancel, now())
        .await
        .unwrap();
    assert_eq!(
        said,
        Searched::Unsaid(mail_runtime::Unsaid(vec![
            "is:pinned (kept on this computer)".to_owned()
        ]))
    );
    assert_eq!(seen.lock().unwrap().len(), 0, "no connection was made");
}

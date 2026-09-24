//! Every followed folder of an IMAP account is fetched, not only the inbox and Sent, and one
//! folder can be fetched on demand — `plan.md` 10.2, the half that lists a folder's mail.
//!
//! Against a server on a real socket that holds several mailboxes, including a nested one and one
//! whose name travels as modified UTF-7, because the parts most likely to be wrong are which
//! mailbox is selected when, and what a message held in two of them becomes.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::sync;
use mail_domain::*;
use mail_runtime::{MapSecrets, OAuthRegistry, Secrets};
use mail_store::{SqliteStore, Store};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const PROJECTS: &str = "Projects/2026";
const REPORTS: &str = "收件匣/報告";
const OLD: &str = "Old";

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

/// One message, with a date that orders it: `minute` minutes into a fixed hour.
fn message(id: &str, subject: &str, minute: u32) -> String {
    format!(
        "From: Ada <ada@example.test>\r\n\
         To: me@example.test\r\n\
         Subject: {subject}\r\n\
         Date: Tue, 14 Nov 2023 {:02}:{:02}:00 +0000\r\n\
         Message-ID: <{id}@example.test>\r\n\
         \r\n\
         the body of {subject}\r\n",
        10 + minute / 60,
        minute % 60
    )
}

/// What the server holds: each mailbox's messages by UID. The same bytes in two mailboxes are
/// the copies a server keeps, under different UIDs.
type Mailboxes = BTreeMap<String, Vec<(u32, String)>>;

fn mailboxes(projects: usize) -> Mailboxes {
    let shared = message("shared", "in both", 3);
    let mut out = Mailboxes::new();
    // UIDs are per mailbox, and these are also UIDs in `Projects/2026` once it is large: a
    // fetch that asked the wrong mailbox for them would get the wrong message back.
    out.insert(
        "INBOX".to_owned(),
        vec![
            (300, message("first", "first", 1)),
            (301, message("second", "second", 2)),
            (302, shared.clone()),
        ],
    );
    let mut filed = vec![(7, shared)];
    for n in 0..projects {
        filed.push((
            100 + n as u32,
            message(&format!("p{n}"), &format!("project {n}"), 4 + n as u32 % 50),
        ));
    }
    out.insert(PROJECTS.to_owned(), filed);
    out.insert(
        REPORTS.to_owned(),
        vec![(5, message("report", "quarterly", 5))],
    );
    out.insert(OLD.to_owned(), vec![(9, message("old", "from before", 6))]);
    out
}

/// Every command the server was sent, in order.
type Seen = Arc<Mutex<Vec<String>>>;

/// What the server holds, which a test can add mail to between passes.
type Held = Arc<Mutex<Mailboxes>>;

/// An IMAP server with several mailboxes, on a real socket.
fn serve(mailboxes: Mailboxes) -> (u16, Seen) {
    let (port, seen, _held) = serve_held(mailboxes);
    (port, seen)
}

fn serve_held(mailboxes: Mailboxes) -> (u16, Seen, Held) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let held: Held = Arc::new(Mutex::new(mailboxes));
    let (log, shared) = (seen.clone(), held.clone());
    std::thread::spawn(move || {
        for sock in listener.incoming() {
            let Ok(sock) = sock else { return };
            let (held, log) = (shared.clone(), log.clone());
            std::thread::spawn(move || session(sock, &held, &log));
        }
    });
    (port, seen, held)
}

fn session(sock: std::net::TcpStream, held: &Held, seen: &Seen) {
    let mut out = sock.try_clone().unwrap();
    let mut lines = BufReader::new(sock);
    if out
        .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        .is_err()
    {
        return;
    }
    let mut selected: Option<Vec<(u32, String)>> = None;
    loop {
        let mut line = String::new();
        match lines.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = line.trim_end().to_owned();
        let (tag, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        let upper = rest.to_uppercase();
        seen.lock().unwrap().push(if upper.starts_with("LOGIN") {
            "LOGIN".to_owned()
        } else {
            rest.to_owned()
        });
        let reply = if upper.starts_with("CAPABILITY") {
            format!("* CAPABILITY IMAP4rev1\r\n{tag} OK done\r\n")
        } else if upper.starts_with("LOGIN") || upper.starts_with("NOOP") {
            format!("{tag} OK done\r\n")
        } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
            let name = rest
                .split_once(' ')
                .map(|(_, n)| n.trim_start_matches('"'))
                .and_then(|n| n.split('"').next())
                .map(mail_proto::mutf7::decode)
                .unwrap_or_default();
            selected = held.lock().unwrap().get(&name).cloned();
            match &selected {
                Some(mailbox) => {
                    let next = mailbox.iter().map(|(uid, _)| uid + 1).max().unwrap_or(1);
                    format!(
                        "* {} EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n\
                         * OK [UIDNEXT {next}] next\r\n{tag} OK [READ-ONLY] done\r\n",
                        mailbox.len()
                    )
                }
                None => format!("{tag} NO no such mailbox\r\n"),
            }
        } else if upper.starts_with("UID FETCH") {
            let mailbox = selected.as_deref().unwrap_or_default();
            fetch(mailbox, &upper, tag)
        } else if upper.starts_with("UID SEARCH") {
            let uids: Vec<String> = selected
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|(uid, _)| uid.to_string())
                .collect();
            format!("* SEARCH {}\r\n{tag} OK done\r\n", uids.join(" "))
        } else if upper.starts_with("LOGOUT") {
            let _ = out.write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes());
            return;
        } else {
            format!("{tag} BAD unknown command\r\n")
        };
        if out.write_all(reply.as_bytes()).is_err() {
            return;
        }
    }
}

/// `UID FETCH <set> <items>`, answered in mailbox order whatever order the set was written in.
fn fetch(mailbox: &[(u32, String)], upper: &str, tag: &str) -> String {
    let set = upper
        .strip_prefix("UID FETCH ")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("");
    let wanted = |uid: u32| {
        set.split(',').any(|part| match part.split_once(':') {
            Some((lo, hi)) => {
                let lo: u32 = lo.parse().unwrap_or(1);
                uid >= lo && (hi == "*" || hi.parse::<u32>().is_ok_and(|hi| uid <= hi))
            }
            None => part.parse::<u32>().is_ok_and(|n| n == uid),
        })
    };
    let mut out = String::new();
    for (seq, (uid, raw)) in mailbox.iter().enumerate() {
        if !wanted(*uid) {
            continue;
        }
        let seq = seq + 1;
        if upper.contains("BODY.PEEK[]") {
            out.push_str(&format!(
                "* {seq} FETCH (UID {uid} BODY[] {{{}}}\r\n{raw})\r\n",
                raw.len()
            ));
        } else if upper.contains("BODY.PEEK[HEADER]") {
            let head = raw.split("\r\n\r\n").next().unwrap_or("").to_owned() + "\r\n\r\n";
            out.push_str(&format!(
                "* {seq} FETCH (UID {uid} FLAGS () BODY[HEADER] {{{}}}\r\n{head})\r\n",
                head.len()
            ));
        } else {
            out.push_str(&format!(
                "* {seq} FETCH (UID {uid} FLAGS () RFC822.SIZE {})\r\n",
                raw.len()
            ));
        }
    }
    out.push_str(&format!("{tag} OK done\r\n"));
    out
}

fn caps(labels: ServerLabels) -> AccountCaps {
    AccountCaps {
        labels,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(Vec::new()),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        // Fresh, so no pass re-reads capabilities or the folder list from this server.
        observed_at: now(),
    }
}

/// A store with one IMAP account on `port`, its password held, and `folders` as its listing.
fn configured(
    port: u16,
    caps: AccountCaps,
    folders: Vec<Folder>,
) -> (Arc<SqliteStore>, Arc<MapSecrets>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    let plan = AccountPlan {
        address: "ada@example.test".to_owned(),
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
    };
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'ada@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    store.put_caps(ACCOUNT, &caps, now()).unwrap();
    store.put_folders(ACCOUNT, folders).unwrap();
    let secrets = Arc::new(MapSecrets::default());
    secrets
        .put(
            &SecretKey {
                account: ACCOUNT,
                purpose: SecretPurpose::IncomingPassword,
            },
            &Credential::Password("s3cr3t".to_owned()),
        )
        .unwrap();
    (store, secrets, dir)
}

fn folder(path: &str, subscription: Subscription) -> Folder {
    Folder {
        account: ACCOUNT,
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription,
        holds: Holds::Mail,
    }
}

/// INBOX, the two followed folders, and one nobody follows.
fn listing() -> Vec<Folder> {
    vec![
        folder("INBOX", Subscription::Subscribed),
        folder(PROJECTS, Subscription::Subscribed),
        folder(REPORTS, Subscription::Subscribed),
        folder(OLD, Subscription::Unsubscribed),
    ]
}

fn pass(store: &Arc<SqliteStore>, secrets: &Arc<MapSecrets>) -> String {
    sync::run_with(
        store.clone(),
        secrets.clone(),
        &OAuthRegistry::default(),
        now(),
    )
    .expect("a pass against a reachable server runs")
    .text
}

fn at(path: &str) -> MailboxRef {
    MailboxRef {
        account: ACCOUNT,
        path: path.to_owned(),
    }
}

/// The subjects of the threads a filter lists, sorted.
fn listed(store: &SqliteStore, filter: Filter) -> Vec<String> {
    let page = store
        .threads(
            &Query {
                filter,
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: 1000,
                },
            },
            now(),
        )
        .unwrap();
    let mut subjects: Vec<String> = page.items.into_iter().map(|s| s.subject).collect();
    subjects.sort();
    subjects
}

fn count(store: &SqliteStore, sql: &str) -> i64 {
    store.connection().query_row(sql, [], |r| r.get(0)).unwrap()
}

/// The paths the server was asked to select, in order, decoded.
fn selected(seen: &Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|c| c.starts_with("EXAMINE") || c.starts_with("SELECT"))
        .filter_map(|c| {
            c.split_once(' ')
                .map(|(_, n)| n.trim_matches('"').to_owned())
        })
        .map(|n| mail_proto::mutf7::decode(&n))
        .collect()
}

#[test]
fn a_pass_fetches_every_followed_folder_and_lists_each_by_its_path() {
    let (port, seen) = serve(mailboxes(1));
    let (store, secrets, _dir) = configured(port, caps(ServerLabels::LocalOnly), listing());

    let out = pass(&store, &secrets);
    assert!(!out.contains("needs attention"), "{out}");

    // Five messages: two only in the inbox, one in the inbox and a folder, one in each folder.
    // Not the one in the folder nobody follows.
    assert_eq!(count(&store, "SELECT count(*) FROM messages"), 5, "{out}");
    let paths = selected(&seen);
    assert!(paths.iter().any(|p| p == PROJECTS), "{paths:?}");
    assert!(paths.iter().any(|p| p == REPORTS), "{paths:?}");
    assert!(
        !paths.iter().any(|p| p == OLD),
        "an unfollowed folder was fetched: {paths:?}"
    );
    // The non-ASCII name went over the wire encoded, and came back to the store decoded.
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .any(|c| c.contains(&mail_proto::mutf7::encode(REPORTS))),
        "the folder was not selected by its modified UTF-7 name"
    );

    assert_eq!(
        listed(&store, Filter::InFolder(at(PROJECTS))),
        vec!["in both", "project 0"]
    );
    assert_eq!(
        listed(&store, Filter::InFolder(at(REPORTS))),
        vec!["quarterly"]
    );
    assert_eq!(
        listed(&store, Filter::InFolder(at("INBOX"))),
        vec!["first", "in both", "second"]
    );
    // On a settled mailbox the inbox by role and by path are the same list: mail from a folder
    // is not in the inbox, and the copy that is in both is.
    assert_eq!(
        listed(&store, Filter::InMailbox(MailboxRole::Inbox)),
        listed(&store, Filter::InFolder(at("INBOX")))
    );
    assert_eq!(
        listed(&store, Filter::InMailbox(MailboxRole::Archive)),
        vec!["project 0", "quarterly"],
        "a folder with no role files its mail as archived"
    );

    // And every body arrived, each fetched from the mailbox its UID belongs to.
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM messages WHERE body_raw IS NULL"
        ),
        0,
        "{out}"
    );
}

#[test]
fn a_message_held_in_two_folders_is_one_message_with_two_addresses() {
    let (port, _seen) = serve(mailboxes(1));
    let (store, secrets, _dir) = configured(port, caps(ServerLabels::LocalOnly), listing());
    pass(&store, &secrets);

    let ids: Vec<String> = {
        let db = store.connection();
        let mut stmt = db
            .prepare("SELECT id FROM messages WHERE subject = 'in both'")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(ids.len(), 1, "the copy in the folder was stored again");
    let id = MessageId::from_uuid(ids[0].parse().unwrap());
    let mut remotes = store.remotes_of(id).unwrap();
    remotes.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(
        remotes,
        vec![
            RemoteRef::Imap {
                mailbox: "INBOX".to_owned(),
                uidvalidity: 42,
                uid: 302
            },
            RemoteRef::Imap {
                mailbox: PROJECTS.to_owned(),
                uidvalidity: 42,
                uid: 7
            },
        ]
    );
    assert_eq!(store.message(id).unwrap().mailbox, MailboxRole::Inbox);

    // A second pass fetches nothing again, in any folder.
    let again = pass(&store, &secrets);
    assert!(again.contains(": 0 headers, 0 bodies"), "{again}");
}

#[test]
fn the_inbox_is_brought_up_to_date_before_a_large_folder() {
    // More than one pass's worth of headers in the folder, and the inbox is still whole after
    // the first pass — fetched first, and before any of the folder's.
    let (port, seen, server) = serve_held(mailboxes(260));
    let (store, secrets, _dir) = configured(port, caps(ServerLabels::LocalOnly), listing());

    pass(&store, &secrets);
    assert_eq!(
        listed(&store, Filter::InFolder(at("INBOX"))),
        vec!["first", "in both", "second"]
    );
    let in_projects = listed(&store, Filter::InFolder(at(PROJECTS))).len();
    assert!(
        in_projects < 261,
        "the folder took more than one pass's budget: {in_projects}"
    );
    let paths = selected(&seen);
    let last_inbox = paths.iter().rposition(|p| p == "INBOX").unwrap();
    let first_folder = paths.iter().position(|p| p == PROJECTS).unwrap();
    assert!(
        last_inbox < first_folder,
        "the folder was fetched before the inbox was done: {paths:?}"
    );

    // New mail in the inbox, while the folder still has bodies outstanding under UIDs the inbox
    // also uses. The next pass finishes the folder's headers.
    server
        .lock()
        .unwrap()
        .get_mut("INBOX")
        .unwrap()
        .push((303, message("third", "third", 59)));
    pass(&store, &secrets);
    assert_eq!(listed(&store, Filter::InFolder(at(PROJECTS))).len(), 261);

    // And every address names the message the server holds there. A body batch drawn from the
    // whole account put the new message and the folder's backlog in one fetch, selected the
    // inbox for all of it, and filed the inbox's messages under the folder's addresses.
    let server = server.lock().unwrap().clone();
    let rows: Vec<(String, i64, String)> = {
        let db = store.connection();
        let mut stmt = db
            .prepare(
                "SELECT r.mailbox, r.uid, m.rfc_message_id
                 FROM remote_map r JOIN messages m ON m.id = r.message",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let held = rows.len();
    for (mailbox, uid, id) in rows {
        let raw = &server[&mailbox]
            .iter()
            .find(|(u, _)| i64::from(*u) == uid)
            .unwrap()
            .1;
        assert!(
            raw.contains(&format!("<{id}>")),
            "{mailbox} {uid} is filed as {id}, which the server does not hold there"
        );
    }
    assert_eq!(
        held,
        4 + 261 + 1,
        "the inbox, the large folder, and the other one"
    );
    // A hundred bodies per folder per pass: a third pass holds all of them.
    pass(&store, &secrets);
    assert_eq!(
        count(
            &store,
            "SELECT count(*) FROM messages WHERE body_raw IS NULL"
        ),
        0
    );
}

#[test]
fn a_folder_nobody_follows_is_fetched_when_asked_for() {
    let (port, seen) = serve(mailboxes(1));
    let (store, secrets, _dir) = configured(port, caps(ServerLabels::LocalOnly), listing());
    assert!(listed(&store, Filter::InFolder(at(OLD))).is_empty());

    let ran = sync::folder_now_with(
        store.clone(),
        secrets.clone(),
        &OAuthRegistry::default(),
        ACCOUNT,
        OLD,
        now(),
    )
    .expect("a folder the server holds can be fetched");
    assert!(ran.text.contains("1 headers, 1 bodies"), "{}", ran.text);
    assert_eq!(
        listed(&store, Filter::InFolder(at(OLD))),
        vec!["from before"]
    );
    // Only that folder: the inbox is the next pass's business.
    assert_eq!(selected(&seen).iter().filter(|p| *p != OLD).count(), 0);
    let message = store.message(first_message(&store)).unwrap();
    assert_eq!(message.mailbox, MailboxRole::Archive);
}

fn first_message(store: &SqliteStore) -> MessageId {
    let id: String = store
        .connection()
        .query_row("SELECT id FROM messages LIMIT 1", [], |r| r.get(0))
        .unwrap();
    MessageId::from_uuid(id.parse().unwrap())
}

#[test]
fn a_folder_the_server_does_not_have_is_said_not_thrown() {
    let (port, _seen) = serve(mailboxes(1));
    let (store, secrets, _dir) = configured(port, caps(ServerLabels::LocalOnly), listing());
    let ran = sync::folder_now_with(
        store,
        secrets,
        &OAuthRegistry::default(),
        ACCOUNT,
        "Nowhere",
        now(),
    )
    .expect("reported, not returned as an error");
    assert!(ran.text.contains("needs attention"), "{}", ran.text);
}

#[test]
fn an_account_whose_folders_are_labels_is_not_fetched_by_folder() {
    // Nothing listens on port 1: both refusals come before anything is sent.
    let (store, secrets, _dir) = configured(1, caps(ServerLabels::Supported), listing());
    let refused = sync::folder_now_with(
        store.clone(),
        secrets,
        &OAuthRegistry::default(),
        ACCOUNT,
        PROJECTS,
        now(),
    )
    .unwrap_err();
    assert!(refused.contains("labels"), "{refused}");
    let paths = sync::mailboxes_by_account(&store).unwrap().remove(0).1;
    assert_eq!(
        paths,
        vec!["INBOX".to_owned()],
        "labels are not fetched as folders"
    );
}

#[test]
fn which_folders_a_pass_fetches() {
    let special = |path: &str, special: SpecialUse| Folder {
        special: Some(special),
        ..folder(path, Subscription::Subscribed)
    };
    let mut listing = listing();
    listing.extend([
        special("Drafts", SpecialUse::Drafts),
        special("Junk", SpecialUse::Junk),
        special("Trash", SpecialUse::Trash),
        special("All", SpecialUse::All),
        Folder {
            holds: Holds::FoldersOnly,
            ..folder("Projects", Subscription::Subscribed)
        },
    ]);
    let mut caps = caps(ServerLabels::LocalOnly);
    caps.folders = FolderRoles(vec![
        ("Sent".to_owned(), MailboxRole::Sent),
        ("Drafts".to_owned(), MailboxRole::Drafts),
        ("Junk".to_owned(), MailboxRole::Spam),
        ("Trash".to_owned(), MailboxRole::Trash),
    ]);
    let (store, _secrets, _dir) = configured(1, caps, listing);
    let paths = sync::mailboxes_by_account(&store).unwrap().remove(0).1;
    let mut rest = paths[2..].to_vec();
    rest.sort();
    assert_eq!(
        &paths[..2],
        ["INBOX", "Sent"],
        "the inbox, then Sent, first"
    );
    assert_eq!(rest, vec!["Junk", PROJECTS, "Trash", REPORTS]);
}

//! A draft's copy in the server's Drafts folder, over real sockets: kept as it changes, one copy
//! at a time, and gone once the draft is sent.
//!
//! The IMAP server holds a Drafts folder for real: `APPEND` puts a message in it under a new UID,
//! `UID SEARCH DRAFT HEADER Message-ID` reads the messages it holds, `UID STORE` sets their flags
//! and `UID EXPUNGE` removes the `\Deleted` ones it names. So a copy left behind, or one removed that
//! should not have been, shows in what the folder holds afterwards, whatever the client said.

use crate::fixture::{IDENTITY, acct_account, now};
use mail_domain::*;
use mail_mime::posting;
use mail_proto::backend::{Authenticate, ImapBackend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession};
use mail_runtime::AccountEngine;
use mail_secrets::AccountSecrets;
use mail_store::{SqliteStore, Store};
use porter_core::{Credential, SecretKey, SecretPurpose, SecretText};
use porter_secrets::MemorySecrets;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::watch;

const PASSWORD: &str = "s3cr3t";

/// One message in the server's Drafts folder.
#[derive(Debug, Clone)]
struct Held {
    uid: u32,
    flags: Vec<String>,
    body: String,
}

/// The Drafts folder, and every command the server was sent.
#[derive(Debug, Default)]
struct Folder {
    held: Vec<Held>,
    next_uid: u32,
    commands: Vec<String>,
}

type Shared = Arc<Mutex<Folder>>;

impl Folder {
    /// The messages not marked `\Deleted`: what another client shows.
    fn live(&self) -> Vec<Held> {
        self.held
            .iter()
            .filter(|held| !held.flags.iter().any(|f| f == "\\Deleted"))
            .cloned()
            .collect()
    }
}

/// The UIDs a set like `3,5` names.
fn uids_of(set: &str) -> Vec<u32> {
    // A piece that is not a number names no UID.
    set.split(',').filter_map(|n| n.parse().ok()).collect()
}

/// An IMAP server with a Drafts folder. `uidplus` decides whether it offers `UID EXPUNGE`.
async fn serve_imap(folder: Shared, uidplus: bool) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let caps = if uidplus {
        "IMAP4rev1 UIDPLUS UNSELECT"
    } else {
        "IMAP4rev1 UNSELECT"
    };
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            let folder = folder.clone();
            tokio::spawn(async move {
                let greeting = format!("* OK [CAPABILITY {caps}] ready\r\n");
                if sock.write_all(greeting.as_bytes()).await.is_err() {
                    return;
                }
                let mut buf = Vec::new();
                loop {
                    let mut chunk = [0u8; 4096];
                    match sock.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                    while let Some(at) = buf.windows(2).position(|w| w == b"\r\n") {
                        let line = String::from_utf8_lossy(&buf[..at]).to_string();
                        buf.drain(..at + 2);
                        let (tag, rest) = line.split_once(' ').unwrap_or((&line, ""));
                        let upper = rest.to_uppercase();
                        if !upper.starts_with("LOGIN") {
                            folder.lock().unwrap().commands.push(rest.to_owned());
                        }
                        let reply = if upper.starts_with("CAPABILITY") {
                            format!("* CAPABILITY {caps}\r\n{tag} OK done\r\n")
                        } else if upper.starts_with("LIST") {
                            format!(
                                "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                                 * LIST (\\HasNoChildren \\Drafts) \"/\" \"Drafts\"\r\n\
                                 {tag} OK done\r\n"
                            )
                        } else if upper.starts_with("LSUB") {
                            format!("* LSUB () \"/\" \"INBOX\"\r\n{tag} OK done\r\n")
                        } else if upper.starts_with("SELECT") || upper.starts_with("EXAMINE") {
                            let folder = folder.lock().unwrap();
                            format!(
                                "* {} EXISTS\r\n* OK [UIDVALIDITY 7] ok\r\n\
                                 * OK [UIDNEXT {}] next\r\n{tag} OK [READ-WRITE] done\r\n",
                                folder.held.len(),
                                folder.next_uid.max(1)
                            )
                        } else if upper.starts_with("UID SEARCH DRAFT HEADER MESSAGE-ID") {
                            let wanted = rest
                                .split_once('"')
                                .and_then(|(_, after)| after.rsplit_once('"'))
                                .map(|(id, _)| id.to_owned())
                                // No quoted id: an empty one, which no copy carries.
                                .unwrap_or_default();
                            let found: Vec<String> = folder
                                .lock()
                                .unwrap()
                                .held
                                .iter()
                                .filter(|held| {
                                    // DRAFT matches the \Draft flag (RFC 9051 §6.4.4.1).
                                    held.flags.contains(&"\\Draft".to_owned())
                                        && held.body.contains(&format!("Message-ID: {wanted}\r\n"))
                                })
                                .map(|held| held.uid.to_string())
                                .collect();
                            format!("* SEARCH {}\r\n{tag} OK done\r\n", found.join(" "))
                        } else if let Some(store) = upper.strip_prefix("UID STORE ") {
                            let (set, what) = store.split_once(' ').unwrap_or((store, ""));
                            assert_eq!(what, "+FLAGS.SILENT (\\DELETED)", "{rest}");
                            let uids = uids_of(set);
                            for held in folder.lock().unwrap().held.iter_mut() {
                                if uids.contains(&held.uid) {
                                    held.flags.push("\\Deleted".to_owned());
                                }
                            }
                            format!("{tag} OK stored\r\n")
                        } else if let Some(set) = upper.strip_prefix("UID EXPUNGE ") {
                            if !uidplus {
                                format!("{tag} BAD no UIDPLUS here\r\n")
                            } else {
                                let uids = uids_of(set);
                                folder.lock().unwrap().held.retain(|held| {
                                    !(uids.contains(&held.uid)
                                        && held.flags.iter().any(|f| f == "\\Deleted"))
                                });
                                format!("{tag} OK expunged\r\n")
                            }
                        } else if upper.starts_with("EXPUNGE") {
                            panic!("a bare EXPUNGE was sent");
                        } else if upper.starts_with("APPEND") {
                            let want: usize = rest
                                .rsplit_once('{')
                                // No literal size: an empty upload.
                                .and_then(|(_, n)| n.trim_end_matches('}').parse().ok())
                                .unwrap_or(0);
                            // A client gone mid-upload ends this connection at the next read.
                            let _ = sock.write_all(b"+ ready for literal\r\n").await;
                            while buf.len() < want + 2 {
                                let mut chunk = [0u8; 4096];
                                match sock.read(&mut chunk).await {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                                }
                            }
                            let body = buf.drain(..want).collect::<Vec<u8>>();
                            buf.drain(..2);
                            let flags = rest
                                .split_once('(')
                                .and_then(|(_, after)| after.split_once(')'))
                                .map(|(flags, _)| {
                                    flags.split_whitespace().map(str::to_owned).collect()
                                })
                                // No flag list: the copy is held with none.
                                .unwrap_or_default();
                            let mut folder = folder.lock().unwrap();
                            folder.next_uid = folder.next_uid.max(1);
                            let uid = folder.next_uid;
                            folder.next_uid += 1;
                            folder.held.push(Held {
                                uid,
                                flags,
                                body: String::from_utf8_lossy(&body).to_string(),
                            });
                            // No `APPENDUID`, as UW IMAP: the copies are found by their
                            // Message-ID, never by where an upload said it landed.
                            format!("{tag} OK append done\r\n")
                        } else if upper.starts_with("LOGOUT") {
                            // The client may already be gone; the connection ends either way.
                            let _ = sock
                                .write_all(format!("* BYE\r\n{tag} OK done\r\n").as_bytes())
                                .await;
                            return;
                        } else if upper.starts_with("LOGIN")
                            || upper.starts_with("NOOP")
                            || upper.starts_with("UNSELECT")
                        {
                            format!("{tag} OK done\r\n")
                        } else {
                            format!("{tag} BAD unknown command\r\n")
                        };
                        if sock.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
            });
        }
    });
    port
}

/// A submission server that accepts every message, and counts them.
async fn serve_smtp(sent: Arc<Mutex<usize>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let sent = sent.clone();
            tokio::spawn(async move {
                let (read, mut write) = sock.into_split();
                let mut lines = BufReader::new(read).lines();
                // A client gone before the greeting ends at the first read.
                let _ = write.write_all(b"220 smtp.example ESMTP ready\r\n").await;
                let mut in_data = false;
                while let Ok(Some(line)) = lines.next_line().await {
                    if in_data {
                        if line == "." {
                            in_data = false;
                            *sent.lock().unwrap() += 1;
                            // A client gone after its message ends at the next read.
                            let _ = write.write_all(b"250 2.0.0 Ok: queued\r\n").await;
                        }
                        continue;
                    }
                    let upper = line.to_uppercase();
                    let reply = if upper.starts_with("EHLO") {
                        "250-smtp.example\r\n250-8BITMIME\r\n250 AUTH PLAIN\r\n"
                    } else if upper.starts_with("AUTH") {
                        "235 2.7.0 Accepted\r\n"
                    } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                        "250 2.1.0 Ok\r\n"
                    } else if upper.starts_with("DATA") {
                        in_data = true;
                        "354 End data with <CR><LF>.<CR><LF>\r\n"
                    } else if upper.starts_with("QUIT") {
                        // The client may already be gone; the connection ends either way.
                        let _ = write.write_all(b"221 2.0.0 Bye\r\n").await;
                        return;
                    } else {
                        "500 5.5.2 Unrecognized command\r\n"
                    };
                    if write.write_all(reply.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

fn identity() -> Identity {
    Identity {
        id: IDENTITY,
        account: acct_account(),
        from: Address {
            name: Some("Me".to_owned()),
            email: "me@example.test".to_owned(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    }
}

fn draft() -> Draft {
    Draft {
        id: DraftId::generate(),
        account: acct_account(),
        identity: IDENTITY,
        to: vec![Address {
            name: None,
            email: "bea@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![Address {
            name: None,
            email: "dee@example.test".to_owned(),
        }],
        subject: "half a thought".to_owned(),
        in_reply_to: None,
        forward_of: None,
        text: "to be continued\r\n".to_owned(),
        html: None,
        attachments: vec![],
        receipt: ReceiptRequest::Unrequested,
        openpgp: None,
        smime: None,
        priority: Priority::Normal,
        delivery: DeliveryReport::Unrequested,
        automatic: false,
        state: SendState::Editing,
        updated: now(),
    }
}

struct Writing {
    store: Arc<SqliteStore>,
    engine: AccountEngine<ImapBackend>,
    draft: Draft,
    _dir: tempfile::TempDir,
}

/// An IMAP account on the server at `imap`, sending through `smtp`, whose server names a Drafts
/// folder, with a draft being written.
fn writing(imap: u16, smtp: u16) -> Writing {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
    mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
    mail_store::testing::seed_identity_for(
        &store,
        IDENTITY,
        acct_account(),
        "me@example.test",
        Some("Me"),
    );
    let secrets = MemorySecrets::default();
    mail_secrets::block_on(secrets.put(
        &SecretKey {
            account: acct_account(),
            purpose: SecretPurpose::IncomingPassword,
        },
        &Credential::Password(SecretText::new(PASSWORD.to_owned())),
    ))
    .unwrap();
    let draft = draft();
    save(&store, &draft);

    let caps = AccountCaps {
        labels: ServerLabels::LocalOnly,
        threads: ServerThreads::Jwz,
        watch: WatchMode::Poll {
            every: std::time::Duration::from_secs(300),
        },
        archive: ArchiveMeans::LocalOnly,
        folders: FolderRoles(vec![("Drafts".to_owned(), MailboxRole::Drafts)]),
        condstore: Condstore::Absent,
        move_ext: MoveExt::Absent,
        expunge: ExpungeMeans::Forbidden,
        top: Supported::Absent,
        pipelining: Supported::Absent,
        connections: ConnectionBudget { max: 1 },
        observed_at: now(),
    };
    let auth = ImapAuth {
        username: "me@example.test".to_owned(),
        credential: Credential::Password(SecretText::new(PASSWORD.to_owned())),
        sasl: vec![SaslMech::Plain],
    };
    let backend = ImapBackend::new(
        acct_account(),
        caps,
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if authenticate == Authenticate::First {
                all.push(ImapCommand::Login);
            }
            all.extend(commands);
            ImapSession::new(auth.clone(), all)
        }),
    );
    let plan = AccountPlan {
        address: "me@example.test".to_owned(),
        incoming: Incoming::Imap {
            host: "127.0.0.1".to_owned(),
            port: imap,
            tls: Tls::Plaintext,
        },
        outgoing: Outgoing::Smtp {
            host: "127.0.0.1".to_owned(),
            port: smtp,
            tls: Tls::Plaintext,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: vec![identity()],
    };
    let engine = AccountEngine::new(
        acct_account(),
        plan,
        backend,
        store.clone(),
        Arc::new(secrets),
    );
    Writing {
        store,
        engine,
        draft,
        _dir: dir,
    }
}

fn save(store: &SqliteStore, draft: &Draft) {
    store
        .apply(
            acct_account(),
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
}

fn drafts() -> MailboxRef {
    MailboxRef {
        account: acct_account(),
        path: "Drafts".to_owned(),
    }
}

/// Queue the draft's copy as it now stands, as the composer's autosave does.
fn keep(store: &SqliteStore, draft: &Draft) {
    assert!(
        mail_runtime::server_drafts::queue_keep(store, draft, drafts(), now()).unwrap(),
        "the copy was not queued"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_draft_kept_twice_leaves_one_copy_and_sending_it_removes_that_copy() {
    let folder: Shared = Arc::default();
    let sent = Arc::new(Mutex::new(0));
    let mut it = writing(
        serve_imap(folder.clone(), true).await,
        serve_smtp(sent.clone()).await,
    );
    let (_tx, mut cancel) = watch::channel(false);
    let message_id = format!("<{}@example.test>", it.draft.id);

    keep(&it.store, &it.draft);
    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert!(report.needs_attention.is_empty(), "{report:?}");
    {
        let folder = folder.lock().unwrap();
        assert_eq!(folder.held.len(), 1, "{:?}", folder.commands);
        let copy = &folder.held[0];
        assert!(copy.flags.contains(&"\\Draft".to_owned()), "{copy:?}");
        assert!(copy.flags.contains(&"\\Seen".to_owned()), "{copy:?}");
        assert!(copy.body.contains(&format!("Message-ID: {message_id}\r\n")));
        // Only the sender reads it there, so the blind copy is named.
        assert!(
            copy.body.contains("Bcc: <dee@example.test>\r\n"),
            "{}",
            copy.body
        );
    }
    // Nothing of it is kept here as a message: the draft is the document.
    assert_eq!(
        mail_store::testing::count(&it.store, "messages"),
        0,
        "the uploaded copy became a message"
    );

    let edited = Draft {
        text: "finished on the train\r\n".to_owned(),
        ..it.draft.clone()
    };
    save(&it.store, &edited);
    keep(&it.store, &edited);
    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert!(report.needs_attention.is_empty(), "{report:?}");
    {
        let folder = folder.lock().unwrap();
        assert_eq!(
            folder.held.len(),
            1,
            "one copy, the latest: {:?}",
            folder.commands
        );
        assert!(folder.held[0].body.contains("finished on the train"));
        assert!(
            folder
                .commands
                .iter()
                .any(|c| c == "UID STORE 1 +FLAGS.SILENT (\\Deleted)"),
            "{:?}",
            folder.commands
        );
        assert!(
            folder.commands.iter().any(|c| c == "UID EXPUNGE 1"),
            "{:?}",
            folder.commands
        );
    }

    let post = posting(&edited, &identity(), None, &[]).unwrap();
    let raw = it.store.blobs().put(&post.message).unwrap();
    it.store
        .enqueue(
            acct_account(),
            RemoteIntent::Send {
                draft: edited.id,
                raw,
                mail_from: post.mail_from.clone(),
                rcpt_to: post.rcpt_to.clone(),
                delivery: post.delivery,
            },
            &Patch {
                id: ChangeId::generate(),
                changes: Vec::new(),
            },
            now(),
        )
        .unwrap()
        .expect("a submission is always queued");
    it.store
        .set_send_state(edited.id, &SendState::Queued, now())
        .unwrap();
    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert_eq!(report.submitted, 1, "{report:?}");
    assert!(report.needs_attention.is_empty(), "{report:?}");
    assert_eq!(
        report.still_queued, 0,
        "removed in the same drain: {report:?}"
    );
    assert_eq!(*sent.lock().unwrap(), 1);
    let folder = folder.lock().unwrap();
    assert!(
        folder.held.is_empty(),
        "the sent draft is still in Drafts: {:?}",
        folder.held
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_uidplus_the_old_copy_is_left_marked_deleted_and_nothing_is_expunged() {
    let folder: Shared = Arc::default();
    let mut it = writing(
        serve_imap(folder.clone(), false).await,
        serve_smtp(Arc::default()).await,
    );
    let (_tx, mut cancel) = watch::channel(false);

    keep(&it.store, &it.draft);
    it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    keep(&it.store, &it.draft);
    let report = it.engine.drain_outbox(&mut cancel, now()).await.unwrap();
    assert!(report.needs_attention.is_empty(), "{report:?}");

    let folder = folder.lock().unwrap();
    assert_eq!(folder.held.len(), 2, "{:?}", folder.commands);
    assert_eq!(folder.live().len(), 1, "one copy shows: {:?}", folder.held);
    assert!(
        !folder
            .commands
            .iter()
            .any(|c| c.to_uppercase().contains("EXPUNGE")),
        "{:?}",
        folder.commands
    );
}

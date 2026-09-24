//! `mailo import` and `mailo export` against a real `SqliteStore` and real files in a temporary
//! directory: formats in, the local-only account, idempotency, a search out, and the local
//! account's absence from everything that talks to a server.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::{cli, compose, export, import, sync};
use mail_domain::*;
use mail_mime::archive::mbox;
use mail_runtime::{OAuthRegistry, RuntimeError, Secrets};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn fresh_store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path().join("blobs")).unwrap());
    (store, dir)
}

/// A small Takeout-style mbox: a thread of two, one of them unread in the inbox, the other
/// archived under a label with a comma in it, and a body line that begins `From `.
const TAKEOUT: &str = "\
From 1@xxx Tue Nov 07 13:20:51 +0000 2023
X-Gmail-Labels: Inbox,Unread
Message-ID: <first@example.test>
From: Ada <ada@example.test>
To: me@example.test
Subject: Lunch on Friday
Date: Tue, 07 Nov 2023 13:20:51 +0000

Shall we?
>From the archive, a quoted line.

From 2@xxx Wed Nov 08 09:00:00 +0000 2023
X-Gmail-Labels: Archived,\"Trips, 2023\",Starred
Message-ID: <second@example.test>
In-Reply-To: <first@example.test>
References: <first@example.test>
From: Bob <bob@example.test>
To: ada@example.test
Subject: Re: Lunch on Friday
Date: Wed, 08 Nov 2023 09:00:00 +0000

Yes, noon.

";

fn quiet(_: &import::Imported) {}

fn write(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn local_account(store: &SqliteStore) -> AccountId {
    sync::local_accounts(store)
        .into_iter()
        .next()
        .expect("the local account exists after an import")
}

fn every_message(store: &SqliteStore, account: AccountId) -> Vec<Message> {
    let page = store
        .threads(
            &Query {
                filter: Filter::Account(account),
                sort: Sort {
                    property: Property::Date,
                    dir: SortDir::Asc,
                },
                page: PageReq {
                    after: None,
                    limit: 1000,
                },
            },
            now(),
        )
        .unwrap();
    let mut out: Vec<Message> = page
        .items
        .iter()
        .flat_map(|t| store.thread(t.id).unwrap().messages)
        .map(|id| store.message(id).unwrap())
        .collect();
    out.sort_by_key(|m| m.date);
    out
}

#[test]
fn a_takeout_mbox_lands_in_local_folders_threaded_labelled_and_only_once() {
    let (store, dir) = fresh_store();
    let path = write(dir.path(), "All mail.mbox", TAKEOUT.as_bytes());
    let source = import::detect(&path).unwrap();
    assert_eq!(source, import::Source::Mbox(path.clone()));

    let first = import::into_local(&store, &source, now(), &mut quiet).unwrap();
    assert_eq!((first.read, first.added, first.already), (2, 2, 0));

    let again = import::into_local(&store, &source, now(), &mut quiet).unwrap();
    assert_eq!(
        (again.read, again.added, again.already),
        (2, 0, 2),
        "re-importing the same file keeps nothing twice"
    );

    let account = local_account(&store);
    let messages = every_message(&store, account);
    assert_eq!(messages.len(), 2);
    let (lunch, reply) = (&messages[0], &messages[1]);
    assert_eq!(
        lunch.thread, reply.thread,
        "the reply threads onto the first"
    );
    assert_eq!(lunch.mailbox, MailboxRole::Inbox);
    assert_eq!(lunch.read, ReadState::Unread);
    assert_eq!(reply.mailbox, MailboxRole::Archive);
    assert_eq!(reply.read, ReadState::Read);
    assert_eq!(reply.star, Star::Starred);
    let names: Vec<String> = store
        .labels(account)
        .unwrap()
        .into_iter()
        .filter(|l| reply.labels.contains(&l.id))
        .map(|l| l.name)
        .collect();
    assert_eq!(names, vec!["Trips, 2023".to_owned()]);

    // The body line was unquoted on the way in, and the bytes are stored in CRLF.
    let Body::Present { raw, .. } = &lunch.body else {
        panic!("imported with a body")
    };
    let bytes = store.blobs().get(&store.connection(), *raw).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\r\nFrom the archive, a quoted line.\r\n"),
        "{text}"
    );

    // And it is searchable like any other mail.
    let found = export::select(&store, "noon", now()).unwrap();
    assert_eq!(found, vec![reply.id]);
}

#[test]
fn a_maildir_keeps_its_flags_and_its_folders() {
    let (store, dir) = fresh_store();
    let root = dir.path().join("Mail");
    for sub in [
        "cur",
        "new",
        "tmp",
        ".Receipts/cur",
        ".Receipts/new",
        ".Sent/cur",
    ] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let message = |id: &str, subject: &str| {
        format!(
            "Message-ID: <{id}@example.test>\nFrom: a@example.test\nSubject: {subject}\n\
             Date: Tue, 07 Nov 2023 13:20:51 +0000\n\nbody\n"
        )
    };
    write(
        &root.join("new"),
        "1699363251.M1P1Q1.host",
        message("new", "fresh").as_bytes(),
    );
    write(
        &root.join("cur"),
        "1699363252.M1P1Q2.host:2,FS",
        message("starred", "starred").as_bytes(),
    );
    write(
        &root.join(".Receipts/cur"),
        "1699363253.M1P1Q3.host:2,S",
        message("receipt", "a receipt").as_bytes(),
    );
    write(
        &root.join(".Sent/cur"),
        "1699363254.M1P1Q4.host:2,S",
        message("sent", "sent one").as_bytes(),
    );

    let source = import::detect(&root).unwrap();
    assert_eq!(source, import::Source::Maildir(root.clone()));
    let total = import::into_local(&store, &source, now(), &mut quiet).unwrap();
    assert_eq!(total.added, 4);

    let account = local_account(&store);
    let by_subject = |subject: &str| {
        every_message(&store, account)
            .into_iter()
            .find(|m| m.subject == subject)
            .unwrap()
    };
    assert_eq!(by_subject("fresh").read, ReadState::Unread);
    assert_eq!(by_subject("fresh").mailbox, MailboxRole::Inbox);
    assert_eq!(by_subject("starred").star, Star::Starred);
    assert_eq!(by_subject("starred").read, ReadState::Read);
    assert_eq!(by_subject("a receipt").mailbox, MailboxRole::Archive);
    assert_eq!(by_subject("a receipt").labels.len(), 1);
    assert_eq!(by_subject("sent one").mailbox, MailboxRole::Sent);
}

#[test]
fn a_single_eml_is_one_message_even_in_crlf() {
    let (store, dir) = fresh_store();
    let path = write(
        dir.path(),
        "note.eml",
        b"From: a@example.test\r\nSubject: just one\r\n\r\nhello\r\n",
    );
    let source = import::detect(&path).unwrap();
    assert_eq!(source, import::Source::Eml(path.clone()));
    let first = import::into_local(&store, &source, now(), &mut quiet).unwrap();
    assert_eq!(first.added, 1);
    // No Message-ID: the identity is a digest that includes the bytes, so it still dedupes.
    let again = import::into_local(&store, &source, now(), &mut quiet).unwrap();
    assert_eq!((again.added, again.already), (0, 1));
}

#[test]
fn a_search_exports_to_mbox_and_reads_back_the_same_bytes() {
    let (store, dir) = fresh_store();
    let path = write(dir.path(), "in.mbox", TAKEOUT.as_bytes());
    import::into_local(&store, &import::detect(&path).unwrap(), now(), &mut quiet).unwrap();

    let chosen = export::select(&store, "from:bob", now()).unwrap();
    assert_eq!(chosen.len(), 1, "only Bob's message, not the thread");
    let out = dir.path().join("out.mbox");
    let done = export::export(
        &store,
        &chosen,
        &export::Target::Mbox(out.clone()),
        now(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(done.written, 1);

    let file = std::fs::File::open(&out).unwrap();
    let back: Vec<_> = mbox::Reader::new(std::io::BufReader::new(file))
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(back.len(), 1);
    assert!(String::from_utf8_lossy(&back[0].raw).contains("Yes, noon."));
    assert_eq!(back[0].envelope.sender, "bob@example.test");

    // Never over an existing file.
    assert!(
        export::export(
            &store,
            &chosen,
            &export::Target::Mbox(out),
            now(),
            &mut |_| {}
        )
        .is_err()
    );
}

#[test]
fn everything_exports_to_a_maildir_that_imports_back_as_the_same_mail() {
    let (store, dir) = fresh_store();
    let path = write(dir.path(), "in.mbox", TAKEOUT.as_bytes());
    import::into_local(&store, &import::detect(&path).unwrap(), now(), &mut quiet).unwrap();

    let all = export::select(&store, "all", now()).unwrap();
    let root = dir.path().join("Exported");
    let done = export::export(
        &store,
        &all,
        &export::Target::Maildir(root.clone()),
        now(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(done.written, 2);
    // The archived message went to its label's folder, flagged read and starred.
    let receipts: Vec<String> = std::fs::read_dir(root.join(".Trips, 2023/cur"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(receipts.len(), 1);
    assert!(receipts[0].ends_with(":2,FS"), "{receipts:?}");

    // Importing the export into a fresh store gives the same messages back.
    let (fresh, _fresh_dir) = fresh_store();
    let total =
        import::into_local(&fresh, &import::detect(&root).unwrap(), now(), &mut quiet).unwrap();
    assert_eq!(total.added, 2);
    let reply = every_message(&fresh, local_account(&fresh))
        .into_iter()
        .find(|m| m.subject.starts_with("Re:"))
        .unwrap();
    assert_eq!(reply.star, Star::Starred);
    assert_eq!(reply.mailbox, MailboxRole::Archive);

    // And into .eml files, one each.
    let emls = dir.path().join("emls");
    let done = export::export(
        &store,
        &all,
        &export::Target::Eml(emls.clone()),
        now(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(done.written, 2);
    assert_eq!(std::fs::read_dir(emls).unwrap().count(), 2);
}

#[test]
fn a_message_with_no_body_yet_is_skipped_and_counted() {
    let (store, _dir) = fresh_store();
    let account = AccountId::generate();
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'ada@example.test', '{}', datetime('now'))",
            [account.to_string()],
        )
        .unwrap();
    let message = Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account,
        key: MessageKey::Rfc("headers-only@example.test".into()),
        date: now(),
        from: Address {
            name: None,
            email: "a@example.test".into(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "headers only".into(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("headers-only@example.test".into()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    };
    store
        .import(
            account,
            Import {
                messages: vec![Kept {
                    key: message.key.clone(),
                    raw: BlobId::generate(),
                    message,
                    labels: vec![],
                }],
            },
        )
        .unwrap();
    let chosen = export::select(&store, "inbox", now()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let target = export::Target::Eml(dir.path().join("out"));
    let done = export::export(&store, &chosen, &target, now(), &mut |_| {}).unwrap();
    assert_eq!((done.written, done.absent), (0, 1));
    assert!(export::said(&done, &target).contains("mailo sync"));
}

/// Secrets that count how often anything asked.
#[derive(Default)]
struct Counting(AtomicUsize);

impl Secrets for Counting {
    fn get(&self, _: &SecretKey) -> Result<Credential, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(RuntimeError::Secrets("none".to_owned()))
    }
    fn put(&self, _: &SecretKey, _: &Credential) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn forget(&self, _: &SecretKey) -> Result<(), RuntimeError> {
        Ok(())
    }
}

#[test]
fn sync_never_touches_the_local_account() {
    let (store, dir) = fresh_store();
    let path = write(dir.path(), "in.mbox", TAKEOUT.as_bytes());
    import::into_local(&store, &import::detect(&path).unwrap(), now(), &mut quiet).unwrap();
    let before = every_message(&store, local_account(&store));

    let secrets = Arc::new(Counting::default());
    let ran = sync::run_with(
        store.clone(),
        secrets.clone(),
        &OAuthRegistry::default(),
        now(),
    )
    .unwrap();
    assert_eq!(
        secrets.0.load(Ordering::SeqCst),
        0,
        "a credential was asked for"
    );
    assert!(ran.text.contains("kept on this computer"), "{}", ran.text);
    assert_eq!(every_message(&store, local_account(&store)), before);

    // No folder work, and no sending.
    let listed = cli::run(&store, &cli::Command::FolderList { account: None }, now()).unwrap();
    assert!(listed.contains("only labels"), "{listed}");
    // An identity by hand: the local account has none, which is the point, but a draft row
    // must name one.
    let identity = IdentityId::generate();
    store
        .connection()
        .execute(
            "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                 is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', NULL, NULL, ?3)",
            rusqlite::params![
                identity.to_string(),
                local_account(&store).to_string(),
                serde_json::to_string(&IsDefault::Default).unwrap()
            ],
        )
        .unwrap();
    let draft = Draft {
        id: DraftId::generate(),
        account: local_account(&store),
        identity,
        to: vec![Address {
            name: None,
            email: "bob@example.test".into(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "from nowhere".into(),
        in_reply_to: None,
        forward_of: None,
        text: "hi".into(),
        html: None,
        attachments: vec![],
        state: SendState::Editing,
        updated: now(),
        receipt: ReceiptRequest::Unrequested,
    };
    store
        .apply(
            draft.account,
            &Patch {
                id: ChangeId::generate(),
                changes: vec![Change::DraftUpsert(Box::new(draft.clone()))],
            },
        )
        .unwrap();
    let refused = compose::send(&store, draft.id, now()).unwrap_err();
    assert!(refused.contains("no server to send from"), "{refused}");
}

#[test]
fn the_commands_parse() {
    let args = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        cli::parse(&args(&["import", "a.mbox"])).unwrap(),
        cli::Command::Import {
            path: "a.mbox".into(),
            into: import::Destination::Local,
        }
    );
    assert_eq!(
        cli::parse(&args(&[
            "import",
            "Mail",
            "--to-mailbox",
            "ada@example.test",
            "Old"
        ]))
        .unwrap(),
        cli::Command::Import {
            path: "Mail".into(),
            into: import::Destination::Mailbox {
                account: "ada@example.test".into(),
                folder: "Old".into(),
            },
        }
    );
    assert_eq!(
        cli::parse(&args(&[
            "export",
            "from:ada",
            "is:unread",
            "--mbox",
            "out.mbox"
        ]))
        .unwrap(),
        cli::Command::Export {
            query: "from:ada is:unread".into(),
            target: export::Target::Mbox("out.mbox".into()),
        }
    );
    assert!(cli::parse(&args(&["export", "--mbox", "out.mbox"])).is_err());
    assert!(cli::parse(&args(&["export", "inbox"])).is_err());
}

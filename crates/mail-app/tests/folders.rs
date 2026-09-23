//! `mailo folder` against a real store: what changes here at once, what is queued for the
//! server, and what is refused before anything is.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::cli;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn run(store: &SqliteStore, words: &str) -> Result<String, String> {
    let args: Vec<String> = words.split(' ').map(str::to_owned).collect();
    let command = cli::parse(&args)?;
    cli::run_with_clients(
        store,
        &command,
        now(),
        &mail_runtime::OAuthRegistry::default(),
    )
}

const IMAP: &str = "me@nowhere.example";
const POP: &str = "you@nowhere.example";

/// Write an account as `account add` would, without going near a credential.
fn configure(store: &SqliteStore, id: AccountId, preset: presets::Preset) {
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                id.to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                now().to_rfc3339(),
            ],
        )
        .unwrap();
    store.put_caps(id, &preset.expected_caps, now()).unwrap();
}

/// A store with one IMAP account whose folders have been listed, and one POP3 account.
fn store() -> (SqliteStore, tempfile::TempDir, AccountId) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let account = AccountId::from_uuid(uuid::Uuid::from_u128(0xa1));
    configure(
        &store,
        account,
        presets::manual(
            IMAP,
            &presets::Manual {
                imap_host: "imap.nowhere.example".to_owned(),
                imap_port: 993,
                smtp_host: "smtp.nowhere.example".to_owned(),
                smtp_port: 465,
                login: None,
            },
            now(),
        ),
    );
    configure(
        &store,
        AccountId::from_uuid(uuid::Uuid::from_u128(0xa2)),
        presets::manual_pop3(
            POP,
            &presets::ManualPop3 {
                pop3_host: "pop.nowhere.example".to_owned(),
                pop3_port: 995,
                smtp_host: "smtp.nowhere.example".to_owned(),
                smtp_port: 465,
                login: None,
            },
            now(),
        ),
    );
    let folder = |path: &str, special: Option<SpecialUse>| Folder {
        account,
        path: path.to_owned(),
        delimiter: Some('/'),
        special,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    store
        .put_folders(
            account,
            vec![
                folder("INBOX", None),
                folder("Sent", Some(SpecialUse::Sent)),
                folder("Work", None),
            ],
        )
        .unwrap();
    (store, dir, account)
}

fn queued(store: &SqliteStore, account: AccountId) -> Vec<ProtoOp> {
    store
        .outbox_due(account, now())
        .unwrap()
        .into_iter()
        .map(|e| e.op)
        .collect()
}

#[test]
fn a_new_folder_is_listed_at_once_and_queued_for_the_server() {
    let (store, _dir, account) = store();
    let before = queued(&store, account).len();

    let said = run(&store, &format!("folder new {IMAP} Receipts")).unwrap();
    assert!(said.contains("created Receipts"), "{said}");

    let listed = run(&store, &format!("folder list {IMAP}")).unwrap();
    assert!(listed.contains("  Receipts\n"), "{listed}");
    assert_eq!(
        queued(&store, account)[before..],
        [ProtoOp::Folder(FolderWork::Create {
            path: "Receipts".to_owned()
        })]
    );
}

#[test]
fn the_listing_marks_what_cannot_be_moved() {
    let (store, _dir, _account) = store();
    let listed = run(&store, "folder list").unwrap();
    assert!(listed.contains("  Sent  (sent)\n"), "{listed}");
    assert!(listed.contains("  INBOX  (inbox)\n"), "{listed}");
    assert!(listed.contains("POP3 has one mailbox"), "{listed}");
}

#[test]
fn a_rename_moves_it_here_and_queues_the_rename() {
    let (store, _dir, account) = store();
    run(&store, &format!("folder rename {IMAP} Work Jobs")).unwrap();
    let paths: Vec<String> = store
        .folders(account)
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(paths, ["INBOX", "Jobs", "Sent"]);
    assert!(
        queued(&store, account).contains(&ProtoOp::Folder(FolderWork::Rename {
            from: "Work".to_owned(),
            to: "Jobs".to_owned(),
        }))
    );
}

#[test]
fn the_sent_folder_and_the_inbox_are_refused_by_name() {
    let (store, _dir, account) = store();
    let before = queued(&store, account).len();
    let error = run(
        &store,
        &format!("folder delete {IMAP} Sent --with-messages"),
    )
    .unwrap_err();
    assert!(error.contains("Sent folder"), "{error}");
    let error = run(&store, &format!("folder rename {IMAP} INBOX Old")).unwrap_err();
    assert!(error.contains("Inbox folder"), "{error}");
    assert_eq!(queued(&store, account).len(), before, "nothing was queued");
}

/// A folder holding mail this client has synced is refused without the option, and allowed
/// with it.
#[test]
fn deleting_a_folder_with_mail_in_it_needs_saying_so() {
    let (store, _dir, account) = store();
    store
        .ingest(
            account,
            Ingest {
                mailbox: MailboxRef {
                    account,
                    path: "Work".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "Work".to_owned(),
                        uidvalidity: 1,
                        uid: 5,
                    },
                    key: MessageKey::Rfc("m5@example.test".to_owned()),
                    raw: BlobId::generate(),
                    message: message(account),
                }],
                flags: vec![],
                labels: vec![],
                label_names: vec![],
                gone: vec![],
            },
        )
        .unwrap();

    let error = run(&store, &format!("folder delete {IMAP} Work")).unwrap_err();
    assert!(error.contains("holds 1 message"), "{error}");
    assert!(
        run(&store, &format!("folder list {IMAP}"))
            .unwrap()
            .contains("  Work\n")
    );

    run(
        &store,
        &format!("folder delete {IMAP} Work --with-messages"),
    )
    .unwrap();
    assert!(
        !run(&store, &format!("folder list {IMAP}"))
            .unwrap()
            .contains("Work")
    );
}

#[test]
fn a_pop3_account_is_told_it_has_no_folders() {
    let (store, _dir, _account) = store();
    let error = run(&store, &format!("folder new {POP} Receipts")).unwrap_err();
    assert!(error.contains("POP3"), "{error}");
}

#[test]
fn an_unknown_account_is_named() {
    let (store, _dir, _account) = store();
    let error = run(&store, "folder new nobody@nowhere.example Receipts").unwrap_err();
    assert!(error.contains("nobody@nowhere.example"), "{error}");
}

fn message(account: AccountId) -> Message {
    Message {
        id: MessageId::generate(),
        thread: ThreadId::generate(),
        account,
        key: MessageKey::Rfc("m5@example.test".to_owned()),
        date: now(),
        from: Address {
            name: None,
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![],
        cc: vec![],
        bcc: vec![],
        subject: "in Work".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: None,
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Absent,
        attachments: vec![],
    }
}

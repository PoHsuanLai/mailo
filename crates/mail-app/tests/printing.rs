//! `mailo print`: which messages an id names, where the document goes, and what it is called.
//!
//! What the document says is `mail-mime/tests/print.rs`. This is the half that reads the store
//! and writes files.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::cli::{self, Command, PrintTo};
use mail_domain::*;
use mail_mime::Pages;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000 + n, 0).unwrap()
}

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn exercise(store: &SqliteStore, command: &Command) -> Result<String, String> {
    cli::run_with_clients(
        store,
        command,
        at(10_000),
        &mail_runtime::OAuthRegistry::default(),
    )
}

fn seeded() -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let plan = mail_domain::presets::manual_pop3(
        "me@example.test",
        &mail_domain::presets::ManualPop3 {
            pop3_host: "pop.example.test".to_owned(),
            pop3_port: 995,
            smtp_host: "smtp.example.test".to_owned(),
            smtp_port: 465,
            login: None,
        },
        at(0),
    )
    .plan;
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&plan).unwrap()],
        )
        .unwrap();
    (store, dir)
}

/// Store a message saying `text`, in `thread` or a new one. Returns its id and thread.
fn stored(
    store: &SqliteStore,
    n: i64,
    subject: &str,
    text: &str,
    thread: Option<ThreadId>,
) -> (MessageId, ThreadId) {
    let rfc_id = format!("m{n}@example.test");
    let raw_bytes = format!(
        "From: sender{n}@example.test\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
         Message-ID: <{rfc_id}>\r\n\r\n{text}\r\n"
    );
    let raw = store
        .blobs()
        .put(&store.connection(), raw_bytes.as_bytes())
        .unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: thread.unwrap_or_else(ThreadId::generate),
        account: ACCOUNT,
        key: MessageKey::Rfc(rfc_id.clone()),
        date: at(n),
        from: Address {
            name: None,
            email: format!("sender{n}@example.test"),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: subject.to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(rfc_id),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some(text.to_owned()),
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
                        uidl: format!("u{n}"),
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
    let thread = store.message(id).unwrap().thread;
    (id, thread)
}

fn print(target: uuid::Uuid, out: PrintTo) -> Command {
    Command::Print {
        target,
        out,
        pages: Pages::Flow,
    }
}

#[test]
fn print_parses_its_id_and_options_in_any_order() {
    let id = "00000000-0000-4000-8000-000000000001";
    let target: uuid::Uuid = id.parse().unwrap();
    assert_eq!(
        cli::parse(&args(&["print", id])).unwrap(),
        print(target, PrintTo::Unsaid)
    );
    assert_eq!(
        cli::parse(&args(&["print", id, "--page-per-message", "--out", "-"])).unwrap(),
        Command::Print {
            target,
            out: PrintTo::Stdout,
            pages: Pages::PerMessage,
        }
    );
    assert_eq!(
        cli::parse(&args(&["print", id, "--out", "lunch.html"])).unwrap(),
        print(target, PrintTo::File("lunch.html".into()))
    );
    assert!(cli::parse(&args(&["print"])).is_err());
    assert!(cli::parse(&args(&["print", "nope"])).is_err());
    assert!(cli::parse(&args(&["print", id, "--out"])).is_err());
    assert!(cli::parse(&args(&["print", id, "--landscape"])).is_err());
}

#[test]
fn a_thread_id_prints_every_message_oldest_first() {
    let (store, _dir) = seeded();
    // Stored out of order: the printout is ordered by date, not by arrival.
    let (_, thread) = stored(&store, 30, "Plan", "second reply", None);
    stored(&store, 10, "Plan", "the first word", Some(thread));
    stored(&store, 20, "Re: Plan", "a reply", Some(thread));
    let messages = store.thread(thread).unwrap().messages.len();
    assert_eq!(messages, 3, "the three share a thread");

    let html = exercise(&store, &print(*thread.as_uuid(), PrintTo::Stdout)).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"), "{html}");
    let first = html.find("the first word").unwrap();
    let second = html.find("a reply").unwrap();
    let third = html.find("second reply").unwrap();
    assert!(first < second && second < third, "{html}");
    assert_eq!(html.matches("<article").count(), 3);
}

#[test]
fn a_message_id_prints_that_message_alone() {
    let (store, _dir) = seeded();
    let (message, thread) = stored(&store, 10, "Plan", "the first word", None);
    stored(&store, 20, "Re: Plan", "a reply", Some(thread));

    let html = exercise(&store, &print(*message.as_uuid(), PrintTo::Unsaid)).unwrap();
    assert_eq!(html.matches("<article").count(), 1);
    assert!(html.contains("the first word") && !html.contains("a reply"));
}

#[test]
fn an_unknown_id_is_an_error_that_says_so() {
    let (store, _dir) = seeded();
    let err = exercise(&store, &print(uuid::Uuid::new_v4(), PrintTo::Stdout)).unwrap_err();
    assert!(err.contains("neither a message nor a thread"), "{err}");
}

#[test]
fn printing_into_a_directory_names_the_file_from_the_subject_and_never_overwrites() {
    let (store, _dir) = seeded();
    let (message, _) = stored(&store, 10, "Q3/Q4 figures", "numbers", None);
    let out = tempfile::tempdir().unwrap();
    let into = PrintTo::Into(out.path().to_owned());

    let said = exercise(&store, &print(*message.as_uuid(), into.clone())).unwrap();
    let first = out.path().join("Q3-Q4 figures.html");
    assert_eq!(said, format!("wrote {}\n", first.display()));
    assert!(std::fs::read_to_string(&first).unwrap().contains("numbers"));

    std::fs::write(&first, "mine").unwrap();
    let said = exercise(&store, &print(*message.as_uuid(), into)).unwrap();
    let second = out.path().join("Q3-Q4 figures (2).html");
    assert_eq!(said, format!("wrote {}\n", second.display()));
    assert_eq!(std::fs::read_to_string(&first).unwrap(), "mine");
    assert!(
        std::fs::read_to_string(&second)
            .unwrap()
            .contains("numbers")
    );
}

#[test]
fn out_writes_the_file_it_names() {
    let (store, _dir) = seeded();
    let (message, _) = stored(&store, 10, "Plan", "the first word", None);
    let out = tempfile::tempdir().unwrap();
    let path = out.path().join("chosen.html");

    let said = exercise(
        &store,
        &print(*message.as_uuid(), PrintTo::File(path.clone())),
    )
    .unwrap();
    assert_eq!(said, format!("wrote {}\n", path.display()));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("the first word")
    );
}

#[test]
fn a_subject_cannot_choose_where_its_file_goes() {
    use mail_app::print::file_name;
    assert_eq!(file_name("Lunch"), "Lunch.html");
    assert_eq!(
        file_name("../../.ssh/authorized_keys"),
        "-..-.ssh-authorized_keys.html"
    );
    assert_eq!(file_name("..\\..\\evil"), "-..-evil.html");
    assert_eq!(file_name(""), "message.html");
    assert_eq!(file_name("  ..  "), "message.html");
    assert_eq!(file_name("line\nbreak\u{0}"), "linebreak.html");
    let long = file_name(&"é".repeat(400));
    assert!(long.len() <= 255 && long.ends_with(".html"), "{long}");
    for name in [file_name("../../x"), file_name("/etc/passwd"), long] {
        assert!(!name.contains('/') && !name.contains('\\'), "{name}");
        assert_ne!(name, "..");
    }
}

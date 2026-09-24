//! The sheets' functions against real files in a temporary directory and a real store: what a
//! path is said to hold, an import into local folders, an export in each format read back, and
//! local folders kept away from everything that talks to a server.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

use super::work::{
    self, Counted, Dest, Format, Looked, expand, export_now, import_now, look, prefill, suggested,
};
use crate::import::Source;
use crate::view::Shell;

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_700_000_000, 0).unwrap()
}

fn fresh_store() -> (Arc<SqliteStore>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(SqliteStore::in_memory(dir.path().join("blobs")).unwrap());
    (store, dir)
}

fn message(id: &str, subject: &str) -> String {
    format!(
        "Message-ID: <{id}@example.test>\nFrom: Ada <ada@example.test>\nTo: me@example.test\n\
         Subject: {subject}\nDate: Tue, 07 Nov 2023 13:20:51 +0000\n\nbody of {subject}\n"
    )
}

/// A Maildir of three: one new, one read and starred, one in a Maildir++ folder.
fn a_maildir(under: &Path) -> PathBuf {
    let root = under.join("Mail");
    for sub in ["cur", "new", "tmp", ".Receipts/cur", ".Receipts/new"] {
        std::fs::create_dir_all(root.join(sub)).unwrap();
    }
    let put = |at: PathBuf, text: String| std::fs::write(at, text).unwrap();
    put(
        root.join("new/1699363251.M1P1Q1.host"),
        message("one", "fresh"),
    );
    put(
        root.join("cur/1699363252.M1P1Q2.host:2,FS"),
        message("two", "starred"),
    );
    put(
        root.join(".Receipts/cur/1699363253.M1P1Q3.host:2,S"),
        message("three", "a receipt"),
    );
    root
}

/// An mbox of two, one of whose body lines begins `From `.
fn an_mbox(under: &Path) -> PathBuf {
    let path = under.join("Takeout.mbox");
    let text = format!(
        "From ada@example.test Tue Nov 07 13:20:51 2023\n{}\n>From here on, quoted.\n\n\
         From bob@example.test Wed Nov 08 09:00:00 2023\n{}\n",
        message("m1", "lunch"),
        message("m2", "Re: lunch")
    );
    std::fs::write(&path, text).unwrap();
    path
}

/// Every message the account holds, by subject.
fn subjects(store: &SqliteStore, account: AccountId) -> Vec<String> {
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
    let mut out: Vec<String> = page
        .items
        .iter()
        .flat_map(|t| store.thread(t.id).unwrap().messages)
        .map(|id| store.message(id).unwrap().subject)
        .collect();
    out.sort();
    out
}

fn the_local_account(store: &SqliteStore) -> AccountId {
    let local = crate::sync::local_accounts(store);
    assert_eq!(local.len(), 1, "one local account: {local:?}");
    local[0]
}

/// What the sheet says is at `path`, and the source it will read.
fn looked_mail(path: &Path) -> (Source, usize, String) {
    match look(path) {
        Looked::Mail {
            source,
            count,
            said,
        } => (source, count, said),
        other => panic!("{} was not mail: {other:?}", path.display()),
    }
}

#[test]
fn a_maildir_is_counted_then_imported_into_local_folders() {
    let (store, dir) = fresh_store();
    let root = a_maildir(dir.path());
    let (source, count, said) = looked_mail(&root);
    assert_eq!(source, Source::Maildir(root.clone()));
    assert_eq!((count, said.as_str()), (3, "Maildir, 3 messages"));
    assert!(crate::sync::local_accounts(&store).is_empty());

    let mut seen = Vec::new();
    let done = import_now(&store, &source, &Dest::Local, now(), &mut |so_far| {
        seen.push(so_far.read)
    })
    .unwrap();
    assert_eq!((done.total.read, done.total.added), (3, 3));
    assert_eq!(
        done.said,
        "3 message(s) read; 3 kept in local folders; 0 already there"
    );
    assert_eq!(done.queued, None, "local mail waits for no upload");
    assert_eq!(seen.last(), Some(&3), "progress reached the end: {seen:?}");
    let account = the_local_account(&store);
    assert_eq!(
        subjects(&store, account),
        vec!["a receipt", "fresh", "starred"]
    );
}

#[test]
fn an_mbox_is_counted_then_imported_into_local_folders_once() {
    let (store, dir) = fresh_store();
    let path = an_mbox(dir.path());
    let (source, count, said) = looked_mail(&path);
    assert_eq!(source, Source::Mbox(path.clone()));
    assert_eq!((count, said.as_str()), (2, "mbox, 2 messages"));

    let done = import_now(&store, &source, &Dest::Local, now(), &mut |_| {}).unwrap();
    assert_eq!(
        done.said,
        "2 message(s) read; 2 kept in local folders; 0 already there"
    );
    let account = the_local_account(&store);
    assert_eq!(subjects(&store, account), vec!["Re: lunch", "lunch"]);

    let again = import_now(&store, &source, &Dest::Local, now(), &mut |_| {}).unwrap();
    assert_eq!(
        again.said,
        "2 message(s) read; 0 kept in local folders; 2 already there"
    );
    assert_eq!(subjects(&store, account).len(), 2);
}

#[test]
fn a_place_exports_in_each_format_and_reads_back_as_as_many_messages() {
    let built = crate::ui::fixtures::work();
    let out = tempfile::tempdir().unwrap();
    let chosen = crate::export::select(&built.store, "inbox", now()).unwrap();
    assert!(!chosen.is_empty(), "the fixture's inbox has mail");
    assert_eq!(
        work::counted(&built.store, "inbox", now()),
        Counted::Some(chosen.len())
    );
    for format in Format::ALL {
        let path = suggested(out.path(), "inbox", format);
        assert!(!path.exists());
        let (done, said) = export_now(
            &built.store,
            "inbox",
            &format.target(path.clone()),
            now(),
            &mut |_| {},
        )
        .unwrap_or_else(|why| panic!("{format:?}: {why}"));
        assert!(
            path.exists(),
            "{format:?} wrote nothing at {}",
            path.display()
        );
        assert_eq!(format.is_file(), path.is_file(), "{format:?}");
        assert_eq!(
            done.written + done.absent + done.partial,
            chosen.len(),
            "{format:?}"
        );
        assert!(done.written > 0, "{format:?}: {said}");
        assert!(
            said.starts_with(&format!("{} message(s) written to ", done.written)),
            "{format:?}: {said}"
        );

        let (again, _kept) = fresh_store();
        let source = match format {
            // A directory of loose files is imported one at a time; the directory as a whole
            // is not a Maildir.
            Format::Eml => {
                let mut read = 0;
                for file in std::fs::read_dir(&path).unwrap() {
                    let (source, _, _) = looked_mail(&file.unwrap().path());
                    read += import_now(&again, &source, &Dest::Local, now(), &mut |_| {})
                        .unwrap()
                        .total
                        .read;
                }
                assert_eq!(read, done.written, "{format:?}");
                continue;
            }
            _ => looked_mail(&path).0,
        };
        let back = import_now(&again, &source, &Dest::Local, now(), &mut |_| {}).unwrap();
        assert_eq!(back.total.read, done.written, "{format:?}");
        assert_eq!(back.total.added, done.written, "{format:?}");
    }
}

#[test]
fn an_export_is_never_written_over_an_existing_file() {
    let out = tempfile::tempdir().unwrap();
    let first = suggested(out.path(), "from:dana has:attachment", Format::Mbox);
    assert_eq!(
        first,
        out.path().join("mailo-from-dana-has-attachment.mbox")
    );
    std::fs::write(&first, b"someone's archive").unwrap();
    let second = suggested(out.path(), "from:dana has:attachment", Format::Mbox);
    assert_eq!(
        second,
        out.path().join("mailo-from-dana-has-attachment (2).mbox")
    );
    std::fs::create_dir(out.path().join("mailo-inbox")).unwrap();
    assert_eq!(
        suggested(out.path(), "inbox", Format::Maildir),
        out.path().join("mailo-inbox (2)")
    );
    assert_eq!(
        suggested(out.path(), "  ", Format::Eml),
        out.path().join("mailo-mail-eml")
    );
}

#[test]
fn what_cannot_be_read_is_said_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nowhere.mbox");
    assert_eq!(
        look(&missing),
        Looked::Refused(format!("There is nothing at {}.", missing.display()))
    );

    let plain_dir = dir.path().join("Photos");
    std::fs::create_dir(&plain_dir).unwrap();
    let Looked::Refused(why) = look(&plain_dir) else {
        panic!("a plain directory was taken for mail");
    };
    assert!(why.contains("not a Maildir"), "{why}");
    assert!(why.ends_with('.') && !why.contains("os error"), "{why}");

    let empty = dir.path().join("empty.eml");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(
        look(&empty),
        Looked::Refused(format!(
            "{} is empty: there is no mail in it.",
            empty.display()
        ))
    );

    let locked = dir.path().join("locked.mbox");
    std::fs::write(&locked, message("x", "x")).unwrap();
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // A superuser reads it anyway, and then there is nothing to refuse.
    if std::fs::File::open(&locked).is_ok() {
        return;
    }
    assert_eq!(
        look(&locked),
        Looked::Refused(format!(
            "{} cannot be opened: you do not have permission to read it.",
            locked.display()
        ))
    );
    assert_eq!(look(Path::new("")), Looked::Blank);
}

#[test]
fn a_typed_path_starts_at_home_with_a_tilde() {
    let home = Some(std::ffi::OsStr::new("/home/ann"));
    const CASES: &[(&str, &str)] = &[
        ("~", "/home/ann"),
        (
            "~/Downloads/All mail.mbox",
            "/home/ann/Downloads/All mail.mbox",
        ),
        ("  ~/Mail  ", "/home/ann/Mail"),
        ("/srv/mail", "/srv/mail"),
        ("~bob/Mail", "~bob/Mail"),
    ];
    for (typed, wanted) in CASES {
        assert_eq!(expand(typed, home), PathBuf::from(wanted), "{typed:?}");
    }
    assert_eq!(expand("~/Mail", None), PathBuf::from("~/Mail"));
    const SHOWN: &[(&str, &str)] = &[
        ("/home/ann/Downloads", "~/Downloads"),
        ("/home/ann", "~"),
        ("/home/ann2/Downloads", "/home/ann2/Downloads"),
    ];
    for (path, wanted) in SHOWN {
        assert_eq!(super::tilde(Path::new(path), home), *wanted, "{path:?}");
    }
}

#[test]
fn counts_are_read_with_their_thousands_apart() {
    const CASES: &[(usize, &str)] = &[
        (0, "0 messages"),
        (1, "1 message"),
        (999, "999 messages"),
        (1204, "1\u{a0}204 messages"),
        (1_204_000, "1\u{a0}204\u{a0}000 messages"),
    ];
    for (count, wanted) in CASES {
        assert_eq!(work::messages(*count), *wanted, "{count}");
    }
}

#[test]
fn the_export_opens_on_the_search_or_the_place_being_shown() {
    let mut shell = Shell::default();
    let place = |shell: &Shell| shell.places[shell.selected].name.clone();
    assert_eq!(place(&shell), "Inbox");
    assert_eq!(prefill(&shell), "inbox");
    for (name, wanted) in [
        ("Starred", "is:starred"),
        ("Sent", "sent"),
        ("Trash", "trash"),
    ] {
        let index = shell
            .places
            .iter()
            .position(|one| one.name == name)
            .unwrap_or_else(|| panic!("no place {name}"));
        shell.select(index);
        assert_eq!(prefill(&shell), wanted, "{name}");
    }
    shell.search = "from:dana has:attachment".to_owned();
    assert_eq!(prefill(&shell), "from:dana has:attachment");
    // What the field opens with means what the place lists.
    let built = crate::ui::fixtures::work();
    assert!(matches!(
        work::counted(&built.store, "is:starred", now()),
        Counted::Some(n) if n > 0
    ));
}

/// An empty store with one IMAP account whose folders are listed, and a POP3 account.
fn with_folders() -> (Arc<SqliteStore>, tempfile::TempDir, AccountId) {
    let (store, dir) = fresh_store();
    let add = |id: AccountId, plan: &AccountPlan| {
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
                [
                    id.to_string(),
                    plan.address.clone(),
                    serde_json::to_string(plan).unwrap(),
                    now().to_rfc3339(),
                ],
            )
            .unwrap();
    };
    let imap = AccountId::generate();
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    add(
        imap,
        &presets::manual("me@nowhere.example", &manual, now()).plan,
    );
    let pop = presets::ManualPop3 {
        pop3_host: "pop.nowhere.example".to_owned(),
        pop3_port: 995,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    add(
        AccountId::generate(),
        &presets::manual_pop3("pop@nowhere.example", &pop, now()).plan,
    );
    let folder = |path: &str| Folder {
        account: imap,
        path: path.to_owned(),
        delimiter: Some('/'),
        special: None,
        subscription: Subscription::Subscribed,
        holds: Holds::Mail,
    };
    store
        .put_folders(imap, vec![folder("INBOX"), folder("Archive/2023")])
        .unwrap();
    (store, dir, imap)
}

#[test]
fn imported_mail_goes_to_local_folders_or_an_imap_accounts_folder() {
    let (store, _dir, imap) = with_folders();
    let folder = |path: &str| Dest::Folder {
        account: imap,
        address: "me@nowhere.example".to_owned(),
        folder: path.to_owned(),
    };
    let wanted = vec![Dest::Local, folder("Archive/2023"), folder("INBOX")];
    assert_eq!(work::destinations(&store), wanted);
    // Local folders are somewhere to import into, never an account to upload to.
    crate::account::local(&store, now()).unwrap();
    assert_eq!(work::destinations(&store), wanted);
    let labels: Vec<String> = wanted.iter().map(Dest::label).collect();
    assert_eq!(
        labels,
        [
            "Local folders",
            "Archive/2023 on me@nowhere.example",
            "INBOX on me@nowhere.example"
        ]
    );
}

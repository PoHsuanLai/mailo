//! Delete forever and Empty Trash, driven the way their user drives them in the real window on
//! Blitz (`ds_harness::Harness`): offered only in Trash and Spam, always asked first by a sheet
//! that names how much goes and says it cannot be undone, and never offered an Undo after.
//!
//! Every case opens the real window over a store seeded in a `TempDir`. The window is handed no
//! directories, so it writes no file anywhere; nothing here touches the real store or config, and
//! nothing is sent anywhere: the deletions wait in the store's outbox, where they are counted.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
#[path = "support/row_menu.rs"]
mod row_menu;
use drive::Drive;
use ds_blitz::{NetPolicy, PrintOutcome};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The seeded mail, newest first: (sender, subject, where it is).
const MAIL: [(&str, &str, MailboxRole); 4] = [
    (
        "ada@example.test",
        "Flight to the conference",
        MailboxRole::Inbox,
    ),
    ("grace@example.test", "Old invoice", MailboxRole::Trash),
    ("alan@example.test", "Stale lunch plans", MailboxRole::Trash),
    (
        "edsger@example.test",
        "Superseded notes",
        MailboxRole::Trash,
    ),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A store in `dir` with one account and [`MAIL`], each where it says.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [acct_account().to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [
                IdentityId::generate().to_string(),
                acct_account().to_string(),
            ],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject, _)) in MAIL.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <gone{n}@example.test>\r\n\r\nThe body of {subject}.\r\n"
        );
        absorb(
            &store,
            acct_account(),
            MailboxRef {
                account: acct_account(),
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("gone{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    // Filed where the table says, as an earlier move to Trash left them.
    let changes: Vec<Change> = all_messages(&store)
        .into_iter()
        .filter_map(|message| {
            let (_, _, role) = MAIL.iter().find(|(_, s, _)| *s == message.subject)?;
            (*role != MailboxRole::Inbox).then_some(Change::MessageMailbox(message.id, *role))
        })
        .collect();
    store
        .apply(
            acct_account(),
            &Patch {
                id: ChangeId::generate(),
                changes,
            },
        )
        .unwrap();
    Arc::new(store)
}

fn all_messages(store: &SqliteStore) -> Vec<Message> {
    let query = Query {
        filter: Filter::Account(acct_account()),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: 100,
        },
    };
    store
        .threads(&query, chrono::Utc::now())
        .unwrap()
        .items
        .into_iter()
        .flat_map(|summary| store.thread(summary.id).unwrap().messages)
        .map(|id| store.message(id).unwrap())
        .collect()
}

/// The subjects the store holds in `role`, newest first.
fn held_in(store: &SqliteStore, role: MailboxRole) -> Vec<String> {
    all_messages(store)
        .into_iter()
        .filter(|m| m.mailbox == role)
        .map(|m| m.subject)
        .collect()
}

/// How many deletions forever wait in the outbox, and how many server addresses they name.
fn queued_deletions(store: &SqliteStore) -> (usize, usize) {
    let horizon = chrono::DateTime::from_timestamp(253_402_300_799, 0).unwrap();
    store
        .outbox_due(acct_account(), horizon)
        .unwrap()
        .iter()
        .filter_map(|entry| match &entry.op {
            ProtoOp::Destroy { remotes } => Some(remotes.len()),
            _ => None,
        })
        .fold((0, 0), |(entries, remotes), n| (entries + 1, remotes + n))
}

fn open() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    (harness, dir, store)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn press(harness: &mut Harness, selector: &str) {
    let at = centre(harness, selector);
    harness.click(at);
    harness.advance(ms(600));
}

fn labelled(label: &str) -> String {
    format!("[*|aria-label=\"{label}\"]")
}

fn place(name: &str) -> String {
    format!("[*|data-place=\"{name}\"]")
}

fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

fn rows(harness: &Harness) -> usize {
    harness.count(".list .ds-thread")
}

const SHEET: &str = ".destroy-sheet";

fn sheet_text(harness: &Harness) -> String {
    harness.text_of(SHEET).unwrap_or_default()
}

/// What row `n`'s menu offers, by name, the menu left open.
fn offered(harness: &mut Harness, n: usize) -> Vec<String> {
    row_menu::open_row_menu(harness, &row(n));
    row_menu::menu_names(harness)
}

#[test]
fn empty_trash_asks_naming_the_count_and_then_the_list_empties_with_no_undo() {
    let (mut harness, _dir, store) = open();
    press(&mut harness, &place("Trash"));
    assert_eq!(rows(&harness), 3, "Trash shows its three conversations");
    let before = held_in(&store, MailboxRole::Trash);
    assert_eq!(before.len(), 3);

    // Asking deletes nothing.
    press(
        &mut harness,
        &format!(".list-col {}", labelled("Empty Trash")),
    );
    assert_eq!(harness.count(SHEET), 1, "Empty Trash put up no sheet");
    let said = sheet_text(&harness);
    assert!(said.contains("Empty Trash?"), "{said}");
    assert!(said.contains("3 messages in Trash"), "the count: {said}");
    assert!(said.contains("cannot be undone"), "{said}");
    assert_eq!(held_in(&store, MailboxRole::Trash), before);
    assert_eq!(queued_deletions(&store), (0, 0));

    // Esc answers no.
    harness.key(Key::Escape);
    harness.advance(ms(400));
    assert_eq!(harness.count(SHEET), 0, "Esc left the sheet up");
    assert_eq!(held_in(&store, MailboxRole::Trash), before);
    assert_eq!(rows(&harness), 3);

    // Asked again, and answered yes.
    press(
        &mut harness,
        &format!(".list-col {}", labelled("Empty Trash")),
    );
    press(
        &mut harness,
        &format!("{SHEET} {}", labelled("Empty Trash")),
    );
    harness.advance(ms(600));
    assert_eq!(harness.count(SHEET), 0);
    assert_eq!(rows(&harness), 0, "the list is empty");
    assert_eq!(held_in(&store, MailboxRole::Trash), Vec::<String>::new());
    assert_eq!(
        held_in(&store, MailboxRole::Inbox),
        vec![MAIL[0].1.to_owned()],
        "the inbox is untouched"
    );
    // One deletion per conversation, each naming its message's server address.
    assert_eq!(queued_deletions(&store), (3, 3));

    let toast = harness.text_of(".ds-toast").unwrap_or_default();
    assert!(toast.contains("Deleted forever"), "{toast}");
    assert_eq!(harness.count(".ds-toast-tab"), 0, "no Undo is offered");
    // And Ctrl Z has nothing of it to take back.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    harness.advance(ms(600));
    assert_eq!(held_in(&store, MailboxRole::Trash), Vec::<String>::new());
    assert_eq!(rows(&harness), 0);
}

/// Outside Trash and Spam nothing offers to delete forever: the inbox's delete is a move to
/// Trash. In Trash, a row is deleted forever from its menu once asked, with no Undo.
#[test]
fn only_in_trash_a_row_is_deleted_forever_from_its_menu_once_asked() {
    let (mut harness, _dir, store) = open();

    // The inbox offers no delete forever.
    assert_eq!(rows(&harness), 1, "the inbox holds one conversation");
    assert_eq!(
        harness.count(&labelled("Empty Trash")),
        0,
        "the inbox offers Empty Trash"
    );
    let offered_in_the_inbox = offered(&mut harness, 1);
    assert!(
        offered_in_the_inbox.iter().any(|op| op == "Trash"),
        "the inbox's delete is a move to Trash: {offered_in_the_inbox:?}"
    );
    assert!(
        !offered_in_the_inbox
            .iter()
            .any(|op| op.starts_with("Delete forever")),
        "the inbox offers delete forever: {offered_in_the_inbox:?}"
    );
    harness.key(Key::Escape);
    settle_until(&mut harness, |h| h.count(".ds-menu") == 0);

    // In Trash, the second row from its menu.
    press(&mut harness, &place("Trash"));
    let offered = offered(&mut harness, 2);
    assert!(
        offered.iter().any(|op| op == "Delete forever…"),
        "Trash's row offers no delete forever: {offered:?}"
    );
    row_menu::press_menu_item(&mut harness, "Delete forever…");
    harness.advance(ms(600));
    let said = sheet_text(&harness);
    assert!(said.contains("Delete this conversation forever?"), "{said}");
    assert!(said.contains("1 message in Trash"), "{said}");
    assert_eq!(
        held_in(&store, MailboxRole::Trash).len(),
        3,
        "not before it is answered"
    );

    press(
        &mut harness,
        &format!("{SHEET} {}", labelled("Delete forever")),
    );
    harness.advance(ms(600));
    assert_eq!(rows(&harness), 2, "Trash's rows after the delete");
    assert_eq!(
        held_in(&store, MailboxRole::Trash),
        vec![MAIL[1].1.to_owned(), MAIL[3].1.to_owned()],
        "the second row, and only it"
    );
    assert_eq!(harness.count(".ds-toast-tab"), 0, "no Undo is offered");
}

//! The launcher's unread count, from the real window on Blitz (`ds_harness::Harness`): the window
//! counts its inbox when it opens, and again when the user reads a conversation or takes that back.
//!
//! The window is handed a recorder as its launcher, so nothing reaches a dock or the session bus;
//! the message a count becomes is a table in `launcher/unity.rs`. It is handed no directories, so
//! it writes no file anywhere.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;
use ds_blitz::{NetPolicy, PrintOutcome};
use mail_app::ui::launcher::{Badge, Launcher, Unread};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

const INBOX: [(&str, &str); 3] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Every count the window handed its launcher, in order.
#[derive(Default)]
struct Recorder(Mutex<Vec<Unread>>);

impl Badge for Recorder {
    fn show(&self, unread: Unread) {
        self.0.lock().unwrap().push(unread);
    }
}

impl Recorder {
    fn seen(&self) -> Vec<u64> {
        self.0.lock().unwrap().iter().map(|u| u.0).collect()
    }
}

/// A store in `dir` with one account, its identity and capabilities, and [`INBOX`], all unread.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
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
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
        let caps = AccountCaps {
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
        };
        db.execute(
            "INSERT INTO account_caps (account, caps, observed_at)
             VALUES (?1, ?2, datetime('now'))",
            rusqlite::params![ACCOUNT.to_string(), serde_json::to_string(&caps).unwrap()],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <launcher{n}@example.test>\r\n\r\nThe body.\r\n"
        );
        absorb(
            &store,
            ACCOUNT,
            MailboxRef {
                account: ACCOUNT,
                path: "INBOX".to_owned(),
            },
            Some(SyncCursor::Pop),
            vec![Arrival {
                remote: RemoteRef::Pop {
                    uidl: format!("launcher{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// The window over a freshly seeded store, with a recorder for its launcher.
fn open() -> (Harness, tempfile::TempDir, Arc<Recorder>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let recorder = Arc::new(Recorder::default());
    // No test may open the system's print dialog.
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(Launcher(Arc::clone(&recorder) as Arc<dyn Badge>));
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let harness = Harness::new(mail_app::ui::native::root, config);
    (harness, dir, recorder)
}

fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

#[test]
fn the_window_counts_its_inbox_and_counts_again_when_one_is_read_and_unread() {
    let (mut harness, _dir, recorder) = open();
    settle_until(&mut harness, |_| recorder.seen() == [3]);

    // Read the second conversation from its hover strip.
    harness.pointer_move(centre(&harness, &format!("{} .ds-thread-sub", row(2))));
    harness.advance(ms(300));
    let read = format!("{} .ds-strip [*|data-op=mark-read]", row(2));
    harness.click(centre(&harness, &read));
    settle_until(&mut harness, |_| recorder.seen() == [3, 2]);

    // Ctrl Z makes it unread again, and the count follows.
    harness.chord(&[Key::Ctrl], Key::Char('z'));
    settle_until(&mut harness, |_| recorder.seen() == [3, 2, 3]);

    // A write that leaves the count where it is (a pin) sends the launcher nothing.
    harness.pointer_move(centre(&harness, &format!("{} .ds-thread-sub", row(1))));
    harness.advance(ms(300));
    let pin = format!("{} .ds-strip [*|data-op=pin]", row(1));
    harness.click(centre(&harness, &pin));
    harness.advance(ms(600));
    let toast = harness.text_of(".ds-toast").unwrap_or_default();
    assert!(toast.contains("Pinned"), "the pin was not written: {toast}");
    assert_eq!(recorder.seen(), [3, 2, 3]);
}

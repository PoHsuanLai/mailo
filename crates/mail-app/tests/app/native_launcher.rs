//! The launcher's unread count, from the real window on Blitz (`ds_harness::Harness`): the window
//! counts its inbox when it opens, and again when the user reads a conversation or takes that back.
//!
//! The window is handed a recorder as its launcher, so nothing reaches a dock or the session bus;
//! the message a count becomes is a table in `launcher/unity.rs`. It is handed no directories, so
//! it writes no file anywhere.

use ds::prelude::ShortcutKey as Key;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::row_menu;
use drive::{Drive, PRIMARY};
use ds_blitz::{NetPolicy, PrintOutcome};
use mail_app::launcher::{Badge, Launcher, Unread};
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

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
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@example.test",
            None,
        );
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
        mail_store::testing::seed_caps(&store, acct_account(), &caps, chrono::Utc::now()).unwrap();
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
            acct_account(),
            MailboxRef {
                account: acct_account(),
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
        None,
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

#[test]
fn the_window_counts_its_inbox_and_counts_again_when_one_is_read_and_unread() {
    let (mut harness, _dir, recorder) = open();
    settle_until(&mut harness, |_| recorder.seen() == [3]);

    // Read the second conversation from its menu.
    row_menu::row_action(&mut harness, &row(2), "Mark as read");
    settle_until(&mut harness, |_| recorder.seen() == [3, 2]);

    // Ctrl Z makes it unread again, and the count follows.
    harness.chord(&[PRIMARY], Key::Char('z'));
    settle_until(&mut harness, |_| recorder.seen() == [3, 2, 3]);

    // A write that leaves the count where it is (a pin) sends the launcher nothing.
    row_menu::row_action(&mut harness, &row(1), "Pin");
    harness.advance(ms(600));
    let toast = harness.text_of(".ds-toast").unwrap_or_default();
    assert!(toast.contains("Pinned"), "the pin was not written: {toast}");
    assert_eq!(recorder.seen(), [3, 2, 3]);
}

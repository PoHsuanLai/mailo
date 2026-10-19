//! The launcher's unread count, from the real window on Blitz (`ds_harness::Harness`): the window
//! counts its inbox when it opens, and again when the user reads a conversation or takes that back.
//!
//! The window is handed a recorder as its launcher, so nothing reaches a dock or the session bus;
//! the message a count becomes is a table in `launcher/unity.rs`. It is handed no directories, so
//! it writes no file anywhere.

use ds::prelude::ShortcutKey as Key;
use ds_harness::Harness;

use crate::settle;
use settle::settle_until;

use crate::drive;
use crate::native::item;
use crate::native::ms;
use crate::opening::{Open, Wait, open_window};
use crate::row_menu;
use crate::store::{Body, Seed, seeded_with};
use drive::{Drive, PRIMARY};
use mail_app::launcher::{Badge, Launcher, Unread};
use mail_core::SqliteStore;
use std::sync::{Arc, Mutex};

const INBOX: [(&str, &str); 3] = [
    ("ada@example.test", "Flight to the conference"),
    ("grace@example.test", "The invoice for September"),
    ("alan@example.test", "Lunch on Thursday"),
];

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
    seeded_with(
        dir,
        Seed {
            inbox: &INBOX,
            prefix: "launcher",
            body: Body::Plain,
            ..Seed::default()
        },
    )
}

/// The window over a freshly seeded store, with a recorder for its launcher.
fn open() -> (Harness, tempfile::TempDir, Arc<Recorder>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let recorder = Arc::new(Recorder::default());
    let harness = open_window(
        &store,
        Open {
            with: Some(Box::new(|c| {
                c.with(Launcher(Arc::clone(&recorder) as Arc<dyn Badge>))
            })),
            wait: Wait::Nothing,
            ..Open::default()
        },
    );
    (harness, dir, recorder)
}

#[test]
fn the_window_counts_its_inbox_and_counts_again_when_one_is_read_and_unread() {
    let (mut harness, _dir, recorder) = open();
    settle_until(&mut harness, |_| recorder.seen() == [3]);

    // Read the second conversation from its menu.
    row_menu::row_action(
        &mut harness,
        &item(2),
        mail_app::ui::copy::menus::MARK_AS_READ,
    );
    settle_until(&mut harness, |_| recorder.seen() == [3, 2]);

    // Ctrl Z makes it unread again, and the count follows.
    harness.chord(&[PRIMARY], Key::Char('z'));
    settle_until(&mut harness, |_| recorder.seen() == [3, 2, 3]);

    // A write that leaves the count where it is (a pin) sends the launcher nothing.
    row_menu::row_action(&mut harness, &item(1), "Pin");
    harness.advance(ms(600));
    let toast = harness.text_of(".ds-toast").unwrap_or_default();
    assert!(toast.contains("Pinned"), "the pin was not written: {toast}");
    assert_eq!(recorder.seen(), [3, 2, 3]);
}

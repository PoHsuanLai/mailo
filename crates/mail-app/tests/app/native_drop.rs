//! Files dragged in from a file manager, on the real window (Blitz, through `ds_harness::Harness`):
//! files let go on an open composer are attached as the Attach dialog's are, a folder is refused
//! with a note, the page lights while files are over it, and a drop anywhere else attaches
//! nothing.
//!
//! The drag is fed as the window's hook feeds winit's events (`Harness::file_drag`). Every file
//! dropped is written into a `TempDir` first.

use ds::file_drop::drag::{DropAcceptance, FileDragInput, Offer};
use ds::prelude::Point;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use drive::Drive;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::path::{Path, PathBuf};
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

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// One account, its identity, and one message in the inbox, so the list has a row to drop on.
fn seeded(dir: &Path) -> Arc<SqliteStore> {
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
    }
    let now = chrono::Utc::now();
    let date = (now - chrono::Duration::hours(1)).to_rfc2822();
    let raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Flight to the conference\r\n\
         Date: {date}\r\nMessage-ID: <seed0@example.test>\r\n\r\nThe body.\r\n"
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
                uidl: "seed0".to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

/// The window, with a new message open. The `TempDir` holds the store and the files to drop.
fn composing() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
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
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == 1);
    harness.key(ds::prelude::ShortcutKey::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    harness.advance(ms(300));
    (harness, dir, store)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

/// Write `name` into `dir` holding `text`, and give its path.
fn file(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// Drag `paths` in at `at` and let go there. What the window told the platform just before the
/// release, which is whether the platform lets go onto the window at all.
fn drop_at(harness: &mut Harness, at: Point, paths: Vec<PathBuf>) -> DropAcceptance {
    harness.file_drag(FileDragInput::Entered { point: Some(at) });
    harness.file_drag(FileDragInput::Offered(Offer::Files(paths)));
    let told = harness.file_drag(FileDragInput::Moved { point: at });
    harness.file_drag(FileDragInput::Dropped);
    told
}

/// The draft `c` opened, as stored.
fn the_draft(store: &SqliteStore) -> Draft {
    let drafts = store.drafts(acct_account()).unwrap();
    assert_eq!(drafts.len(), 1, "{drafts:?}");
    drafts.into_iter().next().unwrap()
}

fn attached_names(store: &SqliteStore) -> Vec<String> {
    the_draft(store)
        .attachments
        .into_iter()
        .map(|attachment| attachment.name)
        .collect()
}

/// The Attached row's chips, as their text.
fn chips(harness: &Harness) -> usize {
    harness.count(".c-props [*|data-row=attached] .ds-chip")
}

/// Files dragged over the window light the composer, and over it target it; the light goes with
/// the drag. Two files dropped on the composer are both attached, in order. A folder is refused
/// with a note. Files dropped outside the composer attach nothing, and the same file let go on
/// the composer is attached.
#[test]
fn files_dragged_in_light_the_composer_and_only_files_dropped_on_it_are_attached() {
    let (mut harness, dir, store) = composing();
    let before = chips(&harness);

    // The light, over the list and then over the composer.
    let agenda = file(dir.path(), "agenda.txt", "Friday, ten o'clock");
    let drop_attr = |h: &Harness| h.attr(".cpage", "data-drop");
    assert_eq!(drop_attr(&harness), None, "nothing lit before a drag");
    let list = centre(&harness, ".list .ds-thread");
    harness.file_drag(FileDragInput::Entered { point: Some(list) });
    let over_list = harness.file_drag(FileDragInput::Offered(Offer::Files(vec![agenda.clone()])));
    assert_eq!(over_list, DropAcceptance::Refuse, "the list takes no files");
    assert_eq!(
        drop_attr(&harness).as_deref(),
        Some("accepts"),
        "the composer is not lit over the list"
    );
    let body = centre(&harness, ".c-body");
    let over_page = harness.file_drag(FileDragInput::Moved { point: body });
    assert_eq!(over_page, DropAcceptance::Copy, "the composer takes files");
    assert_eq!(
        drop_attr(&harness).as_deref(),
        Some("target"),
        "the composer is not the target over it"
    );
    harness.file_drag(FileDragInput::Left);
    assert_eq!(drop_attr(&harness), None, "the light went with the drag");
    assert_eq!(chips(&harness), before, "a drag that left attached a chip");

    // Two files dropped on the composer.
    let notes = file(dir.path(), "notes.md", "# Notes");
    let body = centre(&harness, ".c-body");
    let told = drop_at(&mut harness, body, vec![agenda, notes]);
    assert_eq!(told, DropAcceptance::Copy, "the composer took the drop");
    settle_until(&mut harness, |h| chips(h) == before + 2);
    let html = harness.html();
    assert!(
        html.contains("TXT · agenda.txt") && html.contains("MD · notes.md"),
        "the chips name the files:\n{html}"
    );
    assert_eq!(
        attached_names(&store),
        ["agenda.txt", "notes.md"],
        "two files dropped"
    );

    // A folder.
    let photos = dir.path().join("photos");
    std::fs::create_dir(&photos).unwrap();
    file(&photos, "inside.jpg", "not walked into");
    // The Attached row now sits above the body: find the body again.
    let body = centre(&harness, ".c-body");
    drop_at(&mut harness, body, vec![photos]);
    let note = "photos is a folder; attach the files in it instead";
    settle_until(&mut harness, |h| {
        h.text_of(".cpage .ds-inline-banner-body").as_deref() == Some(note)
    });
    assert_eq!(chips(&harness), before + 2, "the folder was attached");
    assert_eq!(
        attached_names(&store),
        ["agenda.txt", "notes.md"],
        "the folder was attached"
    );

    // Outside the composer, then the same file on it.
    let minutes = file(dir.path(), "minutes.txt", "Friday");
    let list = centre(&harness, ".list .ds-thread");
    let told = drop_at(&mut harness, list, vec![minutes.clone()]);
    assert_eq!(
        told,
        DropAcceptance::Refuse,
        "a drop on the list is refused"
    );
    harness.advance(ms(500));
    assert_eq!(
        chips(&harness),
        before + 2,
        "the list's drop attached a chip"
    );
    assert_eq!(
        attached_names(&store),
        ["agenda.txt", "notes.md"],
        "the list's drop attached a file"
    );
    let body = centre(&harness, ".c-body");
    drop_at(&mut harness, body, vec![minutes]);
    settle_until(&mut harness, |h| chips(h) == before + 3);
    assert_eq!(
        attached_names(&store),
        ["agenda.txt", "notes.md", "minutes.txt"],
        "the file let go on the composer"
    );
}

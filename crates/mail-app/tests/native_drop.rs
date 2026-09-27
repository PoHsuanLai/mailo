//! Files dragged in from a file manager, on the real window (Blitz, through `ds_native::Harness`):
//! files let go on an open composer are attached as the Attach dialog's are, a folder is refused
//! with a note, the page lights while files are over it, and a drop anywhere else attaches
//! nothing.
//!
//! The drag is fed as the window's hook feeds winit's events (`Harness::file_drag`). Every file
//! dropped is written into a `TempDir` first.

use ds::{DropAcceptance, FileDragInput, Offer, Point};
use ds_native::harness::settle_until;
use ds_native::{FocusFallback, Harness, HarnessConfig, NetPolicy, PrintOutcome, Viewport};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

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
    }
    let now = chrono::Utc::now();
    let date = (now - chrono::Duration::hours(1)).to_rfc2822();
    let raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Flight to the conference\r\n\
         Date: {date}\r\nMessage-ID: <seed0@example.test>\r\n\r\nThe body.\r\n"
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
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".ds-list .ds-row") == 1);
    harness.key(ds::Key::Char('c'));
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
    let drafts = store.drafts(ACCOUNT).unwrap();
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
    harness.count(".c-props .prop-row .ds-chip")
}

#[test]
fn two_files_dropped_on_the_composer_are_both_attached_in_order() {
    let (mut harness, dir, store) = composing();
    let agenda = file(dir.path(), "agenda.txt", "Friday, ten o'clock");
    let notes = file(dir.path(), "notes.md", "# Notes");
    let before = chips(&harness);

    let at = centre(&harness, ".c-body");
    let told = drop_at(&mut harness, at, vec![agenda, notes]);
    assert_eq!(told, DropAcceptance::Copy, "the composer took the drop");

    settle_until(&mut harness, |h| chips(h) == before + 2);
    let html = harness.html();
    assert!(
        html.contains("TXT · agenda.txt") && html.contains("MD · notes.md"),
        "the chips name the files:\n{html}"
    );
    assert_eq!(attached_names(&store), ["agenda.txt", "notes.md"]);
}

#[test]
fn a_folder_dropped_on_the_composer_is_refused_with_a_note() {
    let (mut harness, dir, store) = composing();
    let photos = dir.path().join("photos");
    std::fs::create_dir(&photos).unwrap();
    file(&photos, "inside.jpg", "not walked into");
    let before = chips(&harness);

    let at = centre(&harness, ".c-body");
    drop_at(&mut harness, at, vec![photos]);

    let note = "photos is a folder; attach the files in it instead";
    settle_until(&mut harness, |h| {
        h.text_of(".cpage .notice").as_deref() == Some(note)
    });
    assert_eq!(chips(&harness), before, "nothing was attached");
    assert!(attached_names(&store).is_empty());
}

#[test]
fn files_dragged_over_the_window_light_the_composer_and_over_it_target_it() {
    let (mut harness, dir, _store) = composing();
    let agenda = file(dir.path(), "agenda.txt", "Friday");
    let drop_attr = |h: &Harness| h.attr(".cpage", "data-drop");
    assert_eq!(drop_attr(&harness), None, "nothing lit before a drag");

    let list = centre(&harness, ".ds-list .ds-row");
    harness.file_drag(FileDragInput::Entered { point: Some(list) });
    let over_list = harness.file_drag(FileDragInput::Offered(Offer::Files(vec![agenda])));
    assert_eq!(over_list, DropAcceptance::Refuse, "the list takes no files");
    assert_eq!(drop_attr(&harness).as_deref(), Some("accepts"));

    let body = centre(&harness, ".c-body");
    let over_page = harness.file_drag(FileDragInput::Moved { point: body });
    assert_eq!(over_page, DropAcceptance::Copy);
    assert_eq!(drop_attr(&harness).as_deref(), Some("target"));

    harness.file_drag(FileDragInput::Left);
    assert_eq!(drop_attr(&harness), None, "the light went with the drag");
}

#[test]
fn files_dropped_outside_the_composer_attach_nothing() {
    let (mut harness, dir, store) = composing();
    let agenda = file(dir.path(), "agenda.txt", "Friday");
    let before = chips(&harness);

    let list = centre(&harness, ".ds-list .ds-row");
    let told = drop_at(&mut harness, list, vec![agenda.clone()]);
    assert_eq!(
        told,
        DropAcceptance::Refuse,
        "a drop on the list is refused"
    );
    harness.advance(ms(500));
    assert_eq!(chips(&harness), before, "the list's drop attached a chip");
    assert!(attached_names(&store).is_empty());

    // The same file is attachable: let go on the composer, it is attached.
    let body = centre(&harness, ".c-body");
    drop_at(&mut harness, body, vec![agenda]);
    settle_until(&mut harness, |h| chips(h) == before + 1);
    assert_eq!(attached_names(&store), ["agenda.txt"]);
}

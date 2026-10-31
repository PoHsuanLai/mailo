//! What the list says while an account's first mail is on its way, in the real window on Blitz.
//!
//! One account that has never been fetched, no mail: the pane holds outline rows and a line of
//! words, not an empty folder. The window is handed no directories, so nothing here writes
//! outside the `TempDir`.

use ds_blitz::{FocusFallback, NetPolicy};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};

use crate::settle;
use mail_core::{SqliteStore, Store};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use settle::settle_until;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crate::store::acct_account;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// A store with one IMAP account, which has a link, and no mail.
fn bare(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    with_account(&store, acct_account(), "me@example.test");
    mail_store::testing::seed_identity_for(
        &store,
        IdentityId::generate(),
        acct_account(),
        "me@example.test",
        None,
    );
    Arc::new(store)
}

/// An IMAP account at `address` with its link and the capabilities its preset expects.
fn with_account(store: &SqliteStore, id: AccountId, address: &str) {
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual(address, &manual, chrono::Utc::now());
    mail_store::testing::seed_account_plan(
        store,
        id.clone(),
        &preset.plan.address,
        &preset.plan,
        Some(chrono::Utc::now()),
    );
    store
        .put_caps(id, &preset.expected_caps, chrono::Utc::now())
        .unwrap();
}

/// A pass that stays unfinished until the returned sender is dropped (or a minute passes, so a
/// test that fails before dropping it leaves no thread behind for good). The placeholder rows
/// show for as long as an account's first fetch runs; the idle pass the window is otherwise
/// lent ends at once, and the rows are gone between two looks when the machine is busy.
fn held_pass() -> (mail_client::fetch::Pass, mpsc::Sender<()>) {
    let (release, waiting) = mpsc::channel::<()>();
    let waiting = std::sync::Mutex::new(waiting);
    let pass: mail_client::fetch::Pass = Arc::new(move |_store, _now, _account, _hooks| {
        let _ = waiting
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(60));
        Ok(Vec::new())
    });
    (pass, release)
}

#[test]
fn an_account_never_fetched_shows_placeholder_rows_and_no_caption() {
    let dir = tempfile::tempdir().unwrap();
    let store = bare(dir.path());
    let (pass, release) = held_pass();
    let contexts = mail_ui::ui::native::contexts_passing(
        mail_client::Client::open(store, mail_client::ClientConfig::default()),
        mail_ui_kit::view::Appearance::default(),
        None,
        None,
        mail_ui::ui::ask::Open::raise(),
        pass,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_ui::ui::native::root, config);
    harness.advance(Duration::from_millis(300));
    settle_until(&mut harness, |h| h.count(".ds-skeleton-row") > 0);

    let rows = harness.count(".ds-skeleton-row");
    assert!(rows >= 8, "{rows} placeholder rows");
    let rect = harness
        .rect(".ds-skeleton-row")
        .expect("a placeholder row is drawn");
    assert!(
        rect.size.height.0 > 20.0 && rect.size.width.0 > 100.0,
        "a placeholder row is not laid out: {rect:?}"
    );
    assert!(
        !harness.html().contains("Downloading your mail"),
        "a caption over the placeholder rows"
    );
    assert_eq!(
        harness.count(".list .ds-empty-state"),
        0,
        "a first fetch is not an empty mailbox"
    );
    drop(release);
}

/// An account added while the window is open (in the Add Account window, a commit every window
/// hears from the client) is said in a toast, and the window takes it in; the
/// accounts the window opened with are not.
#[test]
fn an_account_added_while_the_window_is_open_is_said() {
    let dir = tempfile::tempdir().unwrap();
    let store = bare(dir.path());
    let (pass, release) = held_pass();
    let contexts = mail_ui::ui::native::contexts_passing(
        mail_client::Client::open(Arc::clone(&store), mail_client::ClientConfig::default()),
        mail_ui_kit::view::Appearance::default(),
        None,
        None,
        mail_ui::ui::ask::Open::raise(),
        pass,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_ui::ui::native::root, config);
    harness.advance(Duration::from_millis(300));
    settle_until(&mut harness, |h| h.count(".ds-skeleton-row") > 0);
    assert!(
        !harness.html().contains("Added me@example.test"),
        "an account the window opened with is not news"
    );

    let other = account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b8"));
    with_account(&store, other, "you@example.test");
    settle_until(&mut harness, |h| {
        h.html().contains("Added you@example.test")
    });
    drop(release);
}

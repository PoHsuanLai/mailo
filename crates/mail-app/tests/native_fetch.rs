//! What the list says while an account's first mail is on its way, in the real window on Blitz.
//!
//! One account that has never been fetched, no mail: the pane holds outline rows and a line of
//! words, not an empty folder. The window is handed no directories, so nothing here writes
//! outside the `TempDir`.

use ds_blitz::{FocusFallback, NetPolicy};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

#[path = "support/settle.rs"]
mod settle;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use settle::settle_until;
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b7"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// A store with one IMAP account, which has a link, and no mail.
fn bare(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    let manual = presets::Manual {
        imap_host: "imap.nowhere.example".to_owned(),
        imap_port: 993,
        smtp_host: "smtp.nowhere.example".to_owned(),
        smtp_port: 465,
        login: None,
    };
    let preset = presets::manual("me@example.test", &manual, chrono::Utc::now());
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            [
                ACCOUNT.to_string(),
                preset.plan.address.clone(),
                serde_json::to_string(&preset.plan).unwrap(),
                chrono::Utc::now().to_rfc3339(),
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    store
        .put_caps(ACCOUNT, &preset.expected_caps, chrono::Utc::now())
        .unwrap();
    Arc::new(store)
}

#[test]
fn an_account_never_fetched_shows_placeholder_rows_and_a_caption() {
    let dir = tempfile::tempdir().unwrap();
    let store = bare(dir.path());
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
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
        harness.html().contains("Downloading your mail"),
        "no caption"
    );
    assert_eq!(
        harness.count(".list .ds-empty-state"),
        0,
        "a first fetch is not an empty mailbox"
    );
}

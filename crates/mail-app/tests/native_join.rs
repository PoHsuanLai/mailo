//! The "+" after a Space's account tiles, on Blitz: it offers the accounts this computer has that
//! the Space does not show, and picking one brings it into the Space.
//!
//! The window is handed no directories, so it writes no file anywhere; the store is a `TempDir`'s.

use ds::prelude::*;
use ds_blitz::NetPolicy;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};
use mail_app::ui::space::{Scope, Space, Spaces};
use mail_domain::{AccountId, presets};
use mail_store::SqliteStore;
use std::sync::Arc;
use std::time::Duration;

#[path = "support/settle.rs"]
mod settle;
use settle::settle_until;

#[path = "support/drive.rs"]
mod drive;
use drive::Drive;

const WORK: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e1"));
const HOME: AccountId = AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000e2"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

const PLUS: &str = r#"[aria-label="Add account"]"#;

fn store(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    for (id, address) in [(WORK, "me@work.example"), (HOME, "me@home.example")] {
        let manual = presets::Manual {
            imap_host: "imap.example".to_owned(),
            imap_port: 993,
            smtp_host: "smtp.example".to_owned(),
            smtp_port: 465,
            login: None,
        };
        let plan = presets::manual(address, &manual, chrono::Utc::now()).plan;
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, ?2, ?3, datetime('now'))",
                [
                    id.to_string(),
                    address.to_owned(),
                    serde_json::to_string(&plan).unwrap(),
                ],
            )
            .unwrap();
    }
    Arc::new(store)
}

/// The window over both accounts, in one Space that shows only Work.
fn open() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let spaces = Spaces {
        spaces: vec![Space {
            name: "Work".to_owned(),
            scope: Scope::Accounts(vec![WORK]),
            ..Space::default()
        }],
        ..Spaces::default()
    };
    let contexts = mail_app::ui::native::contexts(
        store(dir.path()),
        mail_app::ui::view::Appearance::default(),
        spaces,
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(Duration::from_millis(300));
    settle_until(&mut harness, |h| h.count(PLUS) > 0);
    (harness, dir)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

#[test]
fn plus_offers_the_account_the_space_does_not_show_and_picking_it_brings_it_in() {
    let (mut harness, _dir) = open();
    assert!(
        !harness.html().contains("me@home.example"),
        "Home is already shown"
    );

    harness.click(centre(&harness, PLUS));
    settle_until(&mut harness, |h| h.count(".ds-menu") == 1);
    let menu = harness.html();
    assert!(
        menu.contains("me@home.example"),
        "Home is not offered:\n{menu}"
    );
    assert!(menu.contains("Add New Account"), "{menu}");

    harness.click(centre(&harness, ".ds-menu .ds-menu-item"));
    settle_until(&mut harness, |h| h.count(".ds-menu") == 0);
    settle_until(&mut harness, |h| h.html().contains("me@home.example"));
    assert_eq!(
        harness.count(".acct-sheet"),
        0,
        "it opened Add account instead"
    );
}

#[test]
fn when_every_account_is_shown_plus_goes_straight_to_add_account() {
    let (mut harness, _dir) = open();
    harness.click(centre(&harness, PLUS));
    settle_until(&mut harness, |h| h.count(".ds-menu") == 1);
    harness.click(centre(&harness, ".ds-menu .ds-menu-item"));
    settle_until(&mut harness, |h| h.count(".ds-menu") == 0);

    harness.click(centre(&harness, PLUS));
    settle_until(&mut harness, |h| h.count(".acct-sheet") == 1);
    assert_eq!(
        harness.count(".ds-menu"),
        0,
        "a menu with nothing to offer opened"
    );
}

//! Every control is the size its place calls for, on Blitz (`native`), in the real window and the
//! Settings window:
//!
//! - nothing mailo sizes is drawn at Mini: its list, reader and composer, and every button (quire's
//!   own parts, a Space tile's badge among them, keep the sizes quire gives them);
//! - a Large button is a toolbar's, on the quiet `Toolbar` bezel: a push button inside content is
//!   Regular, and a dense row's is Small;
//! - the composer's own bar (Plain text, Attach, Emoji, Send) is Regular throughout;
//! - a glyph on a row, in the reader or in the composer is 14 px or larger: quire's 11 and 13 px
//!   rungs are not used there.
//!
//! The windows are `mail_app::ui::native::root` and `settings_root` over a store seeded in a
//! `TempDir`: nothing here touches the network, the real mail store or the real config.

use ds_blitz::NetPolicy;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};
use mail_app::ui::appearance::WindowDirs;
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

use crate::drive;
use drive::{Drive, Key};

use crate::settle;
use settle::settle_until;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

fn acct() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000ca"))
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A letter with a file on it, so its row draws the clip and the count.
fn letter(date: &str) -> String {
    format!(
        "From: Ada <ada@example.test>\r\nTo: me@example.test\r\nSubject: Flight to the conference\r\n\
         Date: {date}\r\nMessage-ID: <sizes@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=b\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe gate changed to B12.\r\n\
         --b\r\nContent-Type: application/pdf; name=boarding.pdf\r\n\
         Content-Disposition: attachment; filename=boarding.pdf\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n"
    )
}

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        mail_store::testing::seed_account(&store, acct(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct(),
            "me@example.test",
            None,
        );
    }
    let now = chrono::Utc::now();
    let date = (now - chrono::Duration::hours(1)).to_rfc2822();
    absorb(
        &store,
        acct(),
        MailboxRef {
            account: acct(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: "sizes".to_owned(),
            },
            raw: letter(&date).into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

fn harness(root: fn() -> dioxus::prelude::Element, dir: &std::path::Path) -> Harness {
    let contexts = mail_app::ui::native::contexts(
        seeded(dir),
        mail_app::ui::view::Appearance::default(),
        None,
        Some(WindowDirs {
            config: dir.join("config"),
            state: dir.join("state"),
        }),
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(root, config);
    harness.advance(ms(600));
    harness
}

/// The first element `selector` matches, named by its class and accessible name: what a failure
/// says, rather than the whole document.
fn named(harness: &Harness, selector: &str) -> String {
    format!(
        "class={:?} aria-label={:?} text={:?}",
        harness.attr(selector, "class"),
        harness.attr(selector, "aria-label"),
        harness.text_of(selector)
    )
}

/// The rules every screen keeps, checked on what `harness` draws now; `screen` names it.
fn sizes_hold(harness: &Harness, screen: &str) {
    // What mailo sizes: its own panes, and every button. quire's own parts keep the sizes quire
    // gives them (a Space tile's unread badge is Mini by quire's design).
    for mini in [
        ".list [*|data-size=\"mini\"]",
        ".reader [*|data-size=\"mini\"]",
        ".cpage [*|data-size=\"mini\"]",
        "button.ds-button[*|data-size=\"mini\"]",
    ] {
        assert_eq!(
            harness.count(mini),
            0,
            "{screen}: something is drawn at Mini: {}",
            named(harness, mini)
        );
    }
    let large = "button.ds-button[*|data-size=\"large\"]:not([*|data-variant=\"toolbar\"])";
    assert_eq!(
        harness.count(large),
        0,
        "{screen}: a Large button is not a toolbar's: {}",
        named(harness, large)
    );
    for place in [".list", ".reader", ".cpage"] {
        for side in ["11", "13"] {
            let retired = format!("{place} svg.ds-ic[*|data-size=\"{side}\"]");
            assert_eq!(
                harness.count(&retired),
                0,
                "{screen}: a {side} px glyph in {place}: parent of {}",
                named(harness, &retired)
            );
        }
    }
}

#[test]
fn every_control_in_the_window_is_the_size_its_place_calls_for() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = harness(mail_app::ui::native::root, dir.path());
    settle_until(&mut harness, |h| h.count(".list .ds-thread .clip") > 0);
    sizes_hold(&harness, "the list");

    // The reader, on the letter.
    let subject = ".list .ds-list-item[*|aria-posinset=\"1\"] .ds-thread .ds-thread-sub";
    let at = harness.centre(subject).unwrap();
    harness.click(at);
    settle_until(&mut harness, |h| h.count(".reader .msg-head") > 0);
    sizes_hold(&harness, "the reader");

    // The composer, and its own bar.
    harness.key(Key::Char('c'));
    settle_until(&mut harness, |h| h.count(".cpage .c-foot") > 0);
    sizes_hold(&harness, "the composer");
    let foot = harness.count(".cpage .c-foot > button.ds-button")
        + harness.count(".cpage .c-foot > * > button.ds-button");
    let regular = harness.count(".cpage .c-foot > button.ds-button[*|data-size=\"regular\"]")
        + harness.count(".cpage .c-foot > * > button.ds-button[*|data-size=\"regular\"]");
    assert!(
        foot >= 4,
        "the composer's bar has {foot} buttons:\n{}",
        harness.html()
    );
    assert_eq!(
        regular,
        foot,
        "the composer's bar is not Regular throughout:\n{}",
        harness.html()
    );
    assert_eq!(
        harness.count(".cpage .c-foot button.ds-button[*|data-answers=\"return\"]"),
        1,
        "the composer's bar fills one button, Send:\n{}",
        harness.html()
    );
}

#[test]
fn every_control_in_settings_is_the_size_its_place_calls_for() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = harness(mail_app::ui::native::settings_root, dir.path());
    for page in [
        "General",
        "Accounts",
        "Contacts",
        "Rules",
        "Keys and certificates",
        "Keyboard",
    ] {
        let row = format!(".ds-sidebar [*|aria-label=\"{page}\"]");
        settle_until(&mut harness, |h| h.count(&row) == 1);
        let at = harness.centre(&row).unwrap();
        harness.click(at);
        let shown = format!(".settings-page[*|data-page=\"{page}\"]");
        settle_until(&mut harness, |h| h.count(&shown) == 1);
        harness.advance(ms(300));
        sizes_hold(&harness, page);
    }
}

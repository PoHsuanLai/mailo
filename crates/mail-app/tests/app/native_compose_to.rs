//! What is typed in To that is not an address, driven in the real window on Blitz
//! (`ds_harness::Harness`): the field keeps it and keeps the keyboard, so it can be mended; it
//! marks itself invalid when it is left, with no banner; and Send stays put while it is there.
//!
//! The store is seeded in a `TempDir`, and no window is handed directories, so nothing here
//! writes a file anywhere else.

use ds::prelude::{Point, ShortcutKey as Key};
use ds_blitz::NetPolicy;
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};

use crate::drive::{Drive, PRIMARY};
use crate::one_message::{account, seeded};
use crate::settle::settle_until;
use mail_core::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// To's field.
const TO: &str = ".cpage .c-props input[*|aria-label=\"To\"]";

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The main window over `store` with a new message open, To holding the keyboard.
fn composing(store: &Arc<SqliteStore>) -> Harness {
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(store),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") == 1);
    harness.chord(&[PRIMARY], Key::Char('n'));
    settle_until(&mut harness, |h| h.count(".cpage .c-body") == 1);
    settle_until(&mut harness, |h| h.is_focused(TO));
    harness
}

fn typed(harness: &Harness) -> String {
    harness.attr(TO, "value").unwrap_or_default()
}

fn type_text(harness: &mut Harness, text: &str) {
    for c in text.chars() {
        harness.key(Key::Char(c));
    }
    harness.advance(ms(100));
}

/// A press in the body, away from To.
fn leave(harness: &mut Harness) {
    let body = harness
        .rect(".cpage .c-body")
        .unwrap_or_else(|| panic!("no body:\n{}", harness.html()));
    harness.click(Point {
        x: ds::prelude::Px(body.origin.x.0 + 40.0),
        y: ds::prelude::Px(body.origin.y.0 + 12.0),
    });
    harness.advance(ms(100));
}

/// A `/` in To is text like any other: Backspace takes it again, as often as it is typed. On a
/// Mac the field deletes through AppKit's key bindings, which the harness does not send.
#[cfg(not(target_os = "macos"))]
#[test]
fn a_slash_in_to_backspaces_away() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = composing(&store);
    for round in 0..4 {
        type_text(&mut harness, "/");
        assert_eq!(typed(&harness), "/", "round {round}: / was not typed");
        harness.chord(&[], Key::Backspace);
        harness.advance(ms(100));
        assert_eq!(typed(&harness), "", "round {round}: Backspace left the /");
        assert!(
            harness.is_focused(TO),
            "round {round}: To lost the keyboard"
        );
    }
}

/// Left holding what is not an address, To keeps it and marks itself invalid, with no banner;
/// back in it, typing over it mends it, and the mark goes once it is an address.
#[test]
fn what_is_not_an_address_marks_to_and_stays_mendable() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = composing(&store);
    type_text(&mut harness, "/");
    leave(&mut harness);
    assert_eq!(typed(&harness), "/", "leaving To dropped what was typed");
    assert_eq!(
        harness.attr(TO, "aria-invalid").as_deref(),
        Some("true"),
        "To is not marked:\n{}",
        harness.html()
    );
    assert_eq!(
        harness.count(".cpage .ds-inline-banner"),
        0,
        "a banner went up"
    );

    harness.click(harness.centre(TO).unwrap_or_else(|| panic!("no To")));
    settle_until(&mut harness, |h| h.is_focused(TO));
    harness.chord(&[PRIMARY], Key::Char('a'));
    type_text(&mut harness, "zz@example.test");
    assert_eq!(typed(&harness), "zz@example.test");
    leave(&mut harness);
    assert_ne!(
        harness.attr(TO, "aria-invalid").as_deref(),
        Some("true"),
        "an address left To marked"
    );
}

/// Send with To holding what is not an address sends nothing and marks To.
#[test]
fn send_stays_put_while_to_is_not_an_address() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let mut harness = composing(&store);
    type_text(&mut harness, "zz@");
    harness.chord(&[PRIMARY], Key::Enter);
    harness.advance(ms(300));
    assert_eq!(harness.count(".cpage"), 1, "the composer went away");
    assert_eq!(typed(&harness), "zz@");
    assert_eq!(harness.attr(TO, "aria-invalid").as_deref(), Some("true"));
    assert_eq!(
        harness.count(".cpage .ds-inline-banner"),
        0,
        "a banner went up"
    );
    let drafts = store.drafts(account()).unwrap_or_default();
    assert!(
        drafts
            .iter()
            .all(|draft| draft.state == mail_domain::SendState::Editing),
        "something was queued: {drafts:?}"
    );
}

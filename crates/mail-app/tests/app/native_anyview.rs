//! Attachments anyview shows, in the real window (the `anyview` feature): a stored bitmap, a kind
//! mailo does not decode itself, leads its row with a thumbnail drawn by anyview's card; a text
//! file has no picture, so it keeps the paperclip, and Peek shows it as anyview's card; an SVG is
//! no one's to draw and stays unpreviewed, in the strip and in Peek, as without the feature.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`. The harness has
//! no GPU, so a picture's pixels are not drawn; the card's own markup says what it was given.

use ds::prelude::ShortcutKey as Key;
use ds_harness::{Clock, Harness, HarnessConfig};

use super::native_preview::{
    PEEK, VIEW, acct_account, carrying, click, item, ms, open_row, with_account,
};
use crate::drive::Drive;
use crate::settle::settle_until;
use mail_core::{Arrival, SqliteStore, absorb};
use mail_domain::*;
use std::sync::Arc;

/// A 24-bit bitmap of one colour: a kind `mail_core::preview` does not decode.
fn bmp(width: u32, height: u32) -> Vec<u8> {
    let row = (width * 3).div_ceil(4) * 4;
    let image = row * height;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + image).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&image.to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend((0..image).map(|n| if n % 3 == 0 { 0xC8 } else { 0x40 }));
    out
}

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    let store = with_account(dir);
    let now = chrono::Utc::now();
    let date = (now - chrono::Duration::hours(1)).to_rfc2822();
    let raw = carrying(
        "Scans",
        "scans",
        &date,
        &[
            ("image/bmp", "scan.bmp", &bmp(16, 8)),
            ("text/plain", "notes.txt", b"first line\nsecond line\n"),
            (
                "image/svg+xml",
                "logo.svg",
                b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            ),
        ],
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
                uidl: "scans".to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

#[test]
fn a_part_mailo_does_not_draw_gets_anyviews_card_and_an_svg_stays_unpreviewed() {
    let dir = tempfile::tempdir().unwrap();
    let contexts = mail_app::ui::native::contexts(
        seeded(dir.path()),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    let harness = &mut harness;
    open_row(harness, 1);
    settle_until(harness, |h| h.count(".attachments .att-tile") == 3);

    // The bitmap: anyview's card, its picture alone.
    let card = format!("{} .att-thumb .anyview-pane", item(1));
    settle_until(harness, |h| h.count(&card) == 1);
    assert_eq!(harness.attr(&card, "data-body").as_deref(), Some("picture"));
    assert_eq!(
        harness.count(&format!("{card} .anyview-pane-facts")),
        0,
        "the strip's tile drew the facts"
    );
    assert_eq!(
        harness.count(&format!("{card} .anyview-pane-title")),
        0,
        "the strip's tile drew the name"
    );

    // Text and an SVG keep the paperclip: a picture is the only thing a tile shows.
    harness.advance(ms(600));
    for n in 2..=3 {
        assert_eq!(
            harness.count(&format!("{} .att-thumb", item(n))),
            0,
            "row {n} drew a thumbnail"
        );
        assert_eq!(
            harness.count(&format!("{} .att-mark", item(n))),
            1,
            "row {n} lost its paperclip"
        );
    }

    // Peek on the bitmap is anyview's card, with the file's facts; the right arrow moves to the
    // text file, also anyview's; then to the SVG, which no one draws; Esc closes.
    click(harness, &format!("{} .att-thumb", item(1)));
    settle_until(harness, |h| {
        h.count(".ds-peek .peek-card .anyview-pane") == 1
    });
    assert_eq!(
        harness.count(".ds-peek .anyview-pane-facts"),
        1,
        "the card has no facts"
    );
    assert_eq!(harness.count(".ds-peek .peek-picture"), 0);

    harness.key(Key::Right);
    settle_until(harness, |h| {
        h.attr(PEEK, "aria-label").as_deref() == Some("notes.txt")
            && h.count(".ds-peek .peek-card .anyview-pane") == 1
    });
    assert_ne!(
        harness
            .attr(".ds-peek .anyview-pane", "data-body")
            .as_deref(),
        Some("picture"),
        "the text file was drawn as a picture"
    );

    harness.key(Key::Right);
    settle_until(harness, |h| {
        h.attr(PEEK, "aria-label").as_deref() == Some("logo.svg")
            && h.text_of(".ds-peek .peek-note").as_deref()
                == Some(mail_app::ui::copy::status::NO_PREVIEW)
    });
    assert_eq!(
        harness.count(".ds-peek .anyview-pane"),
        0,
        "the SVG was drawn"
    );

    harness.key(Key::Escape);
    settle_until(harness, |h| h.count(PEEK) == 0);
}

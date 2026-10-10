//! Brand logos on the real window (Blitz, through `ds_harness::Harness`): a sender whose logo is
//! already drawn and cached shows it in place of their initial, in the reader's head and on the
//! sender card; a sender DMARC did not pass for keeps their initial.
//!
//! The window is opened over a store seeded in a `TempDir`, with its config directory and its
//! logo cache in `TempDir`s too, so nothing here reads or touches the real mail store, config or
//! cache. The logo is in the cache before the window opens, so nothing is looked up: the account's
//! plan is empty (no server name), and the topmost `Authentication-Results` is the one read.

use ds::prelude::Point;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use drive::Drive;
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
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

/// A logo in SVG Tiny PS: a blue square with a white disc.
const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.2" baseProfile="tiny-ps" viewBox="0 0 64 64"><title>Brand</title><rect width="64" height="64" fill="#1a73e8"/><circle cx="32" cy="32" r="16" fill="#ffffff"/></svg>"##;

/// The seeded inbox, newest first: (sender, subject, the fields above its own headers).
const INBOX: [(&str, &str, &str); 2] = [
    (
        "ada@brand.example",
        "Your order has shipped",
        "Authentication-Results: mx.provider.example; spf=pass smtp.mailfrom=brand.example;\r\n \
         dkim=pass header.d=brand.example; dmarc=pass header.from=brand.example\r\n",
    ),
    (
        "grace@brand.example",
        "Verify your account",
        "Authentication-Results: mx.provider.example; spf=fail smtp.mailfrom=brand.example;\r\n \
         dkim=none; dmarc=fail header.from=brand.example\r\n",
    ),
];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        mail_store::testing::seed_account(&store, acct_account(), "me@provider.example");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@provider.example",
            None,
        );
    }
    let now = chrono::Utc::now();
    for (n, (from, subject, fields)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "{fields}From: {from}\r\nTo: me@provider.example\r\nSubject: {subject}\r\n\
             Date: {date}\r\nMessage-ID: <brand{n}@brand.example>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("brand{n}"),
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

struct Opened {
    harness: Harness,
    _dirs: [tempfile::TempDir; 3],
}

/// The window with the switch `on` and `brand.example`'s logo drawn and cached.
fn open(on: mail_core::bimi::Setting) -> Opened {
    let mail = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let store = seeded(mail.path());
    mail_app::settings::change(&mail_app::settings::root_for(config.path()), |settings| {
        settings.reading.brand_logos = on.into();
    })
    .unwrap();
    let png = mail_runtime::bimi::draw(LOGO.as_bytes()).unwrap();
    mail_runtime::bimi::remember(
        cache.path(),
        "brand.example",
        Some(&png),
        chrono::Utc::now(),
    )
    .unwrap();
    let dirs = mail_app::ui::appearance::WindowDirs {
        config: config.path().to_owned(),
        state: config.path().join("state"),
    };
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::ui::view::Appearance::default(),
        None,
        Some(dirs),
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(mail_app::ui::native::BrandCache(cache.path().to_owned()));
    let config_h = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config_h);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    Opened {
        harness,
        _dirs: [mail, config, cache],
    }
}

fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-row")
}

/// Click the `n`th row at the start of its subject line, clear of its hover strip.
fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!("{} .ds-thread-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: ds::prelude::Px(rect.origin.x.0 + 24.0),
        y: ds::prelude::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

const HEAD_LOGO: &str = ".reader-meta .reader-logo img";
const HEAD_CHECKS: &str = ".reader-meta .sender-checks";

/// A cached logo takes the initial's place in the reader's head; then a sender DMARC did not pass
/// for keeps their initial.
#[test]
fn a_cached_logo_takes_the_initials_place_and_a_sender_dmarc_did_not_pass_for_keeps_it() {
    let Opened { mut harness, _dirs } = open(mail_core::bimi::Setting::On);

    // DMARC passed: the logo.
    open_row(&mut harness, 1);
    settle_until(&mut harness, |harness| harness.count(HEAD_LOGO) == 1);
    let src = harness.attr(HEAD_LOGO, "src").unwrap_or_default();
    assert!(
        src.starts_with("data:image/png;base64,"),
        "the logo's source: {src}"
    );
    assert_eq!(
        harness.attr(HEAD_LOGO, "alt").as_deref(),
        Some("Logo of brand.example"),
        "the logo's alt"
    );
    assert_eq!(
        harness
            .text_of(".reader-meta .reader-av")
            .unwrap_or_default()
            .trim(),
        "",
        "the initial is gone"
    );
    let rect = harness.rect(HEAD_LOGO).expect("the logo is laid out");
    assert!(
        rect.size.width.0 > 20.0 && rect.size.height.0 > 20.0,
        "the logo's size: {rect:?}"
    );

    // DMARC did not pass: the initial.
    open_row(&mut harness, 2);
    // The checks line lands once the blob has been read, and so has the logo's answer. The first
    // row's line was a pass, so wait for the second's.
    settle_until(&mut harness, |harness| {
        harness.count(HEAD_CHECKS) == 1
            && harness.attr(HEAD_CHECKS, "data-standing").as_deref() != Some("pass")
    });
    harness.advance(ms(300));
    assert_eq!(
        harness.count(".reader-logo"),
        0,
        "a logo for a sender DMARC did not pass for"
    );
    assert_eq!(
        harness
            .text_of(".reader-meta .reader-av")
            .as_deref()
            .map(str::trim),
        Some("G"),
        "the initial of a sender DMARC did not pass for"
    );
}

#[test]
fn with_the_switch_off_the_cached_logo_is_not_shown() {
    let Opened { mut harness, _dirs } = open(mail_core::bimi::Setting::Off);
    open_row(&mut harness, 1);
    settle_until(&mut harness, |harness| harness.count(HEAD_CHECKS) == 1);
    harness.advance(ms(300));
    assert_eq!(harness.count(".reader-logo"), 0);
    assert_eq!(
        harness
            .text_of(".reader-meta .reader-av")
            .as_deref()
            .map(str::trim),
        Some("A")
    );
}

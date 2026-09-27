//! Brand logos on the real window (Blitz, through `ds_native::Harness`): a sender whose logo is
//! already drawn and cached shows it in place of their initial, in the reader's head and on the
//! sender card; a sender DMARC did not pass for keeps their initial.
//!
//! The window is opened over a store seeded in a `TempDir`, with its config directory and its
//! logo cache in `TempDir`s too, so nothing here reads or touches the real mail store, config or
//! cache. The logo is in the cache before the window opens, so nothing is looked up: the account's
//! plan is empty (no server name), and the topmost `Authentication-Results` is the one read.

use ds::Point;
use ds_native::harness::settle_until;
use ds_native::{FocusFallback, Harness, HarnessConfig, NetPolicy, PrintOutcome, Viewport};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

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
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@provider.example', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@provider.example', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
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
            ACCOUNT,
            MailboxRef {
                account: ACCOUNT,
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
fn open(on: mail_app::bimi::Setting) -> Opened {
    let mail = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let store = seeded(mail.path());
    mail_app::bimi::save(config.path(), on).unwrap();
    let png = mail_runtime::bimi::draw(LOGO.as_bytes()).unwrap();
    mail_runtime::bimi::remember(
        cache.path(),
        "brand.example",
        Some(&png),
        chrono::Utc::now(),
    )
    .unwrap();
    let dirs = mail_app::appearance::WindowDirs {
        config: config.path().to_owned(),
        state: config.path().join("state"),
    };
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        Arc::clone(&store),
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        Some(dirs),
        mail_app::ui::Start::Inbox,
    )
    .with(printer)
    .with(mail_app::ui::native::BrandCache(cache.path().to_owned()));
    let config_h = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config_h);
    harness.advance(ms(300));
    Opened {
        harness,
        _dirs: [mail, config, cache],
    }
}

fn row(n: usize) -> String {
    format!(".ds-list > .row:nth-child({n}) .ds-row")
}

/// Click the `n`th row at the start of its subject line, clear of its hover strip.
fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!("{} .ds-row-sub", row(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: ds::Px(rect.origin.x.0 + 24.0),
        y: ds::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

const HEAD_LOGO: &str = ".reader-meta .reader-logo img";
const HEAD_CHECKS: &str = ".reader-meta .sender-checks";

#[test]
fn a_cached_logo_takes_the_initials_place_in_the_reader_head() {
    let Opened { mut harness, _dirs } = open(mail_app::bimi::Setting::On);
    open_row(&mut harness, 1);
    settle_until(&mut harness, |harness| harness.count(HEAD_LOGO) == 1);
    let src = harness.attr(HEAD_LOGO, "src").unwrap_or_default();
    assert!(src.starts_with("data:image/png;base64,"), "{src}");
    assert_eq!(
        harness.attr(HEAD_LOGO, "alt").as_deref(),
        Some("Logo of brand.example")
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
        "{rect:?}"
    );
}

#[test]
fn a_sender_dmarc_did_not_pass_for_keeps_the_initial() {
    let Opened { mut harness, _dirs } = open(mail_app::bimi::Setting::On);
    open_row(&mut harness, 2);
    // The checks line lands once the blob has been read, and so has the logo's answer.
    settle_until(&mut harness, |harness| harness.count(HEAD_CHECKS) == 1);
    harness.advance(ms(300));
    assert_eq!(harness.count(".reader-logo"), 0);
    assert_eq!(
        harness
            .text_of(".reader-meta .reader-av")
            .as_deref()
            .map(str::trim),
        Some("G")
    );
}

#[test]
fn with_the_switch_off_the_cached_logo_is_not_shown() {
    let Opened { mut harness, _dirs } = open(mail_app::bimi::Setting::Off);
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

#[test]
fn the_sender_card_shows_the_logo() {
    let Opened { mut harness, _dirs } = open(mail_app::bimi::Setting::On);
    let sender = format!("{} .ds-row-name", row(1));
    let at = harness
        .centre(&sender)
        .unwrap_or_else(|| panic!("{sender} is not drawn:\n{}", harness.html()));
    harness.pointer_move(at);
    settle_until(&mut harness, |harness| {
        harness.count(".ds-hovercard .card-brand img") == 1
    });
    let said = harness
        .text_of(".ds-hovercard .card-brand")
        .unwrap_or_default();
    assert!(said.contains("brand.example"), "{said}");
}

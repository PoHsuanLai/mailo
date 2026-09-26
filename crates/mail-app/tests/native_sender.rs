//! Sender trust on the real window (Blitz, through `ds_native::Harness`): the reader shows what
//! the receiving server checked about the sender, and the sender card blocks them.
//!
//! The window is opened over a store seeded in a `TempDir` and handed no directories, so it
//! writes no file anywhere; nothing here reads or touches the real mail store or config. The
//! account's plan is empty, as `native_harness.rs`'s is, so the window's poll finds no account
//! it can sync and reaches no server; with no server name to go on, the topmost
//! `Authentication-Results` is the one read (`mail_app::auth::receiver`).

use ds::Point;
use ds_native::harness::settle_until;
use ds_native::{FocusFallback, Harness, HarnessConfig, NetPolicy, PrintOutcome, Viewport};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The seeded inbox, newest first: (sender, subject, the fields above its own headers).
const INBOX: [(&str, &str, &str); 2] = [
    (
        "ada@sender.example",
        "Flight to the conference",
        "Authentication-Results: mx.provider.example; spf=pass smtp.mailfrom=sender.example;\r\n \
         dkim=pass header.d=sender.example; dmarc=pass header.from=sender.example\r\n",
    ),
    (
        "grace@bank.example",
        "Your account is locked",
        "Authentication-Results: mx.provider.example; spf=fail smtp.mailfrom=bank.example;\r\n \
         dkim=none; dmarc=fail header.from=bank.example\r\n\
         Received: from attacker.example by mx.provider.example; Fri, 25 Sep 2026 10:00:00 +0000\r\n\
         Authentication-Results: mx.provider.example; spf=pass; dkim=pass; dmarc=pass\r\n",
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
             Date: {date}\r\nMessage-ID: <seed{n}@sender.example>\r\n\r\nThe body of {subject}.\r\n"
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
                    uidl: format!("seed{n}"),
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

fn open() -> (Harness, tempfile::TempDir, Arc<SqliteStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    // Every window here has a print dialog of its own: none may open the system's.
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
    (harness, dir, store)
}

fn row(n: usize) -> String {
    format!(".ds-list > .row:nth-child({n}) .ds-row")
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
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

const HEAD_CHECKS: &str = ".reader-meta .sender-checks";

#[test]
fn the_reader_shows_what_the_receiving_server_checked() {
    let (mut harness, _dir, _store) = open();
    assert_eq!(
        harness.count(".sender-checks"),
        0,
        "checks before any thread was open"
    );
    open_row(&mut harness, 1);
    // Read from the stored message on a blocking thread, so it lands a frame or two later.
    settle_until(&mut harness, |harness| harness.count(HEAD_CHECKS) == 1);
    assert_eq!(
        harness.attr(HEAD_CHECKS, "data-standing").as_deref(),
        Some("pass")
    );
    let said = harness.text_of(HEAD_CHECKS).unwrap_or_default();
    assert!(
        said.contains("SPF pass")
            && said.contains("DKIM pass (sender.example)")
            && said.contains("DMARC pass")
            && said.contains("checked by mx.provider.example"),
        "{said}"
    );
    let rect = harness.rect(HEAD_CHECKS).expect("the line is laid out");
    assert!(
        rect.size.height.0 > 0.0 && rect.size.width.0 > 0.0,
        "{rect:?}"
    );
}

#[test]
fn a_forged_pass_under_the_servers_fail_shows_as_a_fail() {
    let (mut harness, _dir, _store) = open();
    open_row(&mut harness, 2);
    settle_until(&mut harness, |harness| harness.count(HEAD_CHECKS) == 1);
    assert_eq!(
        harness.attr(HEAD_CHECKS, "data-standing").as_deref(),
        Some("fail")
    );
    let said = harness.text_of(HEAD_CHECKS).unwrap_or_default();
    assert!(
        said.contains("DMARC fail") && !said.contains("pass"),
        "{said}"
    );
}

/// The sender card of row `n`, opened by resting the pointer on the sender's name.
fn open_sender_card(harness: &mut Harness, n: usize) {
    let sender = format!("{} .ds-row-name", row(n));
    harness.pointer_move(centre(harness, &sender));
    settle_until(harness, |harness| harness.count(".ds-hovercard") == 1);
}

/// The card's fourth action, after Pin, Their mail and Copy address.
const BLOCK: &str = ".ds-hovercard .acts .ds-menu-item:nth-child(4)";

#[test]
fn the_sender_card_shows_the_checks_and_offers_a_block() {
    let (mut harness, _dir, _store) = open();
    open_sender_card(&mut harness, 1);
    // The same line as the reader's, beside the card's flags; read off the thread that draws.
    settle_until(&mut harness, |harness| {
        harness.count(".ds-hovercard .sender-checks") == 1
    });
    assert_eq!(
        harness
            .attr(".ds-hovercard .sender-checks", "data-standing")
            .as_deref(),
        Some("pass")
    );
    let item = harness.text_of(BLOCK).unwrap_or_default();
    assert!(item.contains("Block sender"), "{item}");
}

/// The pointer goes from the sender's name to the card's Block item in ten steps, crossing the
/// rows below that the card sits over, and the card stays the sender's all the way. Leaving the
/// name used to re-enter the row's own hook, which swapped the sender card for a thread card at
/// once, so no card action could be reached (FINDINGS F162).
#[test]
fn the_sender_cards_actions_can_be_reached_across_the_rows_under_it() {
    let (mut harness, _dir, store) = open();
    open_sender_card(&mut harness, 1);
    settle_until(&mut harness, |harness| {
        harness.count(".ds-hovercard .sender-checks") == 1
    });
    let from = centre(&harness, &format!("{} .ds-row-name", row(1)));
    let to = centre(&harness, BLOCK);
    for step in 1..=10 {
        let t = step as f32 / 10.0;
        harness.pointer_move(Point {
            x: ds::Px(from.x.0 + (to.x.0 - from.x.0) * t),
            y: ds::Px(from.y.0 + (to.y.0 - from.y.0) * t),
        });
        harness.advance(ms(40));
        assert_eq!(
            harness.count(".ds-hovercard .sender-checks"),
            1,
            "the sender card went at step {step}:\n{}",
            harness.html()
        );
    }
    harness.advance(ms(400));
    assert_eq!(
        harness.count(".ds-hovercard .sender-checks"),
        1,
        "gone at rest"
    );

    let before = store.rules(ACCOUNT).unwrap().len();
    harness.click(centre(&harness, BLOCK));
    settle_until(&mut harness, |_| {
        store.rules(ACCOUNT).map(|r| r.len()).unwrap_or(0) == before + 1
    });
    assert_eq!(store.rules(ACCOUNT).unwrap().len(), before + 1);
}

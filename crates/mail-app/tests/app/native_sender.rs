//! Sender trust on the real window (Blitz, through `ds_harness::Harness`): the reader shows what
//! the receiving server checked about the sender, and the sender card blocks them.
//!
//! The window is opened over a store seeded in a `TempDir` and handed no directories, so it
//! writes no file anywhere; nothing here reads or touches the real mail store or config. The
//! account's plan is empty, as `native_harness.rs`'s is, so the window's poll finds no account
//! it can sync and reaches no server; with no server name to go on, the topmost
//! `Authentication-Results` is the one read (`mail_core::auth::receiver`).

use ds::prelude::Point;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use drive::Drive;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
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
            [acct_account().to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@provider.example', '\"default\"')",
            [
                IdentityId::generate().to_string(),
                acct_account().to_string(),
            ],
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
            acct_account(),
            MailboxRef {
                account: acct_account(),
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
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    settle_until(&mut harness, |h| h.count(".list .ds-thread") > 0);
    (harness, dir, store)
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

const HEAD_CHECKS: &str = ".reader-meta .sender-checks";

/// The reader shows what the receiving server checked: nothing before a thread is open; a pass
/// as a mark beside the name, named for a screen reader; and a forged pass under the server's
/// fail as a fail, in words.
#[test]
fn the_reader_shows_what_the_receiving_server_checked() {
    let (mut harness, _dir, _store) = open();
    assert_eq!(
        harness.count(".sender-checks"),
        0,
        "checks before any thread was open"
    );

    // A pass.
    open_row(&mut harness, 1);
    // Read from the stored message on a blocking thread, so it lands a frame or two later.
    settle_until(&mut harness, |harness| harness.count(HEAD_CHECKS) == 1);
    assert_eq!(
        harness.attr(HEAD_CHECKS, "data-standing").as_deref(),
        Some("pass"),
        "the first row's standing"
    );
    // A pass is a mark beside the name, named for a screen reader; no method names, no words.
    assert_eq!(
        harness.text_of(HEAD_CHECKS).unwrap_or_default().trim(),
        "",
        "a pass says nothing"
    );
    assert_eq!(
        harness
            .attr(&format!("{HEAD_CHECKS} .sender-mark"), "aria-label")
            .as_deref(),
        Some("Verified sender"),
        "the pass's mark is not named"
    );

    // A forged pass under the server's fail.
    open_row(&mut harness, 2);
    settle_until(&mut harness, |harness| {
        harness.attr(HEAD_CHECKS, "data-standing").as_deref() == Some("fail")
    });
    assert_eq!(harness.count(HEAD_CHECKS), 1, "the second row's checks");
    let said = harness.text_of(HEAD_CHECKS).unwrap_or_default();
    assert_eq!(
        said.trim(),
        "May not be from this sender",
        "the forged pass's words"
    );
}

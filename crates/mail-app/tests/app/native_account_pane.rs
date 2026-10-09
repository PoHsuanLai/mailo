//! Settings' Accounts pane on Blitz (`ds_harness::Harness`), driven as its user drives it: the
//! account's row pushes its page and the keyboard lands on the page's back button; Escape and ⌘[
//! go back and the keyboard returns to the row; the Remove question answers Escape itself and
//! leaves the page where it was. Both pages keep quire's content inset.

use ds::prelude::Point;
use ds_blitz::{NetPolicy, PrintOutcome};
use ds_harness::inset::{Policy, assert_insets};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query as Read, Viewport};

use crate::settle;
use settle::settle_until;

use crate::drive;
use drive::{Drive, Key};

use mail_app::ui::appearance::WindowDirs;
use mail_domain::id::account_id_from_uuid;
use mail_domain::{AccountPlan, AuthPlan, Incoming, Outgoing, SaslMech, Tls, Username};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

fn acct_ada() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}

const ADDRESS: &str = "ada@example.test";

/// The Settings window at the size it opens at.
const VIEW: Viewport = Viewport {
    width: 780,
    height: 620,
    scale_percent: 100,
};

const SIDEBAR_ACCOUNTS: &str = ".ds-sidebar [*|aria-label=\"Accounts\"]";
const ACCOUNTS: &str = ".settings-page[*|data-page=\"Accounts\"]";
const BACK: &str = "#accounts-pane-back-detail";
const SHOWN: &str = ".ds-pane-stack-page[*|data-presence=present]";

fn row() -> String {
    format!("#accounts-pane-open-{}", acct_ada())
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A store in `dir` with one IMAP account.
fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    let plan = AccountPlan {
        address: ADDRESS.to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example.test".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example.test".to_owned(),
            port: 587,
            tls: Tls::StartTlsRequired,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    };
    mail_store::testing::seed_account_plan(&store, acct_ada(), &ADDRESS, &plan, None);
    Arc::new(store)
}

/// The Settings window on Accounts. The `TempDir` must outlive it.
fn open() -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let dirs = WindowDirs {
        config: dir.path().join("config"),
        state: dir.path().join("state"),
    };
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        None,
        Some(dirs),
        mail_app::ui::Start::Inbox,
    )
    .with(printer);
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::settings_root, config);
    settle_until(&mut harness, |harness| harness.count(SIDEBAR_ACCOUNTS) == 1);
    click(&mut harness, SIDEBAR_ACCOUNTS);
    let row = row();
    settle_until(&mut harness, |harness| {
        harness.count(ACCOUNTS) == 1 && harness.count(&row) == 1
    });
    (harness, dir)
}

fn centre(harness: &Harness, selector: &str) -> Point {
    harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
}

fn click(harness: &mut Harness, selector: &str) {
    let at = centre(harness, selector);
    harness.click(at);
    harness.advance(ms(300));
}

/// Whether only the list is drawn: one page in the stack, and no back button.
fn at_root(harness: &Harness) -> bool {
    harness.count(".ds-pane-stack-page") == 1 && harness.count(BACK) == 0
}

/// Whether only the account's page is drawn, its slide done.
fn at_account(harness: &Harness) -> bool {
    harness.count(".ds-pane-stack-page") == 1 && harness.count(BACK) == 1
}

/// Push the account's page with its row and let the slide rest.
fn push(harness: &mut Harness) {
    click(harness, &row());
    settle_until(harness, at_account);
}

/// The account's row pushes its page; Escape and Command-[ each come back to the list. On the
/// page, Escape on the remove question answers it and stays on the page.
#[test]
fn the_row_pushes_the_page_and_escape_and_command_bracket_come_back_to_it() {
    let (mut harness, _dir) = open();
    assert!(at_root(&harness), "{}", harness.html());
    assert_insets(&harness, &Policy::quire());

    // The row pushes the page.
    push(&mut harness);
    let title = format!("{SHOWN} .ds-page-title");
    assert!(
        harness.html().contains(&format!(">{ADDRESS}</h2>")),
        "the page is not titled with the account:\n{}",
        harness.html()
    );
    assert_eq!(harness.count(&title), 1, "the page's title");
    settle_until(&mut harness, |harness| harness.is_focused(BACK));
    assert_insets(&harness, &Policy::quire());

    // Escape comes back, the keyboard on the row.
    harness.key(Key::Escape);
    settle_until(&mut harness, at_root);
    let row = row();
    settle_until(&mut harness, |harness| harness.is_focused(&row));

    // Command-[ comes back.
    push(&mut harness);
    settle_until(&mut harness, |harness| harness.is_focused(BACK));
    harness.chord(&[Key::Super], Key::Char('['));
    settle_until(&mut harness, at_root);

    // Escape on the remove question answers it and stays on the page.
    push(&mut harness);
    click(
        &mut harness,
        &format!("button[*|aria-label=\"Remove {ADDRESS}\"]"),
    );
    settle_until(&mut harness, |harness| harness.count(".ds-alert") == 1);
    harness.advance(ms(400));
    harness.key(Key::Escape);
    settle_until(&mut harness, |harness| harness.count(".ds-alert") == 0);
    harness.advance(ms(400));
    assert!(
        at_account(&harness),
        "Escape on the remove question went back past the page:\n{}",
        harness.html()
    );
}

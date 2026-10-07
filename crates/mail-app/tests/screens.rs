//! Picture generator (ignored): the main window, the composer, the search bar's panel, a menu and
//! a sheet, in light and dark, over fake mail in a scratch store. Set MAILO_SHOTS to a directory.

use ds::prelude::*;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Query, Viewport};
use ds_settings::Environment;
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

#[path = "support/drive.rs"]
mod drive;
#[path = "support/row_menu.rs"]
mod row_menu;
#[path = "support/settle.rs"]
mod settle;
use drive::{Drive, Key};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
}
const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

const INBOX: [(&str, &str, &str); 8] = [
    (
        "Ada Lovelace <ada@example.test>",
        "Flight to the conference",
        "I booked the Thursday flight. Landing at 14:20, so I can be at the venue before the opening talk.\r\n\r\nCan you meet me at the gate? Bring the slides.\r\n\r\nAda",
    ),
    (
        "Grace Hopper <grace@example.test>",
        "The invoice for September",
        "Attached is the invoice for September. The total is unchanged from last month.\r\n\r\nGrace",
    ),
    (
        "Alan Turing <alan@example.test>",
        "Lunch on Thursday",
        "Are you free at noon? The place by the river has a table.",
    ),
    (
        "Edsger Dijkstra <edsger@example.test>",
        "Notes from the review",
        "Three small things: naming, the error path, and the test that never fails.",
    ),
    (
        "Barbara Liskov <barbara@example.test>",
        "Re: substitution",
        "That holds as long as the contract is honoured.",
    ),
    (
        "Donald Knuth <don@example.test>",
        "Proofs enclosed",
        "The proofs are enclosed. Please check the second chapter.",
    ),
    (
        "Margaret Hamilton <margaret@example.test>",
        "Launch checklist",
        "The checklist is final. Nothing left open.",
    ),
    (
        "Linus <linus@example.test>",
        "Merge window",
        "The merge window opens on Monday.",
    ),
];

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute("INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, 'me@example.test', '{}', datetime('now'))", [acct_account().to_string()]).unwrap();
        db.execute("INSERT INTO identities (id, account, from_name, from_email, is_default) VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')", [IdentityId::generate().to_string(), acct_account().to_string()]).unwrap();
        let caps = AccountCaps {
            labels: ServerLabels::Supported,
            threads: ServerThreads::Jwz,
            watch: WatchMode::Idle,
            archive: ArchiveMeans::DropInbox,
            folders: FolderRoles::default(),
            condstore: Condstore::Supported,
            move_ext: MoveExt::Supported,
            expunge: ExpungeMeans::Forbidden,
            top: Supported::Absent,
            pipelining: Supported::Yes,
            connections: ConnectionBudget::default(),
            observed_at: chrono::Utc::now(),
        };
        db.execute("INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, datetime('now'))", rusqlite::params![acct_account().to_string(), serde_json::to_string(&caps).unwrap()]).unwrap();
    }
    let now = chrono::Utc::now();
    for (n, (from, subject, body)) in INBOX.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
        let raw = format!(
            "From: {from}\r\nTo: me@example.test\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <seed{n}@example.test>\r\n\r\n{body}\r\n"
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
    // A newsletter that asks for a read receipt and draws a remote image: the reader's banners.
    let date = (now - chrono::Duration::minutes(90)).to_rfc2822();
    let raw = format!(
        "From: Weekly Notes <notes@example.test>\r\nTo: me@example.test\r\nSubject: Weekly notes\r\nDate: {date}\r\nMessage-ID: <seednews@example.test>\r\nDisposition-Notification-To: notes@example.test\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Twelve new knits, and a pattern for the autumn.</p><img src=\"https://images.example.test/knit.png\" width=\"320\" height=\"120\">\r\n"
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
                uidl: "seednews".to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    // A few labels, for the label picker.
    {
        let db = store.connection();
        for name in ["Travel", "Receipts", "Family"] {
            db.execute(
                "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, ?3, '\"user\"')",
                rusqlite::params![
                    LabelId::generate().to_string(),
                    acct_account().to_string(),
                    name
                ],
            )
            .unwrap();
        }
    }
    Arc::new(store)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn open(dark: bool) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    );
    let mut env = Environment::default();
    if dark {
        env.system.scheme = Scheme::Dark;
    }
    let printer = mail_app::ui::native::Printer::with_dialog(|_, _| Ok(PrintOutcome::Cancelled));
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Wall)
        .with_net(NetPolicy::Local)
        .with_focus_fallback(FocusFallback::Ancestor)
        .with_contexts(contexts.with(printer))
        .with_context(env);
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(400));
    (harness, dir)
}

fn shot(harness: &mut Harness, dir: &str, name: &str, dark: bool) {
    harness.advance(ms(600));
    let file = format!("{dir}/{name}-{}.png", if dark { "dark" } else { "light" });
    harness.render().unwrap().save(file).unwrap();
}

fn click(harness: &mut Harness, selector: &str) {
    let c = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} not drawn"));
    harness.click(c);
    harness.advance(ms(300));
}

/// The first row's subject line, where a click opens it (the strip lies over the row's middle).
fn open_row(h: &mut Harness, n: usize) {
    let sub = h
        .rect(&format!(
            ".list .ds-list-item[*|aria-posinset=\"{n}\"] .ds-thread-sub"
        ))
        .expect("row");
    h.click(Point {
        x: Px(sub.origin.x.0 + 24.0),
        y: Px(sub.origin.y.0 + sub.size.height.0 / 2.0),
    });
    h.advance(ms(500));
}

/// Row `n` of the list.
fn row(n: usize) -> String {
    format!(".list .ds-list-item[*|aria-posinset=\"{n}\"]")
}

#[test]
#[ignore = "picture generator: set MAILO_SHOTS to a directory and run with --ignored"]
fn screens() {
    let out = std::env::var("MAILO_SHOTS").expect("MAILO_SHOTS names the directory");
    for dark in [false, true] {
        // The list, nothing open: the reader's empty state.
        let (mut h, _d) = open(dark);
        shot(&mut h, &out, "inbox-list", dark);
        // An empty folder: the list's empty state.
        click(&mut h, "[*|data-place=\"Trash\"]");
        shot(&mut h, &out, "empty-state", dark);

        // The reader, then the same message in the centre peek.
        let (mut h, _d) = open(dark);
        open_row(&mut h, 1);
        shot(&mut h, &out, "reader", dark);
        click(&mut h, "[*|aria-label=\"Centre peek\"]");
        shot(&mut h, &out, "peek", dark);

        // A message with banners: remote images blocked and a read receipt asked for.
        let (mut h, _d) = open(dark);
        open_row(&mut h, 2);
        shot(&mut h, &out, "reader-banners", dark);

        // The row's menus: its actions, then labels and snooze picked from them.
        let (mut h, _d) = open(dark);
        row_menu::open_row_menu(&mut h, &row(1));
        h.advance(ms(300));
        shot(&mut h, &out, "row-menu", dark);
        h.key(Key::Escape);
        h.advance(ms(400));
        row_menu::row_action(&mut h, &row(1), "Label…");
        shot(&mut h, &out, "label-picker", dark);
        h.key(Key::Escape);
        h.advance(ms(400));
        row_menu::row_action(&mut h, &row(1), "Snooze…");
        shot(&mut h, &out, "snooze-menu", dark);

        // A toast with its action: Archive, then Undo on the toast.
        let (mut h, _d) = open(dark);
        row_menu::row_action(&mut h, &row(1), "Archive");
        h.advance(ms(500));
        shot(&mut h, &out, "toast-action", dark);

        // Composer.
        let (mut h, _d) = open(dark);
        open_row(&mut h, 1);
        h.key(Key::Char('c'));
        h.advance(ms(500));
        click(&mut h, ".c-body");
        for c in "Here is where we landed".chars() {
            h.key(if c == ' ' { Key::Space } else { Key::Char(c) });
        }
        shot(&mut h, &out, "composer", dark);

        // The search bar's panel (⌘K), then the Add Account sheet from it.
        let (mut h, _d) = open(dark);
        h.chord(&[Key::Ctrl], Key::Char('k'));
        h.advance(ms(400));
        for c in "ar".chars() {
            h.key(Key::Char(c));
        }
        shot(&mut h, &out, "command-menu", dark);
        h.key(Key::Escape);
        h.advance(ms(400));
        h.chord(&[Key::Ctrl], Key::Char('k'));
        h.advance(ms(400));
        for c in "add acc".chars() {
            h.key(if c == ' ' { Key::Space } else { Key::Char(c) });
        }
        h.advance(ms(200));
        h.key(Key::Enter);
        shot(&mut h, &out, "add-account-sheet", dark);
        // The Space's menu, through the Space's name.
        h.key(Key::Escape);
        h.advance(ms(400));
        click(&mut h, ".space-name");
        shot(&mut h, &out, "space-menu", dark);
    }
}

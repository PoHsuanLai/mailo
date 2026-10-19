//! Picture generator (ignored): the main window, the composer, the search bar's panel, a menu and
//! a sheet, in light and dark, over fake mail in a scratch store. Set MAILO_SHOTS to a directory.
//! One test runs on its fake mail as an ordinary one: an older message opened in a thread.

use ds::prelude::*;
use ds_blitz::{FocusFallback, NetPolicy, PrintOutcome};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};
use ds_settings::Environment;
use mail_core::{Arrival, absorb};
use mail_core::{SqliteStore, Store};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

use crate::drive;
use crate::row_menu;
use drive::{Drive, Key, PRIMARY};

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

/// Stores `raw` in the Inbox as POP message `uidl`.
fn arrive(store: &SqliteStore, uidl: &str, raw: Vec<u8>, now: chrono::DateTime<chrono::Utc>) {
    absorb(
        store,
        acct_account(),
        MailboxRef {
            account: acct_account(),
            path: "INBOX".to_owned(),
        },
        Some(SyncCursor::Pop),
        vec![Arrival {
            remote: RemoteRef::Pop {
                uidl: uidl.to_owned(),
            },
            raw,
        }],
        false,
        now,
    )
    .unwrap();
}

fn seeded(dir: &std::path::Path) -> Arc<SqliteStore> {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        mail_store::testing::seed_account(&store, acct_account(), "me@example.test");
        mail_store::testing::seed_identity_for(
            &store,
            IdentityId::generate(),
            acct_account(),
            "me@example.test",
            None,
        );
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
        mail_store::testing::seed_caps(&store, acct_account(), &caps, chrono::Utc::now()).unwrap();
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
    // A conversation of three, read: the older two fold to a line each, the newest quotes the one
    // it answers and carries a file (rows 3; the reader's thread shots).
    let said = |hours: i64| (now - chrono::Duration::minutes(hours)).to_rfc2822();
    let thread = [
        (
            "offsite1",
            format!(
                "From: Alan Turing <alan@example.test>\r\nTo: me@example.test\r\nSubject: Venue for the offsite\r\nDate: {}\r\nMessage-ID: <offsite1@example.test>\r\n\r\nShall we hold the offsite at the lake house? It has room for twelve and a long table.\r\n\r\nAlan\r\n",
                said(26 * 60)
            ),
        ),
        (
            "offsite2",
            format!(
                "From: Grace Hopper <grace@example.test>\r\nTo: me@example.test\r\nSubject: Re: Venue for the offsite\r\nDate: {}\r\nMessage-ID: <offsite2@example.test>\r\nIn-Reply-To: <offsite1@example.test>\r\nReferences: <offsite1@example.test>\r\n\r\nThe lake house works. The 14th and 15th are free.\r\n\r\nOn Monday, Alan Turing wrote:\r\n> Shall we hold the offsite at the lake house? It has room for twelve and a long table.\r\n>\r\n> Alan\r\n",
                said(5 * 60)
            ),
        ),
        (
            "offsite3",
            format!(
                "From: Ada Lovelace <ada@example.test>\r\nTo: me@example.test\r\nSubject: Re: Venue for the offsite\r\nDate: {}\r\nMessage-ID: <offsite3@example.test>\r\nIn-Reply-To: <offsite2@example.test>\r\nReferences: <offsite1@example.test> <offsite2@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nBooked for the 14th. The floor plan is attached; the long table is in the east room.\r\n\r\nAda\r\n\r\nOn Tuesday, Grace Hopper wrote:\r\n> The lake house works. The 14th and 15th are free.\r\n--mix\r\nContent-Type: application/pdf; name=\"Floor plan.pdf\"\r\nContent-Disposition: attachment; filename=\"Floor plan.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQKJWZha2UK\r\n--mix--\r\n",
                said(100)
            ),
        ),
    ];
    for (uidl, raw) in thread {
        arrive(&store, uidl, raw.into_bytes(), now);
    }
    let offsite = store
        .thread_by_rfc_id(acct_account(), "offsite3@example.test")
        .unwrap()
        .expect("the offsite conversation was stored");
    mail_core::act::perform(&store, offsite, Op::SetRead(ReadState::Read));
    // An invitation: one row of answers and Details (row 4).
    let invite = format!(
        "From: Ada Lovelace <ada@example.test>\r\nTo: me@example.test\r\nSubject: Invitation: Design review\r\nDate: {}\r\nMessage-ID: <review-mail@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n--alt\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\nYou are invited to Design review.\r\n--alt\r\nContent-Type: text/calendar; charset=UTF-8; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nPRODID:-//Example//Calendar//EN\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:review-shots@example.test\r\nSEQUENCE:0\r\nDTSTAMP:20260920T101500Z\r\nDTSTART;TZID=Europe/Berlin:20261105T140000\r\nDTEND;TZID=Europe/Berlin:20261105T150000\r\nSUMMARY:Design review\r\nLOCATION:Room 2\r\nDESCRIPTION:We walk through the new reader and the compose window.\r\nSTATUS:CONFIRMED\r\nORGANIZER;CN=Ada Lovelace:mailto:ada@example.test\r\nATTENDEE;CN=Ada Lovelace;PARTSTAT=ACCEPTED;ROLE=CHAIR:mailto:ada@example.test\r\nATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.test\r\nATTENDEE;CN=Grace Hopper;PARTSTAT=TENTATIVE:mailto:grace@example.test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--alt--\r\n",
        said(110)
    );
    arrive(&store, "review", invite.into_bytes(), now);
    // Old conversations already read, for Tidy Up to list.
    for (n, subject) in [
        "Quarterly report",
        "Your order has shipped",
        "Book club in May",
    ]
    .iter()
    .enumerate()
    {
        let date = (now - chrono::Duration::days(30 + n as i64)).to_rfc2822();
        let id = format!("old{n}@example.test");
        let raw = format!(
            "From: Old Friends <old@example.test>\r\nTo: me@example.test\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{id}>\r\n\r\nFrom a while ago.\r\n"
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
                    uidl: format!("old{n}"),
                },
                raw: raw.into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
        let thread = store
            .thread_by_rfc_id(acct_account(), &id)
            .unwrap()
            .expect("the old conversation was stored");
        mail_core::act::perform(&store, thread, Op::SetRead(ReadState::Read));
    }
    // A few labels, for the label picker.
    {
        for name in ["Travel", "Receipts", "Family"] {
            mail_store::testing::seed_label(
                &store,
                LabelId::generate(),
                acct_account(),
                name,
                mail_domain::LabelOrigin::User,
            );
        }
    }
    Arc::new(store)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn open(dark: bool) -> (Harness, tempfile::TempDir) {
    open_with(dark, |_| {})
}

/// [`open`], with `prepare` run on the store first.
fn open_with(dark: bool, prepare: impl FnOnce(&SqliteStore)) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    prepare(&store);
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        None,
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
        click(&mut h, ".reader-head [*|aria-label=\"More\"]");
        shot(&mut h, &out, "reader-more", dark);
        row_menu::press_menu_item(&mut h, "View");
        h.advance(ms(300));
        click(&mut h, ".ds-menu .ds-menu-item:nth-child(2)");
        shot(&mut h, &out, "peek", dark);

        // A message with banners: remote images blocked and a read receipt asked for.
        let (mut h, _d) = open(dark);
        open_row(&mut h, 2);
        shot(&mut h, &out, "reader-banners", dark);

        // A read conversation of three: the older two one line each, the newest open with its
        // quote folded and its file in the strip; then an older one opened by a press.
        let (mut h, _d) = open(dark);
        open_row(&mut h, 3);
        shot(&mut h, &out, "reader-thread", dark);
        click(&mut h, ".reader-body .fold-line");
        shot(&mut h, &out, "reader-thread-opened", dark);

        // An invitation: one row, then its Details.
        let (mut h, _d) = open(dark);
        open_row(&mut h, 4);
        shot(&mut h, &out, "reader-invite", dark);
        click(&mut h, "[*|aria-label=\"Show Details\"]");
        shot(&mut h, &out, "reader-invite-details", dark);

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
        row_menu::press_menu_item(&mut h, mail_app::ui::copy::menus::CHOOSE_TIME);
        h.advance(ms(400));
        shot(&mut h, &out, "snooze-choose-time", dark);

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

        // A message held to leave tomorrow, opened from Scheduled: what it says, and when.
        let (mut h, _d) = open_with(dark, |store| {
            let now = chrono::Utc::now();
            let held = mail_core::compose::draft_new(
                store,
                acct_account(),
                &[Address {
                    name: Some("Grace Hopper".into()),
                    email: "grace@example.test".into(),
                }],
                "Agenda for the offsite",
                "Morning: the plan for the year.\nAfternoon: the lake.\n",
                now,
            )
            .unwrap();
            let at = now + chrono::TimeDelta::days(1);
            store
                .set_send_state(held.id, &SendState::Scheduled { at }, now)
                .unwrap();
        });
        click(&mut h, "[*|data-place=\"Scheduled\"]");
        open_row(&mut h, 1);
        shot(&mut h, &out, "scheduled-held", dark);

        // The search bar's panel (⌘K) with nothing typed, then with a query, then the Add Account
        // sheet from it.
        let (mut h, _d) = open(dark);
        h.chord(&[PRIMARY], Key::Char('k'));
        h.advance(ms(400));
        shot(&mut h, &out, "command-empty", dark);
        for c in "ar".chars() {
            h.key(Key::Char(c));
        }
        shot(&mut h, &out, "command-menu", dark);
        h.key(Key::Escape);
        h.advance(ms(400));
        h.chord(&[PRIMARY], Key::Char('k'));
        h.advance(ms(400));
        for c in "add acc".chars() {
            h.key(if c == ' ' { Key::Space } else { Key::Char(c) });
        }
        h.advance(ms(200));
        h.key(Key::Enter);
        shot(&mut h, &out, "add-account-sheet", dark);
        // The Space's menu, through a right click on the sidebar's foot (one Space draws no
        // name).
        h.key(Key::Escape);
        h.advance(ms(400));
        if let Some(at) = h.centre(".ds-spaces-foot") {
            h.press(at, ds::base::press::PointerButton::Secondary);
            h.advance(ms(400));
        }
        shot(&mut h, &out, "space-menu", dark);

        // Tidy Up…, from the sidebar's menu: what it would archive, and its one button.
        let (mut h, _d) = open(dark);
        click(&mut h, "[*|aria-label=\"Sidebar Menu\"]");
        h.advance(ms(300));
        row_menu::press_menu_item(&mut h, "Tidy Up\u{2026}");
        h.advance(ms(400));
        shot(&mut h, &out, "tidy-up", dark);
    }
}

/// The Settings window, at the size it opens at, over the seeded store with two rules on its
/// account, as `mailo rules add` keeps them.
fn settings(dark: bool) -> (Harness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path());
    for (name, query, actions) in [
        (
            "Receipts",
            "from:shop.example",
            vec![
                RuleAction::Label("Receipts".to_owned()),
                RuleAction::MarkRead,
            ],
        ),
        (
            "Newsletters",
            "from:news@example.test",
            vec![RuleAction::Archive],
        ),
    ] {
        let draft = mail_core::rules::manage::Draft {
            name: name.to_owned(),
            query: query.to_owned(),
            actions,
            ..mail_core::rules::manage::Draft::blank()
        };
        mail_core::rules::manage::save(&store, acct_account(), &draft, &chrono::Utc).unwrap();
    }
    let contexts = mail_app::ui::native::contexts(
        store,
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    );
    let mut env = Environment::default();
    if dark {
        env.system.scheme = Scheme::Dark;
    }
    let config = HarnessConfig::new(Viewport {
        width: 780,
        height: 720,
        scale_percent: 100,
    })
    .with_clock(Clock::Wall)
    .with_net(NetPolicy::Local)
    .with_focus_fallback(FocusFallback::Ancestor)
    .with_contexts(contexts)
    .with_context(env);
    let mut harness = Harness::new(mail_app::ui::native::settings_root, config);
    harness.advance(ms(400));
    (harness, dir)
}

/// Settings' page `page`, by its sidebar row.
fn settings_page(h: &mut Harness, page: &str) {
    click(h, &format!(".ds-sidebar [*|aria-label=\"{page}\"]"));
    h.advance(ms(300));
}

#[test]
#[ignore = "picture generator: set MAILO_SHOTS to a directory and run with --ignored"]
fn settings_screens() {
    let out = std::env::var("MAILO_SHOTS").expect("MAILO_SHOTS names the directory");
    for dark in [false, true] {
        let (mut h, _d) = settings(dark);
        shot(&mut h, &out, "settings-general", dark);
        click(&mut h, "[*|id=\"general-pane-open-trusted\"]");
        shot(&mut h, &out, "settings-trusted-senders", dark);

        settings_page(&mut h, "Accounts");
        shot(&mut h, &out, "settings-accounts", dark);
        click(&mut h, "[*|aria-label=\"Details for me@example.test\"]");
        shot(&mut h, &out, "settings-account", dark);

        settings_page(&mut h, "Rules");
        shot(&mut h, &out, "settings-rules", dark);
        click(&mut h, "[*|id^=\"rules-pane-open-\"]");
        shot(&mut h, &out, "settings-rule", dark);
        settings_page(&mut h, "Rules");
        click(&mut h, "[*|id=\"rules-pane-new\"]");
        shot(&mut h, &out, "settings-rule-new", dark);

        settings_page(&mut h, "Vacation Reply");
        shot(&mut h, &out, "settings-vacation", dark);

        settings_page(&mut h, "Keyboard");
        shot(&mut h, &out, "settings-keyboard", dark);
    }
}

/// An older message opened in a conversation is as tall as what it holds, as Mail draws it: no
/// box of its own to scroll, and no empty room under a short one.
#[test]
fn an_opened_older_message_is_as_tall_as_its_content() {
    let (mut h, _d) = open(false);
    open_row(&mut h, 3);
    click(&mut h, ".reader-body .fold-line");
    let frame = ".reader-body article.frame:not(:last-of-type) .html";
    let height = |h: &Harness| h.rect(frame).map(|rect| rect.size.height.0);
    for _ in 0..40 {
        if height(&h).is_some_and(|px| px > 48.5) {
            break;
        }
        h.advance(ms(120));
    }
    let fitted = height(&h).unwrap_or_else(|| panic!("{frame} is not drawn:\n{}", h.html()));
    // Two short lines of plain text: taller than the frame's start, far shorter than the old box.
    assert!(
        fitted > 48.5 && fitted < 200.0,
        "the older message's frame is {fitted}px tall"
    );
}

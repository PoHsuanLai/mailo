//! The composer is set as the reader sets a message, on Blitz (`native`), in the real window: a
//! new message's subject sits where a message's title does, at its size and weight; its address
//! lines start at the reader's left edge, as a message's header does; and its body starts where
//! the reader's body does, at the reader's size. A reply under the thread lines up the same way,
//! and its bar of buttons is drawn inside the window.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`: nothing here
//! touches the network, the real mail store or the real config.

use ds::prelude::*;
use ds_harness::{Clock, DocQuery, Driver, Harness, HarnessConfig, Query, Viewport};
use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;
use std::time::Duration;

#[path = "support/drive.rs"]
mod drive;
use drive::{Drive, Key};

fn acct_account() -> AccountId {
    account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c7"))
}

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// How far apart two edges may be and still read as one.
const SLACK: f32 = 2.0;

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
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [acct_account().to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [
                IdentityId::generate().to_string(),
                acct_account().to_string(),
            ],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    let date = (now - chrono::Duration::hours(1)).to_rfc2822();
    let raw = format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Flight to the conference\r\n\
         Date: {date}\r\nMessage-ID: <align@example.test>\r\n\r\nThe gate changed to B12.\r\n"
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
                uidl: "align".to_owned(),
            },
            raw: raw.into_bytes(),
        }],
        false,
        now,
    )
    .unwrap();
    Arc::new(store)
}

fn open() -> (Harness, tempfile::TempDir) {
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
    (harness, dir)
}

/// The last message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

/// Open the one message, and wait until its frame shows its body.
fn open_message(harness: &mut Harness) {
    let subject = ".list .ds-list-item[*|aria-posinset=\"1\"] .ds-thread .ds-thread-sub";
    for _ in 0..50 {
        if harness.rect(subject).is_some() {
            break;
        }
        harness.advance(ms(100));
    }
    let rect = harness
        .rect(subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    for _ in 0..100 {
        harness.advance(ms(100));
        if harness
            .frame(FRAME)
            .is_some_and(|frame| frame.text().contains("B12"))
        {
            return;
        }
    }
    panic!("the message's frame never showed:\n{}", harness.html());
}

/// Wait until `selector` is drawn.
fn wait_for(harness: &mut Harness, selector: &str) {
    for _ in 0..100 {
        if harness.rect(selector).is_some() {
            harness.advance(ms(300));
            return;
        }
        harness.advance(ms(100));
    }
    panic!("{selector} is never drawn:\n{}", harness.html());
}

/// Where the text of the first element matching `selector` starts, and its font size: the left
/// of its content box (inside its border and padding) and its computed `font-size`, in px.
fn text_box(harness: &Harness, selector: &str) -> (f32, f32) {
    let rect = harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    let (inset, font) = harness
        .with_doc(|doc| {
            let node = doc.get_node(doc.query_selector(selector).ok()??)?;
            let layout = node.final_layout();
            let font = node
                .primary_styles()?
                .clone_font_size()
                .computed_size()
                .px();
            Some((layout.padding.left + layout.border.left, font))
        })
        .unwrap_or_else(|| panic!("{selector} has no style"));
    (rect.origin.x.0 + inset, font)
}

/// Where the reader's body text starts, and its size: the frame's `body` content box, on screen.
fn reader_body(harness: &Harness) -> (f32, f32) {
    harness
        .with_doc(|doc| {
            let iframe = doc.get_node(doc.query_selector(FRAME).ok()??)?;
            let offset = iframe.absolute_position(0.0, 0.0);
            let sub = iframe.subdoc()?.inner();
            let body_id = sub.query_selector("body").ok()??;
            let rect = sub.get_client_bounding_rect(body_id)?;
            let body = sub.get_node(body_id)?;
            let inset = body.final_layout().padding.left + body.final_layout().border.left;
            let font = body
                .primary_styles()?
                .clone_font_size()
                .computed_size()
                .px();
            Some((offset.x + rect.x as f32 + inset, font))
        })
        .unwrap_or_else(|| panic!("the reader's frame has no body:\n{}", harness.html()))
}

/// The reader's model: the left edge of its title and of its message header, the title's size,
/// and where its body's text starts and at what size.
struct Reader {
    title_left: f32,
    title_font: f32,
    head_left: f32,
    body_left: f32,
    body_font: f32,
}

fn reader(harness: &Harness) -> Reader {
    let (title_left, title_font) = text_box(harness, ".reader-head h2 .ds-label");
    let head_left = harness
        .rect(".reader-body article.frame:last-of-type header.msg-head")
        .expect("the message's header is drawn")
        .origin
        .x
        .0;
    let (body_left, body_font) = reader_body(harness);
    Reader {
        title_left,
        title_font,
        head_left,
        body_left,
        body_font,
    }
}

fn near(what: &str, got: f32, want: f32) {
    assert!(
        (got - want).abs() <= SLACK,
        "{what} is at {got}, the reader's at {want}"
    );
}

/// A property row's label, and its value: both start at the header's left edge, in that order.
fn assert_lines(harness: &Harness, page: &str, rows: &[&str], model: &Reader) {
    for name in rows {
        let row = format!("{page} .c-props [*|data-row={name}]");
        let label = harness
            .rect(&format!("{row} .c-line-k"))
            .unwrap_or_else(|| panic!("{row} has no label:\n{}", harness.html()));
        near(
            &format!("{name}'s label"),
            label.origin.x.0,
            model.head_left,
        );
        let line = harness.rect(&row).expect("the row is drawn");
        near(&format!("{name}'s line"), line.origin.x.0, model.head_left);
    }
}

#[test]
fn a_new_message_is_set_as_the_reader_sets_a_message() {
    let (mut harness, _dir) = open();
    open_message(&mut harness);
    let model = reader(&harness);
    near("the reader's body", model.body_left, model.head_left);

    harness.key(Key::Char('c'));
    wait_for(&mut harness, ".cpage .c-body");

    let (subject_left, subject_font) = text_box(&harness, ".cpage .c-title input");
    near("the subject", subject_left, model.title_left);
    assert_eq!(subject_font, model.title_font, "the subject's size");
    assert_lines(
        &harness,
        ".cpage",
        &["from", "to", "sends", "protection", "remind"],
        &model,
    );
    let (body_left, body_font) = text_box(&harness, ".cpage .c-body");
    near("the body", body_left, model.body_left);
    assert_eq!(body_font, model.body_font, "the body's size");
}

#[test]
fn a_reply_is_set_as_the_reader_sets_a_message() {
    let (mut harness, _dir) = open();
    open_message(&mut harness);
    let model = reader(&harness);

    harness.key(Key::Char('r'));
    wait_for(&mut harness, ".inline-reply .c-body");

    assert_lines(&harness, ".inline-reply", &["to", "remind"], &model);
    let (body_left, body_font) = text_box(&harness, ".inline-reply .c-body");
    near("the reply's body", body_left, model.body_left);
    assert_eq!(body_font, model.body_font, "the reply's body size");

    // Its bar, Send and all, is inside the window.
    let bar = harness
        .rect(".inline-reply .c-foot")
        .expect("the reply's bar is drawn");
    let bottom = bar.origin.y.0 + bar.size.height.0;
    assert!(
        bar.origin.y.0 >= 0.0 && bottom <= VIEW.height as f32,
        "the reply's bar runs out of the window: {bar:?}"
    );
}

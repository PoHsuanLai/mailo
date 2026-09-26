//! The reader's Source view on Blitz, in the real window: a message's stored bytes, every header
//! included, drawn as plain text beside Reader and Original, and Forward as attachment offered
//! from it.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`: nothing here
//! touches the network, the real mail store or the real config.

use ds_native::harness::settle_until;
use ds_native::{Harness, HarnessConfig, Viewport};
use mail_domain::*;
use mail_runtime::assemble::absorb_rebuilt_into;
use mail_runtime::{Arrival, Destination, absorb};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b3"));

const VIEW: Viewport = Viewport {
    width: 1280,
    height: 900,
    scale_percent: 100,
};

/// A plain message whose body is markup, a bare CR and a right-to-left override: none of it may
/// be drawn as anything but the characters it is.
fn plain(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Lunch\r\nDate: {date}\r\n\
         Message-ID: <lunch@example.test>\r\nX-Trace: hop-1\r\n\r\n\
         Thursday? <b>not bold</b>\r\nbare\rcr and a\u{202E}reversal\r\n"
    )
}

fn newsletter(date: &str) -> String {
    format!(
        "From: news@shop.example.test\r\nTo: me@example.test\r\nSubject: The letter\r\n\
         Date: {date}\r\nMessage-ID: <letter@shop.example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n\
         <table><tr><td><h1>Autumn</h1><p>Wool, finally.</p></td></tr></table>\r\n"
    )
}

/// A large IMAP message as the store holds it: rebuilt from its sections, its PDF left on the
/// server and marked so (`mail_mime::reconstruct`).
fn rebuilt(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: The report\r\nDate: {date}\r\n\
         Message-ID: <report@example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n\
         --mix\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe numbers are attached.\r\n\
         --mix\r\nContent-Type: application/pdf; name=\"report.pdf\"\r\n\
         Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\nX-Mailo-Remote-Section: 2\r\n\
         X-Mailo-Remote-Octets: 2000000\r\n\r\n\r\n--mix--\r\n"
    )
}

/// A store in `dir` with one account and three threads, newest first: the plain note, the
/// newsletter, and the rebuilt report.
fn seeded(dir: &std::path::Path) -> (Arc<SqliteStore>, String) {
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    let store = SqliteStore::open(dir.join("mail.db"), dir.join("blobs")).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, NULL, 'me@example.test', '\"default\"')",
            [IdentityId::generate().to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    let now = chrono::Utc::now();
    let date = |hours: i64| (now - chrono::Duration::hours(hours)).to_rfc2822();
    let inbox = MailboxRef {
        account: ACCOUNT,
        path: "INBOX".to_owned(),
    };
    let arrival = |uidl: &str, raw: String| Arrival {
        remote: RemoteRef::Pop {
            uidl: uidl.to_owned(),
        },
        raw: raw.into_bytes(),
    };
    let note = plain(&date(1));
    absorb(
        &store,
        ACCOUNT,
        inbox.clone(),
        Some(SyncCursor::Pop),
        vec![
            arrival("plain", note.clone()),
            arrival("letter", newsletter(&date(2))),
        ],
        false,
        now,
    )
    .unwrap();
    absorb_rebuilt_into(
        &store,
        ACCOUNT,
        Destination {
            mailbox: inbox,
            role: MailboxRole::Inbox,
        },
        vec![arrival("report", rebuilt(&date(3)))],
        now,
    )
    .unwrap();
    (Arc::new(store), note)
}

struct Window {
    harness: Harness,
    store: Arc<SqliteStore>,
    /// The plain note's bytes, as they were stored.
    note: String,
    _dir: tempfile::TempDir,
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn open() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let (store, note) = seeded(dir.path());
    let contexts = mail_app::ui::native::contexts(
        store.clone(),
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    );
    let config = HarnessConfig::new(VIEW).with_contexts(contexts);
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    Window {
        harness,
        store,
        note,
        _dir: dir,
    }
}

fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!(".ds-list > .row:nth-child({n}) .ds-row .ds-row-sub");
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(ds::Point {
        x: ds::Px(rect.origin.x.0 + 24.0),
        y: ds::Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

fn click(harness: &mut Harness, selector: &str) {
    let at = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.click(at);
    harness.advance(ms(300));
}

/// The switch's button that says `label`.
fn switch_button(harness: &Harness, label: &str) -> String {
    (1..=3)
        .map(|n| format!("article.frame header .view-switch .ds-button:nth-child({n})"))
        .find(|selector| harness.attr(selector, "aria-label").as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no {label} in the view switch:\n{}", harness.html()))
}

const TEXT: &str = ".source .source-text";
const BLOCKS: &str = "article.frame .blocks";
const FORWARD: &str = ".source-tools .ds-button";

fn show_source(harness: &mut Harness) {
    let source = switch_button(harness, "Source");
    assert_eq!(harness.count(".source"), 0, "the source was drawn unasked");
    click(harness, &source);
    settle_until(harness, |h| h.count(TEXT) == 1);
}

/// The bytes, as they were stored, are what the view says: every header, the body's markup as
/// characters, and a bare CR and a bidi override named rather than acted on. Reader takes it away
/// again.
#[test]
fn the_source_is_the_stored_bytes_as_plain_text() {
    let mut window = open();
    let harness = &mut window.harness;
    open_row(harness, 1);
    assert!(!harness.has_class(BLOCKS, "is-hidden"));

    show_source(harness);
    let shown = harness.text_of(TEXT).expect("the source text");
    let expect = window
        .note
        .replace("\r\n", "\n")
        .replace('\r', "\u{240D}")
        .replace('\u{202E}', "<U+202E>");
    assert_eq!(shown, expect, "the source is not the stored bytes");
    assert!(shown.contains("X-Trace: hop-1"), "a header is missing");
    assert_eq!(
        harness.count(".source-text b"),
        0,
        "the body's markup was parsed"
    );
    assert!(
        harness.has_class(BLOCKS, "is-hidden"),
        "the Reader view is still drawn under the source"
    );
    assert_eq!(
        harness.count(".source .source-note"),
        0,
        "a note on a message that needs none"
    );

    let reader = switch_button(harness, "Reader");
    click(harness, &reader);
    assert_eq!(harness.count(".source"), 0, "Reader left the source up");
    assert!(!harness.has_class(BLOCKS, "is-hidden"));
}

/// On an HTML message the switch has all three, and Source hides the frame as well as the blocks.
#[test]
fn source_is_the_third_way_beside_reader_and_original() {
    let mut window = open();
    let harness = &mut window.harness;
    open_row(harness, 2);
    assert_eq!(
        harness.count("article.frame header .view-switch .ds-button"),
        3
    );
    show_source(harness);
    assert!(harness.has_class("article.frame iframe.html", "is-hidden"));
    assert!(harness.has_class(BLOCKS, "is-hidden"));
    let shown = harness.text_of(TEXT).unwrap();
    assert!(
        shown.contains("<table><tr><td><h1>Autumn</h1>"),
        "the markup is not shown as text:\n{shown}"
    );
    assert_eq!(harness.count(".source-text table"), 0);
}

/// A message rebuilt from its parts says so, and is not forwarded as an attachment as if it were
/// the message as sent.
#[test]
fn a_rebuilt_message_says_so_and_is_not_forwarded_as_the_original() {
    let mut window = open();
    let drafts = window.store.drafts(ACCOUNT).unwrap().len();
    let harness = &mut window.harness;
    open_row(harness, 3);
    show_source(harness);
    let note = harness.text_of(".source .source-note").expect("a note");
    assert!(note.contains("downloaded in parts"), "{note}");
    assert!(
        harness
            .text_of(TEXT)
            .unwrap()
            .contains("X-Mailo-Remote-Section: 2"),
        "the stand-in part is not shown as it is stored"
    );

    click(harness, FORWARD);
    settle_until(harness, |h| h.count(".reader-body > .notice") == 1);
    let said = harness.text_of(".reader-body > .notice").unwrap();
    assert!(said.contains("Not forwarded as an attachment"), "{said}");
    assert!(said.contains("rebuilt"), "{said}");
    assert_eq!(
        window.store.drafts(ACCOUNT).unwrap().len(),
        drafts,
        "a refused forward left a draft"
    );
}

/// Forward as attachment, from the source: a composer on a draft carrying the message itself.
#[test]
fn forward_as_attachment_opens_a_composer_carrying_the_message() {
    let mut window = open();
    let drafts = window.store.drafts(ACCOUNT).unwrap().len();
    let harness = &mut window.harness;
    open_row(harness, 1);
    show_source(harness);
    click(harness, FORWARD);
    let store = window.store.clone();
    settle_until(harness, |_| {
        store.drafts(ACCOUNT).unwrap().len() == drafts + 1
    });
    let draft = window
        .store
        .drafts(ACCOUNT)
        .unwrap()
        .into_iter()
        .find(|draft| draft.subject == "Fwd: Lunch")
        .expect("the forward");
    assert_eq!(draft.attachments.len(), 1);
    assert_eq!(draft.attachments[0].mime, "message/rfc822");
    assert_eq!(draft.attachments[0].name, "Lunch.eml");
    let carried = window
        .store
        .blobs()
        .get(&window.store.connection(), draft.attachments[0].blob)
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&carried), window.note);
    settle_until(&mut window.harness, |h| h.count(".c-body") >= 1);
}

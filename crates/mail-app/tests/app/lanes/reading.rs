//! Lane 0: opening and reading. The window opens on the inbox with the keyboard, counts its
//! unread mail for the launcher and counts again as it changes; a conversation clicked opens in
//! the reader; a sender whose logo is cached shows it, and the reader warns, without a logo, when
//! the receiving server's checks say a sender may not be who they say; a wheel over an earlier
//! message moves the whole conversation; a newsletter wider than the reader is drawn smaller to
//! fit it; a verification email's one-time code is offered under its header, and copied.

use ds::prelude::{Point, Px};
use ds_harness::{Harness, Input};
use mail_os::launcher::{Badge, Launcher, Unread};
use mail_ui_model::copy;
use std::sync::{Arc, Mutex};

use super::drive::{Drive, Key, PRIMARY};
use super::look::sizes_hold;
use super::row_menu::row_action;
use super::window::{Window, deliver, ms, row_of};

/// A logo in SVG Tiny PS: a blue square with a white disc.
const LOGO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" version="1.2" baseProfile="tiny-ps" viewBox="0 0 64 64"><title>Brand</title><rect width="64" height="64" fill="#1a73e8"/><circle cx="32" cy="32" r="16" fill="#ffffff"/></svg>"##;

const SHIPPED: &str = "Your order has shipped";
const LOCKED: &str = "Your account is locked";
const THREAD: &str = "A long thread";
const WIDE: &str = "A wide newsletter";

/// Every count the window handed its launcher, in order.
#[derive(Default)]
struct Counts(Mutex<Vec<Unread>>);

impl Badge for Counts {
    fn show(&self, unread: Unread) {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(unread);
    }
}

impl Counts {
    fn seen(&self) -> Vec<u64> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|unread| unread.0)
            .collect()
    }
}

/// Eighty numbered lines: far taller than the window.
fn lines() -> String {
    (1..=80)
        .map(|n| format!("Line {n} of a long letter."))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The mail this lane reads, beside the seeded inbox, each newer than it.
fn letters(store: &mail_core::SqliteStore) {
    let checked = |verdict: &str| {
        format!(
            "Authentication-Results: mx.provider.example; spf={verdict} smtp.mailfrom=brand.example;\r\n \
             dkim={verdict} header.d=brand.example; dmarc={verdict} header.from=brand.example\r\n"
        )
    };
    // The receiving server's fail, and under it a pass the sender forged.
    let forged = format!(
        "{}Received: from attacker.example by mx.provider.example; Fri, 25 Sep 2026 10:00:00 +0000\r\n\
         Authentication-Results: mx.provider.example; spf=pass; dkim=pass; dmarc=pass\r\n",
        checked("fail")
    );
    let html = "MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n";
    let letters = [
        format!(
            "{}From: ada@brand.example\r\nSubject: {SHIPPED}\r\nMessage-ID: <shipped@brand.example>\r\n\r\nOn its way.\r\n",
            checked("pass")
        ),
        format!(
            "{forged}From: grace@brand.example\r\nSubject: {LOCKED}\r\nMessage-ID: <locked@brand.example>\r\n\r\nSign in here.\r\n"
        ),
        format!(
            "From: grace@example.test\r\nSubject: {THREAD}\r\nMessage-ID: <t1@example.test>\r\n\r\n{}\r\n",
            lines()
        ),
        format!(
            "From: grace@example.test\r\nSubject: Re: {THREAD}\r\nMessage-ID: <t2@example.test>\r\n\
             In-Reply-To: <t1@example.test>\r\nReferences: <t1@example.test>\r\n\r\n{}\r\n",
            lines()
        ),
        // Fixed 800 px tables, as a bulk sender lays one out, that no max-width narrows.
        format!(
            "From: news@example.test\r\nSubject: {WIDE}\r\nMessage-ID: <wide@example.test>\r\n{html}\r\n\
             <html><body><div align=\"center\"><table border=\"0\" width=\"800\"><tr><td>\
             <table id=\"inner\" width=\"800px\" style=\"table-layout: fixed\"><tr><td>Wide words</td>\
             <td>at the right edge</td></tr></table></td></tr></table></div></body></html>\r\n"
        ),
    ];
    for (n, letter) in letters.iter().enumerate() {
        let raw = format!(
            "To: me@example.test\r\nDate: {}\r\n{letter}",
            (chrono::Utc::now() - chrono::Duration::minutes(50 - 5 * n as i64)).to_rfc2822()
        );
        deliver(store, &format!("read{n}"), &raw);
    }
}

/// The window over the seeded inbox and [`letters`], with `brand.example`'s logo drawn and
/// cached, logos switched on, and `counts` as its launcher.
fn open(counts: &Arc<Counts>) -> Window {
    let counts = Arc::clone(counts);
    Window::open_with(chrono::Utc::now(), letters, move |dir, contexts| {
        let settings = mail_ui_model::settings::root_for(&dir.join("config"));
        mail_ui_model::settings::change(&settings, |settings| {
            settings.reading.brand_logos = mail_core::bimi::Setting::On.into();
        })
        .expect("the setting is written");
        let cache = dir.join("cache");
        let png = mail_runtime::bimi::draw(LOGO.as_bytes()).expect("the logo draws");
        mail_runtime::bimi::remember(&cache, "brand.example", Some(&png), chrono::Utc::now())
            .expect("the logo is cached");
        contexts
            .with(mail_ui::ui::native::BrandCache(cache))
            .with(Launcher(counts as Arc<dyn Badge>))
    })
}

/// The newest message's head, once it says the sender's checks.
const STANDING: &str = ".reader-meta[*|data-standing]";
/// The logo in the reader's head.
const HEAD_LOGO: &str = ".reader-meta .reader-logo img";
/// The last message's frame.
const FRAME: &str = ".reader-body article.frame:last-of-type iframe.html";

/// Whether the head says the open message's checks were `standing`.
fn standing(harness: &Harness, standing: &str) -> bool {
    harness.attr(STANDING, "data-standing").as_deref() == Some(standing)
}

/// How `read` measures the open message, once it has stopped changing.
fn settled(window: &mut Window, read: impl Fn(&Harness) -> Option<f32>) -> f32 {
    let (mut was, mut same) = (read(&window.harness), 0);
    for _ in 0..100 {
        window.harness.advance(ms(150));
        let now = read(&window.harness);
        same = if now == was { same + 1 } else { 0 };
        if same == 4 {
            return now.unwrap_or_else(|| panic!("never drawn:\n{}", window.harness.html()));
        }
        was = now;
    }
    panic!("never settled:\n{}", window.harness.html());
}

#[test]
fn the_window_opens_on_the_inbox_and_reads_what_a_click_opens() {
    let counts = Arc::new(Counts::default());
    let mut window = open(&counts);

    // Nothing open yet, and the keyboard is the window's from the first frame.
    assert_eq!(window.harness.count(".reader .ds-empty-state"), 1);
    sizes_hold(&window.harness, "the list");
    assert!(window.harness.is_focused(".app"), "no keyboard");

    // The launcher counts the unread conversations, again when one is read and when that is
    // taken back, and not for a write that leaves the count where it is.
    let unread = window.harness.count(".list .ds-thread") as u64;
    window.until("the launcher has the count", |_| counts.seen() == [unread]);
    let first = ".list .ds-list-item[*|aria-posinset=\"1\"]";
    row_action(&mut window.harness, first, copy::menus::MARK_AS_READ);
    window.until("the count follows", |_| {
        counts.seen() == [unread, unread - 1]
    });
    window.press(&[PRIMARY], Key::Char('z'), 1);
    window.until("and follows the undo", |_| {
        counts.seen() == [unread, unread - 1, unread]
    });
    row_action(&mut window.harness, first, copy::menus::PIN);
    window.harness.advance(ms(600));
    assert_eq!(counts.seen(), [unread, unread - 1, unread], "a pin counted");

    // The list's ⋯ opens on a click and closes on Escape, handing the keyboard back to it.
    let more = format!(".list-head button[*|aria-label=\"{}\"]", copy::aria::MORE);
    window.click(&more);
    window.until("the ⋯ opens", |h| h.count(".ds-menu") == 1);
    assert_eq!(
        window.harness.attr(&more, "aria-expanded").as_deref(),
        Some("true")
    );
    window.press(&[], Key::Escape, 1);
    window.until("Escape closes it", |h| h.count(".ds-menu") == 0);
    assert!(
        window.harness.is_focused(&more),
        "the keyboard went elsewhere"
    );

    // A pass: the sender's cached logo takes the initial's place, and nothing warns.
    window.open_subject(SHIPPED);
    sizes_hold(&window.harness, "the reader");
    let row = row_of(&window.harness, SHIPPED).unwrap_or_default();
    let selected = format!(".list .ds-list-item[*|aria-posinset=\"{row}\"] .ds-row");
    assert_eq!(
        window.harness.attr(&selected, "aria-selected").as_deref(),
        Some("true")
    );
    window.until("the checks passed", |h| standing(h, "pass"));
    window.until("the logo is drawn", |h| h.count(HEAD_LOGO) == 1);
    let src = window.harness.attr(HEAD_LOGO, "src").unwrap_or_default();
    assert!(src.starts_with("data:image/png;base64,"), "{src}");
    assert_eq!(window.text(".reader-meta .reader-av").trim(), "");
    assert_eq!(
        window.harness.count(".trust-warn"),
        0,
        "a pass warned about"
    );

    // A pass forged under the server's fail: one line naming the sender, and their initial.
    window.open_subject(LOCKED);
    window.until("the checks failed", |h| standing(h, "fail"));
    window.harness.advance(ms(300));
    assert_eq!(
        window.text(".trust-warn").trim(),
        copy::errors::may_not_be_from("grace@brand.example")
    );
    assert_eq!(window.harness.count(".reader-logo"), 0, "a logo for a fail");
    assert_eq!(window.text(".reader-meta .reader-av").trim(), "G");

    // A wheel burst over the earlier of two long messages moves the conversation, not the
    // message's own frame.
    window.open_subject(THREAD);
    let last = |h: &Harness| {
        h.rect(".reader-body article.frame:last-of-type")
            .map(|r| r.origin.y.0)
    };
    settled(&mut window, last);
    let early = ".reader-body article.frame:first-of-type iframe.html";
    let frame = window
        .harness
        .rect(early)
        .expect("the first message's frame");
    let at = Point {
        x: Px(frame.origin.x.0 + frame.size.width.0 / 2.0),
        y: Px((frame.origin.y.0 + 20.0).max(130.0)),
    };
    let text = |h: &Harness| h.frame(early).and_then(|f| f.centre("body")).map(|p| p.y.0);
    let (text_was, last_was) = (
        text(&window.harness).expect("its text"),
        last(&window.harness).expect("the last message"),
    );
    for _ in 0..6 {
        window.harness.send(Input::Detents {
            at,
            x: 0.0,
            y: -3.0,
            held: dioxus::prelude::Modifiers::empty(),
        });
        window.harness.advance(ms(16));
    }
    window.harness.advance(ms(800));
    let (text_now, last_now) = (
        text(&window.harness).expect("its text"),
        last(&window.harness).expect("the last message"),
    );
    assert!(last_was - last_now > 500.0, "{last_was} -> {last_now}");
    assert!(
        ((text_was - text_now) - (last_was - last_now)).abs() < 1.0,
        "the frame kept its own place: text {text_was} -> {text_now}"
    );

    // A newsletter of fixed 800 px tables, in a narrower pane, is drawn smaller to fit.
    window.open_subject(WIDE);
    window.until("the newsletter is drawn", |h| {
        h.frame(FRAME).is_some_and(|f| f.count("#inner") == 1)
    });
    let pane = window.harness.rect(FRAME).expect("the frame").size.width.0;
    assert!(
        pane < 800.0,
        "the pane is wider than the newsletter: {pane}"
    );
    let table = settled(&mut window, |h| {
        h.frame(FRAME)
            .and_then(|f| f.width("#inner"))
            .map(|px| px.0)
    });
    assert!(
        table <= pane && table > pane * 0.8,
        "the 800 px table is {table} px wide in a {pane} px frame"
    );
}

/// A sign-in email in HTML alone, its code in bold, and an ordinary letter beside it: the code is
/// offered under the email's header and Copy Code copies it as a sensitive text, kept out of
/// clipboard history (a recorder's, never the desktop's), not through the plain clipboard; the
/// letter offers nothing.
#[test]
fn a_one_time_code_is_offered_under_the_header_and_copied() {
    const SIGN_IN: &str = "Sign in to Example";
    const LUNCH: &str = "Lunch on Friday";
    let copied = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink = copied.clone();
    let mut window = Window::open_with(
        chrono::Utc::now(),
        |store| {
            let at = |minutes: i64| {
                (chrono::Utc::now() - chrono::Duration::minutes(minutes)).to_rfc2822()
            };
            deliver(
                store,
                "otp1",
                &format!(
                    "To: me@example.test\r\nDate: {}\r\nFrom: no-reply@example.test\r\n\
                 Subject: {SIGN_IN}\r\nMessage-ID: <otp1@example.test>\r\nMIME-Version: 1.0\r\n\
                 Content-Type: text/html; charset=utf-8\r\n\r\n<html><body><p>Your verification \
                 code is <b>482913</b>.</p><p>It expires in 10 minutes.</p></body></html>\r\n",
                    at(3)
                ),
            );
            deliver(
                store,
                "otp2",
                &format!(
                    "To: me@example.test\r\nDate: {}\r\nFrom: ada@example.test\r\n\
                 Subject: {LUNCH}\r\nMessage-ID: <otp2@example.test>\r\n\r\n\
                 Order 4521 is in; see you at 12:30 in room 2048.\r\n",
                    at(4)
                ),
            );
        },
        move |_, contexts| {
            contexts.with(mail_ui::ui::native::SensitiveCopy::with_sink(move |text| {
                sink.lock().unwrap().push(text.to_owned());
            }))
        },
    );
    const LINE: &str = ".reader-body .otp-code";

    window.open_subject(SIGN_IN);
    window.until("the code is offered", |h| h.count(LINE) == 1);
    assert_eq!(
        window.harness.attr(LINE, "data-code").as_deref(),
        Some("482913")
    );
    assert!(
        window.text(LINE).contains("Code 482913"),
        "{}",
        window.text(LINE)
    );
    let button = format!(
        "{LINE} button[*|aria-label=\"{}\"]",
        copy::controls::copy_code("482913")
    );
    window.click(&button);
    window.harness.advance(ms(100));
    assert_eq!(*copied.lock().unwrap(), ["482913"]);
    assert_eq!(
        window.harness.clipboard_text(),
        None,
        "the plain clipboard, which history keeps, was not used"
    );

    window.open_subject(LUNCH);
    window.harness.advance(ms(600));
    assert_eq!(
        window.harness.count(LINE),
        0,
        "a letter with numbers offered a code"
    );
}

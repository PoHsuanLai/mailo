//! The reader's Original view on Blitz (`native`), in the real window: the sender's sanitized
//! markup is a sealed document of its own, its remote images are fetched only for the thread
//! the reader consented to, a link in it opens in the browser, and a click on its inline image
//! opens that image in Peek. The window's own document asks for nothing remote, and no script
//! engine is built in.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`, with an
//! `Original` whose fetcher and browser are recorders: nothing here touches the network, the
//! real mail store or the real config. The component-level guarantees, with bodies fed around
//! the sanitizer, are `tests/app/native_frame.rs`.

use ds::prelude::*;
use ds_blitz::{AppNet, NetDecision, NetPolicy, NetReply, NetRequest, RequestOrigin, RootContexts};
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};
use mail_app::ui::native::{Browse, Fetch, Original};
use mail_core::SqliteStore;
use mail_core::{Arrival, absorb};
use mail_domain::*;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::drive;
use crate::native::click;
use crate::native::ms;
use crate::native::thread;
use drive::{Drive, Key};

use crate::store::acct_account;

const VIEW: Viewport = Viewport {
    width: 1280,
    height: 900,
    scale_percent: 100,
};

const HERO_URL: &str = "https://images.example.test/autumn/hero.png";
const BADGE_URL: &str = "https://images.example.test/autumn/badge.png";
const OTHER: &str = "https://cdn.example.test/other.png";
const OFFER: &str = "https://shop.example.test/autumn?ref=mail";

/// A laid-out newsletter: tables, two remote images, an inline one (`cid:`) and a link. The
/// images carry no size or alt, so one that did not load lays out 0 wide. The sanitizer strips
/// `class`, so the frame's elements are found by where they sit: [`LOGO`], [`HERO`],
/// [`BADGE`], [`LINK`], and [`LIAR`], a link whose text names somewhere it does not go. [`LINK`]
/// carries tracking parameters beside its own `ref`; a click opens [`OFFER`], without them.
const NEWSLETTER: &str = r##"<table width="600" cellpadding="0" cellspacing="0" bgcolor="#f4efe6"><tr><td>
<table width="100%"><tr>
<td><img src="cid:logo@shop"></td>
<td align="right"><font color="#7a5c3a">Autumn letter &middot; No. 14</font>
<a href="https://shop.example.test/autumn?ref=mail&amp;utm_source=letter&amp;utm_medium=email&amp;fbclid=IwAR0">See the collection</a>
<a href="https://g00gle-security.xyz/verify">google.com</a></td>
</tr></table>
<h1>The autumn collection is here</h1>
<img src="https://images.example.test/autumn/hero.png">
<table width="100%"><tr>
<td width="50%"><h3>Wool, finally</h3><p>Twelve new knits, spun a valley over.</p></td>
<td width="50%"><h3>Lamps</h3><p>Warm light for the long evenings, in brass and paper.</p></td>
</tr></table>
<p><img src="https://images.example.test/autumn/badge.png"></p>
<p><font size="1">You are receiving this because you asked to.</font></p>
</td></tr></table>"##;

/// A 7 × 5 PNG: an image that loads lays out 7 px wide, one that does not, 0.
fn swatch() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(7, 5, image::Rgba([196, 120, 40, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode the swatch");
    bytes
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The newsletter as a `multipart/related` message with its inline logo.
fn newsletter(date: &str) -> String {
    format!(
        "From: Shop <letters@shop.example.test>\r\nTo: me@example.test\r\n\
         Subject: The autumn letter\r\nDate: {date}\r\nMessage-ID: <autumn@shop.example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"b1\"\r\n\r\n\
         --b1\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{NEWSLETTER}\r\n\
         --b1\r\nContent-Type: image/png\r\nContent-ID: <logo@shop>\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{}\r\n--b1--\r\n",
        base64(&swatch())
    )
}

fn other(date: &str) -> String {
    format!(
        "From: news@cdn.example.test\r\nTo: me@example.test\r\nSubject: Another letter\r\n\
         Date: {date}\r\nMessage-ID: <other@cdn.example.test>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/html; charset=utf-8\r\n\r\n\
         <p>Hello.</p><img src=\"{OTHER}\">\r\n"
    )
}

fn plain(date: &str) -> String {
    format!(
        "From: ada@example.test\r\nTo: me@example.test\r\nSubject: Lunch\r\nDate: {date}\r\n\
         Message-ID: <lunch@example.test>\r\n\r\nThursday?\r\n"
    )
}

/// A store in `dir` with one account and three threads, newest first: the newsletter, another
/// HTML letter with a remote image, and a plain-text note.
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
    }
    let now = chrono::Utc::now();
    let bodies: [fn(&str) -> String; 3] = [newsletter, other, plain];
    for (n, body) in bodies.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
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
                    uidl: format!("orig{n}"),
                },
                raw: body(&date).into_bytes(),
            }],
            false,
            now,
        )
        .unwrap();
    }
    Arc::new(store)
}

/// Every URL the frames' network fetched, answered with [`swatch`] at once.
#[derive(Default)]
struct Fetched(Mutex<Vec<String>>);

impl Fetch for Fetched {
    fn get(&self, url: String, done: Box<dyn FnOnce(Vec<u8>) + Send>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url);
        done(swatch());
    }
}

impl Fetched {
    fn count(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).len()
    }

    fn urls(&self) -> Vec<String> {
        let mut urls = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        urls.sort();
        urls.dedup();
        urls
    }
}

/// Every link opened in the browser.
#[derive(Default)]
struct Opened(Mutex<Vec<String>>);

impl Browse for Opened {
    fn open(&self, url: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.to_owned());
    }
}

struct Window {
    harness: Harness,
    original: Original,
    fetched: Arc<Fetched>,
    opened: Arc<Opened>,
    _dir: tempfile::TempDir,
}

fn contexts(dir: &std::path::Path) -> RootContexts {
    mail_app::ui::native::contexts(
        mail_client::Client::open(seeded(dir), mail_client::ClientConfig::default()),
        mail_app::ui::view::Appearance::default(),
        None,
        None,
        mail_app::ui::Start::Inbox,
    )
}

/// The window over the seeded store, its frames held by an `Original` with recorders.
fn open() -> Window {
    let dir = tempfile::tempdir().unwrap();
    let fetched = Arc::new(Fetched::default());
    let opened = Arc::new(Opened::default());
    let original = Original::new(fetched.clone(), opened.clone());
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts(dir.path()))
        .with_net(original.net())
        .with_frame_links(original.links())
        .with_contexts(original.contexts());
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    Window {
        harness,
        original,
        fetched,
        opened,
        _dir: dir,
    }
}

fn open_row(harness: &mut Harness, n: usize) {
    let subject = format!("{} .ds-thread-sub", thread(n));
    let rect = harness
        .rect(&subject)
        .unwrap_or_else(|| panic!("{subject} is not drawn:\n{}", harness.html()));
    harness.click(Point {
        x: Px(rect.origin.x.0 + 24.0),
        y: Px(rect.origin.y.0 + rect.size.height.0 / 2.0),
    });
    harness.advance(ms(300));
}

/// The inline logo, in the header's own table.
const LOGO: &str = "table table img";
/// The hero image, under the heading.
const HERO: &str = "h1 + img";
/// The badge, alone in its paragraph.
const BADGE: &str = "p > img";
/// The honest link, in the header, where a 320 px frame shows it without scrolling.
const LINK: &str = "a";
/// The link beside it, whose text names one domain and whose target is another.
const LIAR: &str = "a + a";

const SHOW_IMAGES: &str = ".images-hidden .ds-button";
const FRAME: &str = "article.frame iframe.html";

fn frame_width(harness: &Harness, selector: &str) -> f32 {
    let frame = harness
        .frame(FRAME)
        .unwrap_or_else(|| panic!("no frame document:\n{}", harness.html()));
    frame
        .width(selector)
        .unwrap_or_else(|| panic!("{selector} is not in the frame:\n{}", frame.html()))
        .0
}

const PEEK: &str = ".ds-peek";

/// Advance until `done`, or fail saying `what` never came.
fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    let start = std::time::Instant::now();
    while !done(harness) {
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "{what} never came:\n{}",
            harness.html()
        );
        harness.advance(ms(50));
    }
}

/// Rest the pointer on each of the rows `rows`, long enough for a card to open.
fn hover_rows(harness: &mut Harness, rows: std::ops::RangeInclusive<usize>) {
    for n in rows {
        let at = harness
            .centre(&format!("{} .ds-thread-from", thread(n)))
            .expect("a row's sender");
        harness.pointer_move(at);
        harness.advance(ds::style::tokens::delay::DelayToken::CardOpen.delay() + ms(250));
    }
}

/// What the browser was asked to open.
fn opened(window: &Window) -> Vec<String> {
    window
        .opened
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// The newsletter's frame, from rest to consent and back.
///
/// At rest, `cid:` inline images resolve inside the frame as `data:` in its own markup: shown
/// before any consent, with no request made. (a) No shared DOM: the sender's elements are found
/// in the frame and never by the window's queries, and the frame holds none of the window's.
///
/// The link pill shows where a link goes as the pointer comes onto it, read by the same honesty
/// check as a Reader view link, and goes as the pointer leaves. (e) A click opens the link in the
/// browser through mailo, without its tracking parameters, and the frame stays as it was. A
/// click on the inline image opens it in Peek; Esc closes it.
///
/// (d) Consent, 0 → N → 0: nothing is fetched until Load Images, hovering rows included, then
/// exactly the newsletter's two images; a click on a remote image opens nothing, as it is not
/// the message's own. Opening another thread revokes the consent, and the newsletter opened
/// again asks again; closing the reader takes the consent back.
#[test]
fn the_newsletter_s_frame_is_sealed_its_links_are_checked_and_its_images_wait_for_consent() {
    let mut window = open();
    hover_rows(&mut window.harness, 1..=3);
    assert_eq!(window.fetched.count(), 0, "hovering fetched");
    open_row(&mut window.harness, 1);
    assert!(
        window.harness.count(FRAME) >= 1,
        "the Original view did not mount"
    );

    assert_eq!(frame_width(&window.harness, LOGO), 7.0, "the inline logo");
    let frame = window.harness.frame(FRAME).expect("a frame document");
    let src = frame.attr(LOGO, "src").unwrap_or_default();
    assert!(
        src.starts_with("data:image/png;base64,"),
        "the inline logo: {src}"
    );
    assert_eq!(frame.count("h3"), 2, "the sender's headings in the frame");
    assert!(
        frame.text().contains("Twelve new knits"),
        "the sender's words in the frame"
    );
    assert_eq!(
        frame.count(".reader, .reader-body, .app, .ds-inline-banner, iframe"),
        0,
        "the window's elements in the frame"
    );
    assert_eq!(window.harness.count("iframe"), 1, "the window's frames");
    assert_eq!(
        window.harness.count("table table"),
        0,
        "the sender's tables in the window"
    );

    let (before, honest, liar, logo) = (
        frame.id(),
        frame.centre(LINK).expect("the link is drawn"),
        frame.centre(LIAR).expect("the second link is drawn"),
        frame.centre(LOGO).expect("the inline logo is drawn"),
    );
    let truth = |h: &Harness| h.attr(".ds-link-pill", "data-truth");
    assert_eq!(window.harness.count(".ds-link-pill"), 0);
    window.harness.pointer_move(honest);
    window.harness.advance(ms(100));
    assert_eq!(truth(&window.harness).as_deref(), Some("honest"));
    let said = window.harness.text_of(".ds-link-pill").unwrap_or_default();
    assert!(said.contains("example.test"), "{said}");
    window.harness.pointer_move(liar);
    window.harness.advance(ms(100));
    assert_eq!(truth(&window.harness).as_deref(), Some("lying"));
    let said = window.harness.text_of(".ds-link-pill").unwrap_or_default();
    assert!(said.contains("g00gle-security.xyz"), "{said}");
    let off = window
        .harness
        .centre(".images-hidden")
        .expect("the images line");
    window.harness.pointer_move(off);
    window.harness.advance(ms(100));
    assert_eq!(window.harness.count(".ds-link-pill"), 0, "the pill stayed");
    assert!(opened(&window).is_empty(), "hovering opened a link");
    window.harness.click(honest);
    window.harness.advance(ms(300));
    assert_eq!(opened(&window), [OFFER], "the click opened");
    let frame = window.harness.frame(FRAME).expect("still a frame document");
    assert_eq!(frame.id(), before, "the frame navigated");
    assert_eq!(frame.count(HERO), 1, "the frame lost its hero");

    window.harness.click(logo);
    until(&mut window.harness, "Peek's picture", |h| {
        h.count(".ds-peek .peek-picture") == 1
    });
    window.harness.key(Key::Escape);
    until(&mut window.harness, "Peek to close", |h| h.count(PEEK) == 0);
    assert_eq!(window.fetched.count(), 0, "fetched before consent");
    assert!(!window.original.consent().granted());
    assert_eq!(frame_width(&window.harness, HERO), 0.0);

    click(&mut window.harness, SHOW_IMAGES);
    assert!(window.original.consent().granted());
    let consented = vec![BADGE_URL.to_owned(), HERO_URL.to_owned()];
    assert_eq!(
        window.fetched.urls(),
        consented,
        "not exactly the consented images"
    );
    until(&mut window.harness, "the hero to load", |h| {
        h.frame(FRAME)
            .and_then(|frame| frame.width(HERO))
            .is_some_and(|width| width.0 == 7.0)
    });
    assert!(frame_width(&window.harness, BADGE) > 0.0);
    let hero = window
        .harness
        .frame(FRAME)
        .and_then(|frame| frame.centre(HERO))
        .expect("the hero is drawn");
    window.harness.click(hero);
    window.harness.advance(ms(300));
    assert_eq!(window.harness.count(PEEK), 0, "a remote image opened Peek");
    assert_eq!(
        opened(&window).len(),
        1,
        "an image click opened the browser"
    );
    let n = window.fetched.count();
    hover_rows(&mut window.harness, 2..=3);
    assert_eq!(window.fetched.count(), n, "hovering another row fetched");

    open_row(&mut window.harness, 2);
    assert!(
        !window.original.consent().granted(),
        "opening kept the consent"
    );
    assert_eq!(window.fetched.count(), n, "the next thread fetched unasked");
    open_row(&mut window.harness, 1);
    assert_eq!(
        window.fetched.count(),
        n,
        "the consent came back on its own"
    );
    assert_eq!(frame_width(&window.harness, HERO), 0.0);
    click(&mut window.harness, SHOW_IMAGES);
    assert_eq!(
        window.fetched.urls(),
        consented,
        "the other letter's image was fetched"
    );
    window.harness.key(Key::Escape);
    window.harness.advance(ms(400));
    assert_eq!(
        window.harness.count(FRAME),
        0,
        "Escape did not close the reader"
    );
    assert!(
        !window.original.consent().granted(),
        "closing kept the consent"
    );
}

/// Every request any document of the window makes, refused and kept.
#[derive(Default)]
struct Requests(Mutex<Vec<(RequestOrigin, String)>>);

impl AppNet for Requests {
    fn decide(&self, request: &NetRequest) -> NetDecision {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((request.origin(), request.url().to_owned()));
        NetDecision::Deny
    }

    fn fetch(&self, _request: NetRequest, _reply: NetReply) {}
}

/// Refusing the window's own document costs it nothing: it asks for nothing beyond its `file:`
/// and `data:`, so the policy that refuses the rest loses no resource of mailo's, and a frame
/// asks for no remote image while nothing is consented.
#[test]
fn the_windows_own_document_asks_for_nothing_remote() {
    let dir = tempfile::tempdir().unwrap();
    let requests = Arc::new(Requests::default());
    let config = HarnessConfig::new(VIEW)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts(dir.path()))
        .with_net(NetPolicy::Custom(requests.clone()));
    let mut harness = Harness::new(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    open_row(&mut harness, 1);
    let at = harness
        .centre(&format!("{} .ds-thread-from", thread(2)))
        .unwrap();
    harness.pointer_move(at);
    harness.advance(ms(700));
    let seen = requests
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(seen, Vec::new(), "requests were made");
}

/// (c) No script engine anywhere in the native program: not in the window, not in a frame. A
/// `<script>` in a frame (`tests/app/native_frame.rs`) is inert because there is nothing to run
/// it, and this keeps it so: no JS engine crate may enter `mail-app`'s native graph. Not
/// `--offline`: `--target all` walks crates a runner never built, and `--locked` pins the graph.
#[test]
fn no_script_engine_is_built_into_the_native_window() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let output = std::process::Command::new(cargo)
        .args([
            "tree",
            "--manifest-path",
            manifest,
            "-p",
            "mail-app",
            "--edges",
            "normal,build",
            "--target",
            "all",
            "--prefix",
            "none",
            "--format",
            "{p}",
            "--locked",
        ])
        .output()
        .expect("run cargo tree");
    assert!(
        output.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let tree = String::from_utf8_lossy(&output.stdout);
    let crates: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert!(
        crates.contains(&"blitz-dom"),
        "not the native graph:\n{tree}"
    );
    const ENGINES: [&str; 12] = [
        "boa_engine",
        "boa_runtime",
        "v8",
        "rusty_v8",
        "deno_core",
        "rquickjs",
        "quickjs-rs",
        "mozjs",
        "javascriptcore-rs",
        "webkit2gtk",
        "blitz-vibey-script",
        "wry",
    ];
    let found: Vec<&&str> = ENGINES
        .iter()
        .filter(|name| crates.contains(name))
        .collect();
    assert!(
        found.is_empty(),
        "a script engine is in the native build: {found:?}"
    );
}

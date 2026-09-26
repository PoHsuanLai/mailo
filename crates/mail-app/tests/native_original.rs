//! The reader's Original view on Blitz (`native`), in the real window: the sender's sanitized
//! markup is a sealed document of its own, its remote images are fetched only for the thread
//! the reader consented to, and a link in it opens in the browser.
//!
//! The window is `mail_app::ui::native::root` over a store seeded in a `TempDir`, with an
//! `Original` whose fetcher and browser are recorders: nothing here touches the network, the
//! real mail store or the real config. The component-level guarantees, with bodies fed around
//! the sanitizer, are `tests/native_frame.rs`.

use ds::Point;
use ds_native::{
    AppNet, Harness, HarnessConfig, NetDecision, NetReply, NetRequest, RequestOrigin, Viewport,
};
use mail_app::ui::native::{Browse, Fetch, Original};
use mail_domain::*;
use mail_runtime::{Arrival, absorb};
use mail_store::SqliteStore;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));

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
/// `class` (F146), so the frame's elements are found by where they sit: [`LOGO`], [`HERO`],
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
    let bodies: [fn(&str) -> String; 3] = [newsletter, other, plain];
    for (n, body) in bodies.iter().enumerate() {
        let date = (now - chrono::Duration::hours(n as i64 + 1)).to_rfc2822();
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

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn contexts(dir: &std::path::Path) -> ds_native::RootContexts {
    mail_app::ui::native::contexts(
        seeded(dir),
        mail_app::view::Appearance::default(),
        mail_app::space::Spaces::default(),
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
    let config = original.harness(HarnessConfig::new(VIEW).with_contexts(contexts(dir.path())));
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    Window {
        harness,
        original,
        fetched,
        opened,
        _dir: dir,
    }
}

fn row(n: usize) -> String {
    format!(".ds-list > .row:nth-child({n}) .ds-row")
}

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

fn click(harness: &mut Harness, selector: &str) {
    let at = harness
        .centre(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()));
    harness.click(at);
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

const ORIGINAL: &str = ".view-switch .ds-button:nth-child(2)";
const SHOW_IMAGES: &str = ".consent .ds-button";
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

/// The newsletter open in its Original view.
fn newsletter_original() -> Window {
    let mut window = open();
    open_row(&mut window.harness, 1);
    assert_eq!(
        window.harness.attr(ORIGINAL, "aria-label").as_deref(),
        Some("Original")
    );
    click(&mut window.harness, ORIGINAL);
    assert!(
        !window.harness.has_class(FRAME, "is-hidden"),
        "the Original view did not show"
    );
    window
}

/// (d) Consent, 0 → N → 0: nothing is fetched until "Show images", exactly the newsletter's two
/// images then, and nothing more once opening another thread has revoked it, including when the
/// newsletter is opened again.
#[test]
fn remote_images_are_fetched_only_while_consent_stands() {
    let mut window = newsletter_original();
    assert_eq!(window.fetched.count(), 0, "fetched before consent");
    assert!(!window.original.consent().granted());
    assert_eq!(frame_width(&window.harness, HERO), 0.0);

    click(&mut window.harness, SHOW_IMAGES);
    assert!(window.original.consent().granted());
    assert_eq!(
        window.fetched.urls(),
        vec![BADGE_URL.to_owned(), HERO_URL.to_owned()],
        "not exactly the consented images"
    );
    assert!(
        frame_width(&window.harness, HERO) > 0.0,
        "the hero did not show"
    );
    assert!(frame_width(&window.harness, BADGE) > 0.0);
    let n = window.fetched.count();

    // Another thread: consent is per thread, and opening revokes it.
    open_row(&mut window.harness, 2);
    assert!(
        !window.original.consent().granted(),
        "opening kept the consent"
    );
    assert_eq!(window.fetched.count(), n, "the next thread fetched unasked");

    // Back to the newsletter: asked again, not remembered.
    open_row(&mut window.harness, 1);
    click(&mut window.harness, ORIGINAL);
    assert_eq!(
        window.fetched.count(),
        n,
        "the consent came back on its own"
    );
    assert_eq!(frame_width(&window.harness, HERO), 0.0);
}

/// Consent is for the thread shown: the other letter's image is never on the newsletter's list,
/// and closing the reader takes the consent back.
#[test]
fn consent_covers_the_open_thread_only_and_closing_revokes_it() {
    let mut window = newsletter_original();
    click(&mut window.harness, SHOW_IMAGES);
    assert!(!window.fetched.urls().contains(&OTHER.to_owned()));
    window.harness.key(ds::Key::Escape);
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

/// Remote images are never fetched because something was hovered: not the rows (whose cards
/// open), with nothing consented and with the newsletter consented.
#[test]
fn hovering_fetches_nothing() {
    let mut window = open();
    for n in 1..=3 {
        let at = window
            .harness
            .centre(&format!("{} .ds-row-name", row(n)))
            .expect("a row's sender");
        window.harness.pointer_move(at);
        window.harness.advance(ds::delays::HOVER_OPEN + ms(250));
    }
    assert_eq!(window.fetched.count(), 0, "hovering fetched");

    let mut window = newsletter_original();
    click(&mut window.harness, SHOW_IMAGES);
    let n = window.fetched.count();
    for n in 2..=3 {
        let at = window
            .harness
            .centre(&format!("{} .ds-row-name", row(n)))
            .expect("a row's sender");
        window.harness.pointer_move(at);
        window.harness.advance(ds::delays::HOVER_OPEN + ms(250));
    }
    assert_eq!(window.fetched.count(), n, "hovering another row fetched");
}

/// `cid:` inline images resolve inside the frame, as `data:` in its own markup (F42): shown
/// before any consent, with no request made for them.
#[test]
fn the_inline_image_shows_without_a_request() {
    let window = newsletter_original();
    assert_eq!(frame_width(&window.harness, LOGO), 7.0);
    let frame = window.harness.frame(FRAME).expect("a frame document");
    let src = frame.attr(LOGO, "src").unwrap_or_default();
    assert!(src.starts_with("data:image/png;base64,"), "{src}");
    assert_eq!(window.fetched.count(), 0);
}

/// (e) A link in the frame opens in the browser, through mailo, and the frame stays as it was.
/// What opens is the link without its tracking parameters, and with its own.
#[test]
fn a_link_in_the_frame_opens_in_the_browser_and_the_frame_stays() {
    let mut window = newsletter_original();
    let (before, at) = {
        let frame = window.harness.frame(FRAME).expect("a frame document");
        (frame.id(), frame.centre(LINK).expect("the link is drawn"))
    };
    window.harness.click(at);
    window.harness.advance(ms(300));
    let opened = window
        .opened
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    assert_eq!(opened, vec![OFFER.to_owned()]);
    let frame = window.harness.frame(FRAME).expect("still a frame document");
    assert_eq!(frame.id(), before, "the frame navigated");
    assert_eq!(frame.count(HERO), 1);
    assert_eq!(window.fetched.count(), 0, "the click fetched");
}

/// The link pill shows where a link in the frame goes as the pointer comes onto it, read by the
/// same honesty check as a Reader view link, and goes as the pointer leaves.
#[test]
fn hovering_a_link_in_the_frame_shows_where_it_goes() {
    let mut window = newsletter_original();
    let (honest, liar) = {
        let frame = window.harness.frame(FRAME).expect("a frame document");
        (
            frame.centre(LINK).expect("the link is drawn"),
            frame.centre(LIAR).expect("the second link is drawn"),
        )
    };
    assert_eq!(window.harness.count(".ds-link-pill"), 0);
    window.harness.pointer_move(honest);
    window.harness.advance(ms(100));
    assert_eq!(
        window
            .harness
            .attr(".ds-link-pill", "data-truth")
            .as_deref(),
        Some("honest"),
        "no pill for the frame's link:\n{}",
        window.harness.html()
    );
    let said = window.harness.text_of(".ds-link-pill").unwrap_or_default();
    assert!(said.contains("example.test"), "{said}");
    // From one link to the next: the pill follows, and says the text lies.
    window.harness.pointer_move(liar);
    window.harness.advance(ms(100));
    assert_eq!(
        window
            .harness
            .attr(".ds-link-pill", "data-truth")
            .as_deref(),
        Some("lying")
    );
    let said = window.harness.text_of(".ds-link-pill").unwrap_or_default();
    assert!(said.contains("g00gle-security.xyz"), "{said}");
    // Off every link: the pill goes, and nothing was opened.
    let off = window
        .harness
        .centre(".consent")
        .expect("the consent strip");
    window.harness.pointer_move(off);
    window.harness.advance(ms(100));
    assert_eq!(window.harness.count(".ds-link-pill"), 0, "the pill stayed");
    assert!(
        window
            .opened
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
}

/// (a) No shared DOM in the real window: the sender's elements are found in the frame and never
/// by the window's queries, and the frame holds none of the window's elements.
#[test]
fn the_window_and_the_frame_share_no_nodes() {
    let window = newsletter_original();
    let frame = window.harness.frame(FRAME).expect("a frame document");
    assert_eq!(frame.count("h3"), 2);
    assert!(frame.text().contains("Twelve new knits"));
    assert_eq!(
        frame.count(".reader, .reader-body, .app, .consent, iframe"),
        0
    );
    // The window's own copy of the words is its blocks (the Reader view, hidden), never the
    // frame's elements: the window finds one frame and none of the sender's tables.
    assert_eq!(window.harness.count("iframe"), 1);
    assert_eq!(window.harness.count("table table"), 0);
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
        .with_contexts(contexts(dir.path()))
        .with_net(ds_native::NetPolicy::Custom(requests.clone()));
    let mut harness = Harness::with_config(mail_app::ui::native::root, config);
    harness.advance(ms(300));
    open_row(&mut harness, 1);
    click(&mut harness, ORIGINAL);
    let at = harness.centre(&format!("{} .ds-row-name", row(2))).unwrap();
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
/// `<script>` in a frame (`tests/native_frame.rs`) is inert because there is nothing to run
/// it, and this keeps it so: no JS engine crate may enter `mail-app`'s native graph.
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
            "--offline",
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

/// Pictures of the newsletter's Original view, images blocked and then consented, painted
/// headlessly by Blitz.
#[test]
#[ignore = "picture generator: set MAILO_SNAPSHOT_DIR to a directory and run with --ignored"]
fn snapshot() {
    let dir = std::env::var("MAILO_SNAPSHOT_DIR").expect("MAILO_SNAPSHOT_DIR names a directory");
    let mut window = newsletter_original();
    window.harness.advance(ms(600));
    let blocked = window.harness.render().unwrap();
    blocked.save(format!("{dir}/original-blocked.png")).unwrap();
    click(&mut window.harness, SHOW_IMAGES);
    window.harness.advance(ms(600));
    let consented = window.harness.render().unwrap();
    consented
        .save(format!("{dir}/original-consented.png"))
        .unwrap();
}

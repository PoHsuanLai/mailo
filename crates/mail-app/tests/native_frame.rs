//! The Original view's guarantees on Blitz, against the reader's real frame
//! (`mail_app::ui::native::OriginalFrame`, the `Sandbox` iframe in mailo's stylesheet) in a
//! headless Blitz document (`ds_native::Harness`).
//!
//! The webview's frame was held by its markup, and `src/ui/reading/tests.rs` asserts that markup
//! (`sandbox=""`, no parent between the article and the iframe, the "sandboxed frame" note).
//! Blitz ignores `sandbox`; what holds the frame there is that it is a separate document, and
//! what it may reach is mailo's to say (`ui/original`). So these assert the behaviour itself.
//! Each body here is fed *around* the sanitizer, as a parser differential or an ammonia miss
//! would deliver it: the guarantee must hold for markup the sanitizer should never have let
//! through. The same guarantees in the real window, through the sanitizer, are
//! `tests/native_original.rs`.
//!
//! The pairs, the webview's markup (gone with the webview) ↔ Blitz:
//! - `sandbox=""` without `allow-same-origin` ↔ `the_frame_and_the_window_share_no_nodes`;
//! - the frame being its own document ↔ `sender_css_cannot_reach_the_window`,
//!   `the_windows_css_cannot_reach_the_frame`;
//! - `sandbox=""` without `allow-scripts` ↔ `a_script_in_the_frame_does_nothing`, and
//!   `native_original.rs`'s check that no JS engine is built in;
//! - the sanitizer's blocked `src` ↔ `nothing_is_fetched_without_consent`,
//!   `a_frame_is_never_served_a_file`;
//! - `sandbox=""` without `allow-popups`/`allow-top-navigation` ↔
//!   `a_link_click_never_navigates_the_frame`.
//!
//! Each case runs twice: under mailo's `Original` (the window's), and under `NetPolicy::Local`
//! with `FrameLinks::Inert`, the mechanism before mailo held the frames' network itself.

use dioxus::prelude::*;
use ds_native::{FrameLinks, Harness, HarnessConfig, NetPolicy, Viewport};
use mail_app::ui::native::{Browse, Fetch, Original, OriginalFrame};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 800,
    height: 600,
    scale_percent: 100,
};

/// What the test window draws: the sender's markup, and the window's own extra CSS.
#[derive(Clone)]
struct Page {
    body: String,
    css: String,
}

/// A window with one element of its own beside the reader's Original frame.
#[allow(non_snake_case)]
fn Window() -> Element {
    let page = use_context::<Page>();
    rsx! {
        style { {page.css} }
        div { class: "window",
            p { class: "b b-note ours", span { class: "probe", "MMMM" } }
            span { class: "tok-app", style: "display: inline-block", "x" }
            OriginalFrame { html: page.body }
        }
    }
}

/// A 7 × 5 PNG: an image that loads lays out 7 px wide, one that does not, 0.
fn swatch() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(7, 5, image::Rgba([0, 160, 0, 255]))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode the swatch");
    bytes
}

/// Every URL fetched, answered with [`swatch`] at once.
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
    fn urls(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Every link opened.
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

impl Opened {
    fn urls(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// How the frames are held: as the window holds them now, or as they were held before.
enum Held {
    /// mailo's `Original`, with recorders for its fetches and its browser.
    Mailo(Original, Arc<Fetched>, Arc<Opened>),
    /// `NetPolicy::Local`, `FrameLinks::Inert`: `data:` only, clicks dropped.
    Before,
}

impl Held {
    fn mailo() -> Held {
        let fetched = Arc::new(Fetched::default());
        let opened = Arc::new(Opened::default());
        Held::Mailo(
            Original::new(fetched.clone(), opened.clone()),
            fetched,
            opened,
        )
    }

    fn both() -> [Held; 2] {
        [Held::mailo(), Held::Before]
    }

    fn name(&self) -> &'static str {
        match self {
            Held::Mailo(..) => "mailo's Original",
            Held::Before => "Local + Inert",
        }
    }

    fn config(&self, page: Page) -> HarnessConfig {
        let config = HarnessConfig::new(VIEW).with_context(page);
        match self {
            Held::Mailo(original, ..) => original.harness(config),
            Held::Before => config
                .with_net(NetPolicy::Local)
                .with_frame_links(FrameLinks::Inert),
        }
    }

    fn fetched(&self) -> Vec<String> {
        match self {
            Held::Mailo(_, fetched, _) => fetched.urls(),
            Held::Before => Vec::new(),
        }
    }
}

fn open(held: &Held, body: &str, css: &str) -> Harness {
    let page = Page {
        body: body.to_owned(),
        css: css.to_owned(),
    };
    let mut harness = Harness::with_config(Window, held.config(page));
    harness.advance(Duration::from_millis(50));
    harness
}

const FRAME: &str = "iframe.html";

fn width(harness: &Harness, selector: &str) -> f32 {
    harness
        .rect(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn:\n{}", harness.html()))
        .size
        .width
        .0
}

fn frame_width(harness: &Harness, selector: &str) -> f32 {
    let frame = harness.frame(FRAME).expect("the frame has a document");
    frame
        .width(selector)
        .unwrap_or_else(|| panic!("{selector} is not in the frame:\n{}", frame.html()))
        .0
}

/// (a) No same-origin: the window's queries never see the frame's nodes, and the frame's never
/// see the window's. A class the sender copies from mailo's markup finds only its own element.
#[test]
fn the_frame_and_the_window_share_no_nodes() {
    let body = "<p class='b b-note theirs'>from the sender</p><div class='reader-body'></div>";
    for held in Held::both() {
        let harness = open(&held, body, "");
        let frame = harness.frame(FRAME).expect("the frame has a document");
        let name = held.name();
        assert_eq!(
            harness.count(".b-note"),
            1,
            "{name}: the window saw into the frame"
        );
        assert_eq!(harness.count(".theirs"), 0, "{name}");
        assert_eq!(frame.count(".b-note"), 1, "{name}");
        assert_eq!(
            frame.count(".ours, .probe, .tok-app"),
            0,
            "{name}: the frame saw the window"
        );
        assert_eq!(
            frame.count(".reader-body"),
            1,
            "{name}: its own copy, not ours"
        );
        assert!(!frame.text().contains("MMMM"), "{name}");
        let ours = harness.text_of(".window > .b-note").unwrap_or_default();
        assert!(!ours.contains("from the sender"), "{name}: {ours}");
    }
}

/// (b) Sender CSS cannot touch mailo: a `<style>` resurrected in the frame, aimed at every
/// element and at mailo's own classes, leaves the window's elements exactly as they were.
#[test]
fn sender_css_cannot_reach_the_window() {
    let hostile = "<style>* { font-size: 64px !important; letter-spacing: 30px !important; \
        padding: 0 50px !important } .probe, .b-note, span { display: none !important }</style>\
        <p>hi</p>";
    for held in Held::both() {
        let name = held.name();
        let plain = width(&open(&held, "<p>hi</p>", ""), ".probe");
        let attacked = open(&held, hostile, "");
        assert!(plain > 0.0, "{name}: the probe is not drawn");
        assert_eq!(
            width(&attacked, ".probe"),
            plain,
            "{name}: the sender's CSS reached the window"
        );
        assert_eq!(attacked.count(".probe"), 1, "{name}");
    }
}

/// (b) mailo's CSS cannot reach the frame, nor can its custom properties (the way every quire
/// token is spelled): the frame's span is as wide as in a window with no CSS of its own, however
/// hard the window's rules insist.
#[test]
fn the_windows_css_cannot_reach_the_frame() {
    let body = "<span class='probe' style='display: inline-block'>MMMM</span>\
        <span class='tok' style='display: inline-block; padding-left: var(--probe-pad)'>x</span>\
        <span class='bare' style='display: inline-block'>x</span>";
    let loud = ":root, html, body, * { --probe-pad: 40px } \
        span, .probe { font-size: 64px !important; padding: 0 40px !important } \
        .tok-app { padding-left: var(--probe-pad) }";
    for held in Held::both() {
        let name = held.name();
        let quiet = open(&held, body, "");
        let shouted = open(&held, body, loud);
        assert_eq!(
            frame_width(&shouted, ".probe"),
            frame_width(&quiet, ".probe"),
            "{name}: the window's CSS reached the frame"
        );
        // The token resolves in the window and not in the frame.
        assert!(
            width(&shouted, ".tok-app") > width(&quiet, ".tok-app") + 30.0,
            "{name}: the token did not resolve in the window, so this proves nothing"
        );
        assert_eq!(
            frame_width(&shouted, ".tok"),
            frame_width(&shouted, ".bare"),
            "{name}: the window's custom property reached the frame"
        );
    }
}

/// (c) No scripts: a `<script>` in the frame is inert text in its DOM.
#[test]
fn a_script_in_the_frame_does_nothing() {
    let body = "<p class='inner'>hello</p>\
        <script>document.body.innerHTML = '<p class=\"pwned\">rewritten</p>'; \
        parent.document.body.innerHTML = '<p class=\"pwned\">rewritten</p>';</script>\
        <img src='data:image/png;base64,iVBORw0KGgo=' onerror=\"document.title='pwned'\">";
    for held in Held::both() {
        let mut harness = open(&held, body, "");
        harness.advance(Duration::from_millis(200));
        let frame = harness.frame(FRAME).expect("the frame has a document");
        let name = held.name();
        assert_eq!(frame.count(".pwned"), 0, "{name}");
        assert_eq!(frame.count(".inner"), 1, "{name}");
        assert_eq!(harness.count(".pwned"), 0, "{name}");
        assert_eq!(harness.count(".probe"), 1, "{name}");
    }
}

/// (d) Zero requests without consent: remote images in the frame's markup (which the sanitizer
/// would have stripped) are neither fetched nor shown.
#[test]
fn nothing_is_fetched_without_consent() {
    let body = "<img class='remote' src='https://tracker.example.test/pixel.png'>\
        <img class='plain' src='http://cdn.example.test/a.png'>";
    for held in Held::both() {
        let harness = open(&held, body, "");
        let name = held.name();
        assert_eq!(held.fetched(), Vec::<String>::new(), "{name}");
        assert_eq!(frame_width(&harness, ".remote"), 0.0, "{name}");
        assert_eq!(frame_width(&harness, ".plain"), 0.0, "{name}");
    }
}

/// (d) The frame is never served `file:`. (Nor with consent: a consented list is read through a
/// web-only filter, `ui/original/consent_tests.rs`.)
#[test]
fn a_frame_is_never_served_a_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("on-disk.png");
    std::fs::write(&path, swatch()).expect("write the swatch");
    let url = format!("file://{}", path.display());
    let body = format!("<img class='disk' src='{url}'>");
    for held in Held::both() {
        let harness = open(&held, &body, "");
        let name = held.name();
        assert_eq!(
            frame_width(&harness, ".disk"),
            0.0,
            "{name}: the frame read the disk"
        );
        assert_eq!(held.fetched(), Vec::<String>::new(), "{name}");
    }
}

/// (e) No navigation: a link clicked in the frame reloads nothing and fetches nothing. Under
/// mailo's `Original` the click is heard and handed to the browser, checked: a `file:` or
/// `javascript:` link opens nothing.
#[test]
fn a_link_click_never_navigates_the_frame() {
    let body = "<body style='margin:0'>\
        <a class='web' href='https://shop.example.test/offer' style='display:block; font-size:40px'>Offer</a>\
        <a class='disk' href='file:///etc/passwd' style='display:block; font-size:40px'>Disk</a>\
        <a class='js' href='javascript:alert(1)' style='display:block; font-size:40px'>Script</a>\
        </body>";
    for held in Held::both() {
        let name = held.name();
        let mut harness = open(&held, body, "");
        let before = harness.frame(FRAME).expect("a frame document").id();
        for link in ["a.web", "a.disk", "a.js"] {
            let at = harness
                .frame(FRAME)
                .and_then(|frame| frame.centre(link))
                .unwrap_or_else(|| panic!("{name}: {link} is not drawn"));
            harness.click(at);
            harness.advance(Duration::from_millis(50));
        }
        let after = harness.frame(FRAME).expect("still a frame document");
        assert_eq!(after.id(), before, "{name}: the frame navigated");
        assert_eq!(after.text_of("a.web").as_deref(), Some("Offer"), "{name}");
        assert_eq!(held.fetched(), Vec::<String>::new(), "{name}");
        if let Held::Mailo(_, _, opened) = &held {
            assert_eq!(
                opened.urls(),
                vec!["https://shop.example.test/offer".to_owned()],
                "{name}"
            );
        }
    }
}

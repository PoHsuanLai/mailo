//! The Reader view's remote images on Blitz: fetched by mailo while consent stands, drawn as
//! `data:`.
//!
//! The fetcher is a recorder that answers only when the test says so: nothing here touches the
//! network. The reader is drawn on its own, over a store in a `TempDir`.

use super::super::Reader;
use super::super::tests::{add_thread, add_to, html_message, thread_of};
use crate::ui::fixtures::{click, dispatching, rebuild_into};
use crate::ui::original::{FetchImage, Got, ReaderNet, data_uri};
use crate::view::Shell;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::ThreadId;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

const A: &str = "https://images.example.test/a.png";
const B: &str = "https://images.example.test/b.png";
const C: &str = "https://images.example.test/c.png";
const ELSEWHERE: &str = "https://cdn.example.test/other.png";

type Done = Box<dyn FnOnce(Got) + Send>;

/// Every URL asked for, in order, and the answers not given yet.
#[derive(Clone, Default)]
struct Recorder {
    asked: Arc<Mutex<Vec<String>>>,
    waiting: Arc<Mutex<Vec<(String, Done)>>>,
}

impl FetchImage for Recorder {
    fn get(&self, url: String, done: Done) {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(url.clone());
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((url, done));
    }
}

impl Recorder {
    fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Answer everything asked so far with what `answer` says.
    fn answer(&self, answer: impl Fn(&str) -> Got) {
        let waiting: Vec<_> = self
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect();
        for (url, done) in waiting {
            done(answer(&url));
        }
    }
}

/// A 1 × 1 PNG's first bytes, and a little more.
fn png() -> Got {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
    Got {
        content_type: Some("image/png".to_owned()),
        bytes,
    }
}

fn page(html: &str) -> Vec<u8> {
    html_message("the autumn letter", html)
}

fn img(url: &str) -> String {
    format!("<p><img src=\"{url}\" alt=\"a picture\" width=\"40\" height=\"30\"></p>")
}

/// The reader on `first`, with the consent already given when `consented`; the button labelled
/// "Open the other" opens `second`, as a click on a row does (`Shell::open`, which clears the
/// consent), and "Draw again" draws the reader again, as the window does when mail lands.
#[component]
fn Pane(first: ThreadId, second: ThreadId, consented: bool) -> Element {
    let mut shell = use_signal(|| Shell {
        open: Some(first),
        show_remote_images: consented,
        ..Shell::default()
    });
    let other = "Open the other".to_owned();
    let again = "Draw again".to_owned();
    let thread = shell.read().open.unwrap_or(first);
    rsx! {
        button { aria_label: "{other}", onclick: move |_| shell.write().open(second) }
        button {
            aria_label: "{again}",
            onclick: move |_| {
                shell.write();
            },
        }
        Reader { thread, shell }
    }
}

struct Open {
    dom: VirtualDom,
    net: Recorder,
    seen: crate::ui::fixtures::Seen,
    store: Arc<mail_store::SqliteStore>,
    first: ThreadId,
    _dir: tempfile::TempDir,
}

fn open(first: &str, second: &str) -> Open {
    open_with(first, second, false)
}

fn open_with(first: &str, second: &str, consented: bool) -> Open {
    dispatching();
    let (store, first, dir) = thread_of(&[("the autumn letter", page(first))]);
    let second = add_thread(&store, "other", &[("another letter", page(second))]);
    let net = Recorder::default();
    let props = PaneProps {
        first,
        second,
        consented,
    };
    let mut dom = VirtualDom::new_with_props(Pane, props)
        .with_root_context(store.clone())
        .with_root_context(ReaderNet(Arc::new(net.clone())));
    let seen = rebuild_into(&mut dom);
    Open {
        dom,
        net,
        seen,
        store,
        first,
        _dir: dir,
    }
}

impl Open {
    /// Poll the window's tasks and draw, until `done` or a few rounds have passed.
    async fn settle(&mut self, done: impl Fn(&Open) -> bool) {
        for _ in 0..40 {
            if done(self) {
                break;
            }
            tokio::time::timeout(Duration::from_millis(50), self.dom.wait_for_work())
                .await
                .ok();
            self.dom.render_immediate(&mut dioxus_core::NoOpMutations);
        }
        self.dom.render_immediate(&mut dioxus_core::NoOpMutations);
    }

    fn markup(&self) -> String {
        dioxus_ssr::render(&self.dom)
    }
}

fn data_images(markup: &str) -> usize {
    markup.matches("src=\"data:image/png;base64,").count()
}

#[tokio::test]
async fn nothing_is_fetched_before_consent() {
    let mut open = open(&format!("{}{}", img(A), img(B)), &img(ELSEWHERE));
    open.settle(|_| false).await;
    assert!(open.net.asked().is_empty(), "{:?}", open.net.asked());
    let markup = open.markup();
    assert!(
        !markup.contains(A),
        "the URL reached the document:\n{markup}"
    );
    assert_eq!(data_images(&markup), 0);
}

#[tokio::test]
async fn a_reader_mounted_with_consent_already_given_fetches_at_once() {
    // Consent stands when the reader is drawn, as it does when the reader is drawn again for the
    // same thread: its images are fetched without another click, and nothing offers to load them.
    let mut open = open_with(&img(A), &img(ELSEWHERE), true);
    open.settle(|open| !open.net.asked().is_empty()).await;
    assert_eq!(open.net.asked(), vec![A.to_owned()]);
    let loading = open.markup();
    assert!(loading.contains("Loading the image from"), "{loading}");
    assert!(!loading.contains("load images"), "{loading}");

    open.net.answer(|_| png());
    open.settle(|open| data_images(&open.markup()) == 1).await;
    let markup = open.markup();
    assert_eq!(data_images(&markup), 1, "{markup}");
    assert!(!markup.contains(&format!("src=\"{A}\"")), "{markup}");
    assert_eq!(open.net.asked().len(), 1, "an image was fetched twice");
}

#[tokio::test]
async fn a_message_that_lands_while_consent_stands_is_fetched_too() {
    let mut open = open(&img(A), &img(ELSEWHERE));
    let show = open.seen.one("aria-label", "Show images");
    click(&mut open.dom, show);
    open.settle(|open| !open.net.asked().is_empty()).await;
    open.net.answer(|_| png());
    open.settle(|open| data_images(&open.markup()) == 1).await;
    assert_eq!(open.net.asked(), vec![A.to_owned()]);

    // A reply lands in the same thread, with one image already shown and one new one.
    add_to(
        &open.store,
        open.first,
        "reply",
        &[("the reply", page(&format!("{}{}", img(A), img(B))))],
    );
    let again = open.seen.one("aria-label", "Draw again");
    click(&mut open.dom, again);
    open.settle(|open| open.net.asked().len() >= 2).await;
    assert_eq!(
        open.net.asked(),
        vec![A.to_owned(), B.to_owned()],
        "only the new image is asked for"
    );

    open.net.answer(|_| png());
    open.settle(|open| data_images(&open.markup()) == 3).await;
    let markup = open.markup();
    assert_eq!(data_images(&markup), 3, "{markup}");
    assert!(!markup.contains("Loading the image from"), "{markup}");
}

#[tokio::test]
async fn consent_fetches_each_image_once_and_draws_the_images_as_data() {
    // Three images, one of them twice; the third answer is a page, not a picture.
    let body = format!("{}{}{}{}", img(A), img(B), img(A), img(C));
    let mut open = open(&body, &img(ELSEWHERE));
    let show = open.seen.one("aria-label", "Show images");
    click(&mut open.dom, show);
    open.settle(|open| open.net.asked().len() >= 3).await;
    let mut asked = open.net.asked();
    asked.sort();
    assert_eq!(asked, vec![A.to_owned(), B.to_owned(), C.to_owned()]);

    let loading = open.markup();
    assert!(loading.contains("Loading the image from"), "{loading}");
    assert_eq!(data_images(&loading), 0);

    open.net.answer(|url| {
        if url == C {
            Got {
                content_type: Some("text/html".to_owned()),
                bytes: b"<script>alert(1)</script>".to_vec(),
            }
        } else {
            png()
        }
    });
    open.settle(|open| data_images(&open.markup()) == 3).await;
    let markup = open.markup();
    assert_eq!(data_images(&markup), 3, "{markup}");
    assert_eq!(markup.matches("could not be shown").count(), 1, "{markup}");
    assert!(!markup.contains("src=\"https:"), "{markup}");
    assert!(!markup.contains("<script"), "{markup}");
    assert_eq!(open.net.asked().len(), 3, "an image was fetched twice");
}

#[tokio::test]
async fn another_thread_fetches_nothing_and_what_lands_late_is_dropped() {
    let mut open = open(&format!("{}{}", img(A), img(B)), &img(ELSEWHERE));
    let show = open.seen.one("aria-label", "Show images");
    click(&mut open.dom, show);
    open.settle(|open| open.net.asked().len() >= 2).await;
    assert_eq!(open.net.asked().len(), 2);

    let other = open.seen.one("aria-label", "Open the other");
    click(&mut open.dom, other);
    open.net.answer(|_| png());
    open.settle(|_| false).await;

    let markup = open.markup();
    assert_eq!(
        data_images(&markup),
        0,
        "a revoked image was drawn:\n{markup}"
    );
    assert!(markup.contains("another letter"), "{markup}");
    assert!(!markup.contains(ELSEWHERE), "{markup}");
    assert_eq!(
        open.net.asked().len(),
        2,
        "the other thread was fetched: {:?}",
        open.net.asked()
    );
}

#[test]
fn only_a_raster_image_within_the_cap_becomes_a_data_uri() {
    let got = |content_type: Option<&str>, bytes: Vec<u8>| Got {
        content_type: content_type.map(str::to_owned),
        bytes,
    };
    let png_bytes = png().bytes;
    let jpeg = vec![0xff, 0xd8, 0xff, 0xe0, 0, 16];
    let gif = b"GIF89a\x01\x00\x01\x00".to_vec();
    let webp = b"RIFF\x10\x00\x00\x00WEBPVP8 ".to_vec();
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec();
    let mut huge = png_bytes.clone();
    huge.resize(16 * 1024 * 1024 + 1, 0);
    let cases: Vec<(&str, Got, Option<&str>)> = vec![
        (
            "png",
            got(Some("image/png"), png_bytes.clone()),
            Some("image/png"),
        ),
        (
            "jpeg with a parameter",
            got(Some("IMAGE/JPEG; q=1"), jpeg.clone()),
            Some("image/jpeg"),
        ),
        ("gif", got(Some("image/gif"), gif), Some("image/gif")),
        ("webp", got(Some("image/webp"), webp), Some("image/webp")),
        // The type written is what the bytes are, never the server's word for them.
        (
            "mislabelled raster",
            got(Some("image/png"), jpeg),
            Some("image/jpeg"),
        ),
        ("svg", got(Some("image/svg+xml"), svg.clone()), None),
        ("svg called a png", got(Some("image/png"), svg), None),
        ("html", got(Some("text/html"), b"<p>hi</p>".to_vec()), None),
        (
            "png called html",
            got(Some("text/html"), png_bytes.clone()),
            None,
        ),
        ("no type", got(None, png_bytes), None),
        ("empty", got(Some("image/png"), Vec::new()), None),
        ("past the cap", got(Some("image/png"), huge), None),
    ];
    for (name, got, want) in cases {
        let uri = data_uri(&got);
        match want {
            Some(kind) => assert!(
                uri.as_deref()
                    .is_some_and(|uri| uri.starts_with(&format!("data:{kind};base64,"))),
                "{name}: {:?}",
                uri.map(|uri| uri.chars().take(40).collect::<String>())
            ),
            None => assert!(uri.is_none(), "{name} was drawn"),
        }
    }
}

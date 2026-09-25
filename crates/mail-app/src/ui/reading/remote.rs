//! The Reader view's remote images on Blitz (`native`): fetched by mailo, drawn as `data:`.
//!
//! On the webview a consented image is an `<img src=https…>` and the webview fetches it. On Blitz
//! the window's own document is refused every remote request (`original/net.rs`: a request from it
//! cannot be tied to the reader, and a hover card is the same document). So here mailo fetches:
//! - **Only what the consented reader draws.** The list is the `ImgSrc::Remote` URLs, http(s)
//!   only, of the blocks the reader's own render built under the consenting policy
//!   ([`wanted`]): what the sanitizer and the block builder kept, and nothing else. Without
//!   consent that list is empty, so nothing is asked for.
//! - **From an effect, never a render (F140).** [`Fetcher`] is a child of the reader whose effect
//!   runs whenever that list or the thread changes, and whose tasks are its own scope's, so they
//!   end with the reader. That covers "Show images", a reader mounted with the consent already
//!   given, and a message that arrives in the thread while the consent stands. A hover never
//!   fetches: it draws no reader.
//! - **Each image once per grant.** An image already asked for is not asked for again.
//! - **Only through [`ReaderNet`]**, the Original frame's client: no cookies, no `Referer`, three
//!   redirects, twenty seconds, 16 MiB.
//! - **Only a raster image is drawn**, as a `data:` URI ([`data_uri`]); anything else is refused.
//! - **Only while the grant stands.** Another thread, or the consent cleared, drops every image
//!   and every answer still on its way ([`Pictures::hold`]).

use super::super::original::{Got, ReaderNet, data_uri};
use dioxus::prelude::*;
use futures_util::StreamExt as _;
use mail_domain::ThreadId;
use mail_mime::{Block, Document, ImgSrc};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Where one remote image is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Picture {
    /// Asked for; nothing back yet.
    Loading,
    /// Fetched, and a raster image: its `data:` URI.
    Shown(String),
    /// Fetched and not an image to show, or not fetched at all.
    Refused,
}

/// What the reader's grant holds: which thread, and what has landed for it.
#[derive(Default)]
struct Drawn {
    /// Moved by every grant and every revoke; a fetch carries the value it started under.
    generation: u64,
    /// The thread whose images are granted, if any.
    granted: Option<ThreadId>,
    /// The thread the reader shows now, and whether its images are consented.
    showing: Option<(ThreadId, bool)>,
    images: HashMap<String, Picture>,
}

impl Drawn {
    fn revoke(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.granted = None;
        self.images.clear();
    }
}

/// One reader's remote images: a context the reader provides for its blocks.
#[derive(Clone)]
pub(super) struct Pictures {
    drawn: Rc<RefCell<Drawn>>,
    /// Moved when an image lands, so the blocks that read it are drawn again.
    landed: Signal<u64>,
    net: Option<ReaderNet>,
}

impl Pictures {
    /// From the reader's hook: the window's fetcher, if it has one.
    pub(super) fn new() -> Self {
        Pictures {
            drawn: Rc::default(),
            landed: Signal::new(0),
            net: try_consume_context::<ReaderNet>(),
        }
    }

    /// The reader shows `thread`, with images consented (`showing`) or not. Called on every
    /// render: another thread, or no consent, drops what was granted. Writes no signal.
    pub(super) fn hold(&self, thread: ThreadId, showing: bool) {
        let mut drawn = self.drawn.borrow_mut();
        drawn.showing = Some((thread, showing));
        if drawn.granted.is_some() && (!showing || drawn.granted != Some(thread)) {
            drawn.revoke();
        }
    }

    /// What there is for `url` now. Subscribes the caller to what lands.
    pub(super) fn picture(&self, url: &str) -> Option<Picture> {
        let _ = (self.landed)();
        self.drawn.borrow().images.get(url).cloned()
    }

    fn stands(&self, generation: u64) -> bool {
        let drawn = self.drawn.borrow();
        drawn.generation == generation && drawn.granted.is_some()
    }

    fn landed(&self) {
        // The reader may be gone: then nothing is drawn, and nothing needs to be.
        let mut landed = self.landed;
        if let Ok(mut landed) = landed.try_write() {
            *landed = landed.wrapping_add(1);
        }
    }

    /// Grant `thread` if the reader still shows it consented, and fetch those of `wanted` not
    /// asked for yet under this grant. From [`Fetcher`]'s effect: the task is that scope's.
    fn fetch(&self, thread: ThreadId, wanted: &[String]) {
        let (generation, fresh) = {
            let mut drawn = self.drawn.borrow_mut();
            if wanted.is_empty() || drawn.showing != Some((thread, true)) {
                return;
            }
            if drawn.granted != Some(thread) {
                drawn.revoke();
                drawn.granted = Some(thread);
            }
            let fresh: Vec<String> = wanted
                .iter()
                .filter(|url| !drawn.images.contains_key(*url))
                .cloned()
                .collect();
            let asked = if self.net.is_some() {
                Picture::Loading
            } else {
                Picture::Refused
            };
            for url in &fresh {
                drawn.images.insert(url.clone(), asked.clone());
            }
            (drawn.generation, fresh)
        };
        if fresh.is_empty() {
            return;
        }
        self.landed();
        let Some(net) = self.net.clone() else {
            return;
        };
        let mut waiting = futures_util::stream::FuturesUnordered::new();
        for url in fresh {
            let (send, got) = tokio::sync::oneshot::channel::<Got>();
            net.0.get(
                url.clone(),
                Box::new(move |answer| {
                    let _ = send.send(answer);
                }),
            );
            waiting.push(async move { (url, got.await.ok()) });
        }
        let this = self.clone();
        spawn(async move {
            while let Some((url, got)) = waiting.next().await {
                // Answers that land after a revoke are dropped, and so is the rest of the wait.
                if !this.stands(generation) {
                    return;
                }
                let picture = got
                    .as_ref()
                    .and_then(data_uri)
                    .map_or(Picture::Refused, Picture::Shown);
                this.drawn.borrow_mut().images.insert(url, picture);
                this.landed();
            }
        });
    }
}

/// Fetches what the reader draws of `thread`'s remote images, `wanted`, from an effect that runs
/// again whenever either changes. Draws nothing. A child of the reader, under its [`Pictures`].
#[component]
pub(super) fn Fetcher(thread: ThreadId, wanted: Vec<String>) -> Element {
    let pictures = use_context::<Pictures>();
    use_effect(use_reactive((&thread, &wanted), move |(thread, wanted)| {
        pictures.fetch(thread, &wanted);
    }));
    rsx! {}
}

/// The http(s) URLs of the remote images in `documents`, nested ones included, each once.
pub(super) fn wanted<'a>(documents: impl IntoIterator<Item = &'a Document>) -> Vec<String> {
    let mut urls = Vec::new();
    for document in documents {
        collect(&document.blocks, &mut urls);
    }
    urls
}

/// Every remote image in `blocks`, nested ones included, onto `urls`.
fn collect(blocks: &[Block], urls: &mut Vec<String>) {
    let mut stack: Vec<&[Block]> = vec![blocks];
    while let Some(level) = stack.pop() {
        for block in level {
            match block {
                Block::Image {
                    src: ImgSrc::Remote(url),
                    ..
                } => {
                    let url = url.as_str();
                    let web = url.starts_with("https://") || url.starts_with("http://");
                    if web && !urls.iter().any(|seen| seen == url) {
                        urls.push(url.to_owned());
                    }
                }
                Block::List { items, .. } => stack.extend(items.iter().map(Vec::as_slice)),
                Block::Quote { blocks, .. } | Block::Signature(blocks) => stack.push(blocks),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "remote_tests.rs"]
mod tests;

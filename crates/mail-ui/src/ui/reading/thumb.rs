//! A stored attachment's thumbnail in the reader's strip, and the press that opens it in Peek.
//!
//! Every stored part's leading cell opens Peek (`peek.rs`): a click, or Space or Return while it
//! has the keyboard. Drawn only for a part whose bytes are here (`Kept::Here`): a part still on the server shows
//! its paperclip until it is downloaded, and nothing is fetched to draw it (`attachment_image.rs` in mail-core). The
//! bytes are read and decoded on a blocking thread, from a resource: nothing is spawned from
//! the render, and what was drawn is remembered, so opening the conversation again draws
//! at once.
//!
//! A picture mailo decodes and a PDF's first page are drawn as they always were. With the
//! `anyview` feature, a part of any other kind that anyview can show as a picture (a bitmap, an
//! SVG, a recording's cover) is drawn by anyview's pane, media only; one it has only facts for
//! keeps the paperclip (`peek/previewer.rs` says who draws which).

use std::sync::Mutex;

use dioxus::prelude::*;
use ds::components::content::pdf_thumb::PdfPage;
use ds::prelude::*;
use ds::style::icon::render::Glyph;
use mail_client::Client;
use mail_domain::MessageId;

use crate::ui::view::Shell;
use mail_client::attachment_image::{self, Kind, Picture, Unavailable};
use mail_ui_model::copy;

/// The thumbnail's room, in logical pixels.
const ROOM: f32 = 56.0;

/// How many thumbnails to remember. One is a PNG of at most 112 × 112.
const KEPT: usize = 64;

/// How many of anyview's to remember: each holds its decoded pixels (up to a megapixel), not a
/// small PNG, so fewer.
#[cfg(feature = "anyview")]
const KEPT_PEEKED: usize = 8;

/// What the strip draws for one part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Shown {
    /// Nothing to preview: the paperclip.
    Nothing,
    Image(Picture),
    Pdf(PdfPage),
    /// A picture anyview made of a part mailo does not decode.
    #[cfg(feature = "anyview")]
    Peeked(std::sync::Arc<anyview_peek::AnyPeeked>),
    /// A picture here that is not drawn, and why.
    Refused(String),
}

type Known = Vec<((MessageId, usize), Shown)>;

static CACHE: Mutex<Known> = Mutex::new(Vec::new());

fn cached(key: (MessageId, usize)) -> Option<Shown> {
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, shown)| shown.clone())
}

fn remember(key: (MessageId, usize), shown: &Shown) {
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(k, _)| *k != key);
    cache.push((key, shown.clone()));
    if cache.len() > KEPT {
        cache.remove(0);
    }
    #[cfg(feature = "anyview")]
    {
        let peeked = |entry: &((MessageId, usize), Shown)| matches!(entry.1, Shown::Peeked(_));
        while cache.iter().filter(|entry| peeked(entry)).count() > KEPT_PEEKED {
            // The oldest of them: the cache is oldest first.
            if let Some(oldest) = cache.iter().position(peeked) {
                cache.remove(oldest);
            }
        }
    }
}

/// A part mailo does not decode, as anyview shows it: a picture, or nothing (the paperclip).
#[cfg(feature = "anyview")]
fn beyond_mailo(client: &Client, message: MessageId, index: usize) -> Shown {
    use super::anyview::{Seen, see};
    match see(client, message, index) {
        Seen::Ready(peeked) if peeked.still().is_some() => Shown::Peeked(peeked),
        Seen::Ready(_) | Seen::NotHere | Seen::Own | Seen::Failed(_) => Shown::Nothing,
    }
}

/// Without anyview, a part mailo does not decode has no thumbnail.
#[cfg(not(feature = "anyview"))]
fn beyond_mailo(_store: &Client, _message: MessageId, _index: usize) -> Shown {
    Shown::Nothing
}

/// Attachment `index` of `message` as the strip draws it. Blocking: reads and decodes.
pub(super) fn thumbnail(client: &Client, message: MessageId, index: usize) -> Shown {
    if let Some(had) = cached((message, index)) {
        return had;
    }
    let shown = match client.preview().load(message, index) {
        Ok((Kind::Pdf, bytes)) => Shown::Pdf(ds_blitz::pdf_thumb_bytes(
            bytes,
            ds_blitz::DeviceBox {
                width: attachment_image::SMALL.width,
                height: attachment_image::SMALL.height,
            },
        )),
        Ok((kind, bytes)) => match attachment_image::picture(&bytes, kind, attachment_image::SMALL)
        {
            Ok(picture) => Shown::Image(picture),
            Err(refusal) => Shown::Refused(refusal.sentence()),
        },
        Err(Unavailable::Refused(refusal)) => Shown::Refused(refusal.sentence()),
        // Not remembered: a part downloaded later is drawn when the strip is drawn again.
        Err(Unavailable::NotHere | Unavailable::Store(_)) => return Shown::Nothing,
        Err(Unavailable::NotAPicture) => beyond_mailo(client, message, index),
    };
    remember((message, index), &shown);
    shown
}

/// Space or Return, alone, on a cell that has the keyboard opens it, as a click does; the key goes
/// no further, so the reader behind does not scroll. A chord (Primary+Return, which opens the part
/// in its app once Peek is up) is the window's.
pub(super) fn pressed(event: &KeyboardEvent) -> bool {
    event.modifiers().is_empty() && matches!(event.key().to_string().as_str(), " " | "Enter")
}

/// The leading cell of a stored part's row: its thumbnail, which opens the viewer, or the
/// paperclip, with a note when a picture is refused.
#[component]
pub(super) fn Thumb(
    message: MessageId,
    index: usize,
    name: String,
    shell: Signal<Shell>,
) -> Element {
    let shown = use_resource(move || {
        let client = crate::ui::live::client();
        async move {
            tokio::task::spawn_blocking(move || thumbnail(&client, message, index))
                .await
                .unwrap_or(Shown::Nothing)
        }
    });
    let shown = shown.read().clone().unwrap_or(Shown::Nothing);
    let room = Size {
        width: Px(ROOM),
        height: Px(ROOM),
    };
    // The paperclip stands in a thumbnail's room, so every name in the list starts in line.
    let (class, face) = match &shown {
        Shown::Nothing | Shown::Refused(_) => (
            "att-mark",
            rsx! { Glyph { icon: Icon::Paperclip, size: IconSize::Compact } },
        ),
        Shown::Image(picture) => (
            "att-thumb",
            rsx! { img { alt: "{name}", src: "{picture.uri}" } },
        ),
        Shown::Pdf(page) => (
            "att-thumb",
            rsx! { PdfThumb { page: page.clone(), size: room, label: Some(name.clone()) } },
        ),
        #[cfg(feature = "anyview")]
        Shown::Peeked(peeked) => (
            "att-thumb",
            rsx! {
                anyview_peek::Card {
                    peeked: peeked.clone(),
                    parts: anyview_peek::Parts::of([anyview_peek::Part::Media]),
                }
            },
        ),
    };
    rsx! {
        div {
            class,
            role: "button",
            tabindex: "0",
            "aria-label": copy::aria::preview(&name),
            onclick: move |_| super::peek::open(shell, message, index),
            onkeydown: move |event: KeyboardEvent| {
                if pressed(&event) {
                    event.prevent_default();
                    event.stop_propagation();
                    super::peek::open(shell, message, index);
                }
            },
            {face}
        }
        if let Shown::Refused(why) = shown {
            span { class: "att-note", "{why}" }
        }
    }
}

//! A stored attachment's thumbnail in the reader's strip, and the press that opens the viewer.
//!
//! Drawn only for a part whose bytes are here (`Kept::Here`): a part still on the server shows
//! its paperclip until it is downloaded, and nothing is fetched to draw it (`preview.rs`). The
//! bytes are read and decoded on a blocking thread, from a resource: nothing is spawned from
//! the render (F140), and what was drawn is remembered, so opening the conversation again draws
//! at once.

use std::sync::{Arc, Mutex};

use dioxus::prelude::*;
use ds::components::content::pdf_thumb::PdfPage;
use ds::prelude::*;
use ds::style::icon::render::Glyph;
use mail_domain::MessageId;
use mail_store::SqliteStore;

use crate::ui::view::{Shell, Viewing};
use mail_core::preview::{self, Kind, Picture, Unshown};

/// The thumbnail's room, in logical pixels.
const ROOM: f32 = 56.0;

/// How many thumbnails to remember. One is a PNG of at most 112 × 112.
const KEPT: usize = 64;

/// What the strip draws for one part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Shown {
    /// Nothing to preview: the paperclip.
    Nothing,
    Image(Picture),
    Pdf(PdfPage),
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
}

/// Attachment `index` of `message` as the strip draws it. Blocking: reads and decodes.
pub(super) fn thumbnail(store: &SqliteStore, message: MessageId, index: usize) -> Shown {
    if let Some(had) = cached((message, index)) {
        return had;
    }
    let shown = match preview::load(store, message, index) {
        Ok((Kind::Pdf, bytes)) => Shown::Pdf(ds_blitz::pdf_thumb_bytes(
            bytes,
            ds_blitz::DeviceBox {
                width: preview::THUMB.width,
                height: preview::THUMB.height,
            },
        )),
        Ok((kind, bytes)) => match preview::picture(&bytes, kind, preview::THUMB) {
            Ok(picture) => Shown::Image(picture),
            Err(refusal) => Shown::Refused(refusal.sentence()),
        },
        Err(Unshown::Refused(refusal)) => Shown::Refused(refusal.sentence()),
        // Not remembered: a part downloaded later is drawn when the strip is drawn again.
        Err(Unshown::NotHere | Unshown::Store(_)) => return Shown::Nothing,
        Err(Unshown::NotAPicture) => Shown::Nothing,
    };
    remember((message, index), &shown);
    shown
}

/// Open the viewer on attachment `index` of `message`, and give the window the keyboard, so Esc
/// and the arrows reach it.
fn open(mut shell: Signal<Shell>, message: MessageId, index: usize) {
    shell.write().viewing = Some(Viewing::of(message, index));
    crate::ui::host::Host::focus_app();
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
        let store = consume_context::<Arc<SqliteStore>>();
        async move {
            tokio::task::spawn_blocking(move || thumbnail(&store, message, index))
                .await
                .unwrap_or(Shown::Nothing)
        }
    });
    let shown = shown.read().clone().unwrap_or(Shown::Nothing);
    let room = Size {
        width: Px(ROOM),
        height: Px(ROOM),
    };
    match shown {
        // The paperclip stands in a thumbnail's room, so every name in the list starts in line.
        Shown::Nothing => rsx! {
            div { class: "att-mark",
                Glyph { icon: Icon::Paperclip, size: IconSize::Compact }
            }
        },
        Shown::Refused(why) => rsx! {
            div { class: "att-mark",
                Glyph { icon: Icon::Paperclip, size: IconSize::Compact }
            }
            span { class: "att-note", "{why}" }
        },
        Shown::Image(picture) => rsx! {
            div {
                class: "att-thumb",
                role: "button",
                "aria-label": "Preview {name}",
                onclick: move |_| open(shell, message, index),
                img { alt: "{name}", src: "{picture.uri}" }
            }
        },
        Shown::Pdf(page) => rsx! {
            div {
                class: "att-thumb",
                role: "button",
                "aria-label": "Preview {name}",
                onclick: move |_| open(shell, message, index),
                PdfThumb { page, size: room, label: Some(name.clone()) }
            }
        },
    }
}

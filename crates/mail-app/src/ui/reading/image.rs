//! An image block: the picture, or a placeholder that holds its room while it is blocked.
//!
//! Moved out of `blocks.rs` unchanged when that file grew the marks (`CONVENTIONS.md` §8).

use super::super::press::on_primary;
use crate::view::Shell;
use dioxus::prelude::*;
use mail_mime::ImgSrc;

pub(super) fn image(
    path: &str,
    src: &ImgSrc,
    alt: &str,
    width: Option<u32>,
    height: Option<u32>,
    mut shell: Signal<Shell>,
) -> Element {
    // A one-pixel image is a tracking pixel or a layout spacer. Drawing it as a
    // placeholder would name its host in the page for a picture nobody can see.
    if spacer(width, height) {
        return rsx! { "" };
    }
    match src {
        ImgSrc::Blocked { host } => rsx! {
            div { key: "{path}", class: "b b-img blocked", style: "{placeholder_ratio(width, height)}",
                span { "Image from " strong { "{host}" } " — " }
                ds::Button {
                    variant: ds::ButtonVariant::Mini,
                    label: "load images".to_owned(),
                    aria_label: "load images".to_owned(),
                    onclick: on_primary(move || shell.write().show_remote_images = true),
                }
            }
        },
        ImgSrc::Inline(uri) => rsx! {
            div { key: "{path}", class: "b b-img",
                img { alt: "{alt}", src: "{uri.as_str()}" }
            }
        },
        // The window does not fetch a consented image as it draws it: mailo fetched it, and it is drawn from what came back (`remote.rs`).
        // The web address itself is never put in the document.
        ImgSrc::Remote(url) => fetched(path, url.as_str(), alt, width, height),
    }
}

/// A consented remote image on Blitz: its `data:` URI once it has landed, and until then a
/// placeholder that holds its room and says where it stands. Not asked for yet is the same as
/// loading: the reader's fetcher asks for every image it draws, right after the render.
fn fetched(path: &str, url: &str, alt: &str, width: Option<u32>, height: Option<u32>) -> Element {
    use super::remote::{Picture, Pictures};
    let picture = try_consume_context::<Pictures>().and_then(|pictures| pictures.picture(url));
    let host = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_default();
    let ratio = placeholder_ratio(width, height);
    match picture {
        Some(Picture::Shown(uri)) => rsx! {
            div { key: "{path}", class: "b b-img",
                img { alt: "{alt}", src: "{uri}" }
            }
        },
        Some(Picture::Refused) => rsx! {
            div { key: "{path}", class: "b b-img blocked", style: "{ratio}",
                span { "The image from " strong { "{host}" } " could not be shown" }
            }
        },
        Some(Picture::Loading) | None => rsx! {
            div { key: "{path}", class: "b b-img blocked", style: "{ratio}",
                span { "Loading the image from " strong { "{host}" } }
            }
        },
    }
}

/// The declared shape of a blocked image, so the placeholder holds the room the
/// picture will take. Numbers only: the sender's own style never reaches us. A
/// ratio past 8:1 either way is not drawn to shape, so a hostile `height` cannot
/// make the placeholder a page tall.
fn placeholder_ratio(width: Option<u32>, height: Option<u32>) -> String {
    match (width, height) {
        (Some(w), Some(h))
            if w > 0 && h > 0 && w <= h.saturating_mul(8) && h <= w.saturating_mul(8) =>
        {
            format!("aspect-ratio: {w} / {h}")
        }
        _ => String::new(),
    }
}

fn spacer(width: Option<u32>, height: Option<u32>) -> bool {
    match (width, height) {
        (Some(w), Some(h)) => w <= 1 || h <= 1,
        (Some(w), None) => w <= 1,
        (None, Some(h)) => h <= 1,
        (None, None) => false,
    }
}

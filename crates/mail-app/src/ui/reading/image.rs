//! An image block: the picture, or a placeholder that holds its room while it is blocked.
//!
//! Moved out of `blocks.rs` unchanged when that file grew the marks (`CONVENTIONS.md` §8).

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
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "load images",
                    onclick: move |_| shell.write().show_remote_images = true,
                    "load images"
                }
            }
        },
        ImgSrc::Inline(uri) => rsx! {
            div { key: "{path}", class: "b b-img",
                img { alt: "{alt}", src: "{uri.as_str()}" }
            }
        },
        ImgSrc::Remote(url) => rsx! {
            div { key: "{path}", class: "b b-img",
                img { alt: "{alt}", src: "{url.as_str()}" }
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

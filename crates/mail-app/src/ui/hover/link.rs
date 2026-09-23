//! The link pill: where a link in the reader really goes, read from the parsed blocks.

use super::super::icon::{Glyph, Icon};
use super::hover;
use crate::trust::Destination;
use dioxus::prelude::*;

/// Where the link under the pointer goes, like a browser's status bar, and loud when its text
/// names somewhere else. Drawn in the reader; the link reports itself from the parsed blocks.
#[component]
pub(in crate::ui) fn LinkPill() -> Element {
    let Some(state) = hover() else {
        return rsx! {};
    };
    let Some(link) = state.link.read().clone() else {
        return rsx! {};
    };
    match link {
        Destination::Web {
            scheme,
            sub,
            registered,
            path,
        } => rsx! {
            div { class: "linkpill", role: "status",
                span { class: "dim", "{scheme}{sub}" }
                b { "{registered}" }
                span { class: "dim", "{path}" }
            }
        },
        Destination::Lies { goes_to, claims } => rsx! {
            div { class: "linkpill warn", role: "status",
                Glyph { icon: Icon::X, class: None }
                span {
                    "Goes to "
                    b { "{goes_to}" }
                    ", not {claims}"
                }
            }
        },
        Destination::Other(href) => rsx! {
            div { class: "linkpill", role: "status", span { class: "dim", "{href}" } }
        },
    }
}

/// A link in the reader was entered or left. Called from the parsed blocks.
pub(in crate::ui) fn link_over(text: &str, href: &str) {
    if let Some(mut state) = hover() {
        state.link.set(Some(crate::trust::destination(text, href)));
    }
}

pub(in crate::ui) fn link_out() {
    if let Some(mut state) = hover() {
        state.link.set(None);
    }
}

//! Inline spans inside a block.
//!
//! Links carry `target` and `rel` here. The block types do not store them, and the
//! sanitizer no longer gets to set them once the output is blocks.

use dioxus::prelude::*;
use mail_mime::{LINK_REL, LINK_TARGET, Span};

pub(super) fn spans(items: &[Span]) -> Element {
    rsx! {
        for (index, span) in items.iter().enumerate() {
            {one(span, index)}
        }
    }
}

fn one(span: &Span, index: usize) -> Element {
    match span {
        Span::Text(text) => rsx! { span { key: "{index}", "{text}" } },
        Span::Strong(inner) => rsx! { strong { key: "{index}", {spans(inner)} } },
        Span::Emphasis(inner) => rsx! { em { key: "{index}", {spans(inner)} } },
        Span::Code(text) => rsx! { code { key: "{index}", "{text}" } },
        Span::Break => rsx! { br { key: "{index}" } },
        Span::Link { url, spans: inner } => rsx! {
            a {
                key: "{index}",
                href: "{url.as_str()}",
                target: "{LINK_TARGET}",
                rel: "{LINK_REL}",
                {spans(inner)}
            }
        },
    }
}

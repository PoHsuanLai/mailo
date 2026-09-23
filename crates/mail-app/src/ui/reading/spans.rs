//! Inline spans inside a block.
//!
//! Links carry `target` and `rel` here. The block types do not store them, and the
//! sanitizer no longer gets to set them once the output is blocks.
//!
//! `key` is the block's path. Span `i` is the leaf `{key}/{i}`, which is how [`Found`] names
//! it; nested spans append their own index the same way.

use super::super::marked::marked;
use super::found::Found;
use dioxus::prelude::*;
use mail_mime::{LINK_REL, LINK_TARGET, Span};

pub(super) fn spans(items: &[Span], key: &str, found: &Found) -> Element {
    rsx! {
        for (index, span) in items.iter().enumerate() {
            {one(span, index, &format!("{key}/{index}"), found)}
        }
    }
}

fn one(span: &Span, index: usize, leaf: &str, found: &Found) -> Element {
    match span {
        Span::Text(text) => {
            let (marks, numbering) = found.at(leaf);
            rsx! { span { key: "{index}", {marked(text, marks, numbering)} } }
        }
        Span::Strong(inner) => rsx! { strong { key: "{index}", {spans(inner, leaf, found)} } },
        Span::Emphasis(inner) => rsx! { em { key: "{index}", {spans(inner, leaf, found)} } },
        Span::Code(text) => {
            let (marks, numbering) = found.at(leaf);
            rsx! { code { key: "{index}", {marked(text, marks, numbering)} } }
        }
        Span::Break => rsx! { br { key: "{index}" } },
        Span::Link { url, spans: inner } => rsx! {
            a {
                key: "{index}",
                href: "{url.as_str()}",
                target: "{LINK_TARGET}",
                rel: "{LINK_REL}",
                {spans(inner, leaf, found)}
            }
        },
    }
}

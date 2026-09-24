//! One message body, drawn as blocks.
//!
//! Nothing here is the sender's markup. The Original frame, when the body is
//! laid out, is a separate sandboxed iframe that stays mounted: showing and
//! hiding it is a class, because mounting it again would reload the document.
//!
//! Marks from a search or a find are drawn into the blocks' text, looked up by
//! each leaf's key in [`Found`]. The frame takes no part in that: its props are
//! the markup and a class, and a find changes neither.

use super::super::marked::marked;
use super::found::{Found, PRIMARY};
use super::image::image;
use super::spans::spans;
use super::table::table;
use crate::view::{Reading, Shell};
use dioxus::prelude::*;
use ds::{Glyph, Icon};
use mail_domain::MessageId;
use mail_mime::{Block, Dir, Document, LINK_REL, LINK_TARGET, Reached, SafeUrl, Shape, Span};
use std::collections::HashMap;

#[cfg(test)]
thread_local! {
    static IFRAME_MOUNTS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// How many times the sandboxed frame has mounted on this thread.
#[cfg(test)]
pub(super) fn iframe_mounts() -> u32 {
    IFRAME_MOUNTS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn reset_iframe_mounts() {
    IFRAME_MOUNTS.with(|mounts| mounts.set(0));
}

pub(super) type OpenQuotes = HashMap<String, bool>;

/// What every block of one message is drawn with.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    message_id: MessageId,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
    found: &'a Found,
}

/// The sandboxed original. Always mounted when the body has HTML; `concealed`
/// only changes a class.
#[component]
fn Sandbox(html: String, concealed: bool) -> Element {
    #[cfg(test)]
    use_hook(|| {
        IFRAME_MOUNTS.with(|mounts| mounts.set(mounts.get().saturating_add(1)));
    });
    let class = if concealed { "html is-hidden" } else { "html" };
    rsx! {
        iframe {
            class: "{class}",
            "sandbox": "",
            srcdoc: "{html}",
            title: "The message as the sender laid it out",
        }
    }
}

/// Blocks, and the Original frame when the body has HTML.
#[component]
pub(super) fn MessageView(
    message_id: MessageId,
    reading: Reading,
    original: Signal<HashMap<MessageId, bool>>,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
    /// Marks to draw. The default marks nothing.
    #[props(default)]
    found: Found,
) -> Element {
    let frame = reading.frame_html().map(str::to_owned);
    let show_original = frame.is_some() && original.read().get(&message_id) == Some(&true);
    let document = reading.document().cloned();
    let cx = Ctx {
        message_id,
        quotes,
        shell,
        found: &found,
    };
    rsx! {
        if let Some(html) = frame {
            Sandbox { html, concealed: !show_original }
        }
        if let Some(document) = document {
            div {
                class: if show_original { "blocks is-hidden" } else { "blocks" },
                {body(&document, cx)}
                if document.reached != Reached::Nothing {
                    p { class: "b b-note", "This message was shortened to display it." }
                }
            }
        }
    }
}

fn body(document: &Document, cx: Ctx) -> Element {
    let machine = document.shape == Shape::Machine;
    rsx! {
        if machine {
            div { class: "b b-receipt",
                if let Some(action) = &document.primary {
                    div { class: "top",
                        {action_link(&action.label, &action.url, PRIMARY, cx.found)}
                    }
                }
                {render_blocks(&document.blocks, "0", false, cx)}
                p { class: "b-note", "reshaped: the action first, the rest as facts" }
            }
        } else {
            {render_blocks(&document.blocks, "0", false, cx)}
        }
        if document.shape == Shape::Layout {
            p { class: "b-note", "laid-out mail: shown as blocks; Original is the sandboxed frame" }
        }
    }
}

fn render_blocks(blocks: &[Block], path: &str, in_quote: bool, cx: Ctx) -> Element {
    rsx! {
        for (index, block) in blocks.iter().enumerate() {
            {one_block(block, &format!("{path}.{index}"), in_quote, cx)}
        }
    }
}

fn one_block(block: &Block, path: &str, in_quote: bool, cx: Ctx) -> Element {
    let found = cx.found;
    match block {
        Block::Heading {
            level,
            spans: inner,
        } => heading(*level, inner, path, found),
        Block::Paragraph { spans: inner, dir } => rsx! {
            p { key: "{path}", class: "b b-p", dir: "{dir_name(*dir)}", {spans(inner, path, found)} }
        },
        Block::List { ordered, items } => list(path, *ordered, items, in_quote, cx),
        Block::Quote {
            attribution,
            blocks,
        } => quote_block(path, in_quote, attribution, blocks, cx),
        Block::Code { lang, text } => {
            let (marks, numbering) = found.at(&format!("{path}/code"));
            rsx! {
                pre { key: "{path}", class: "b b-code",
                    if let Some(lang) = lang {
                        span { class: "lang", "{lang}" }
                    }
                    {marked(text, marks, numbering)}
                }
            }
        }
        Block::Table { head, rows } => table(path, head, rows, found),
        Block::Facts(pairs) => rsx! {
            dl { key: "{path}", class: "b b-kv",
                for (index, (label, value)) in pairs.iter().enumerate() {
                    dt { key: "{index}", {spans(label, &format!("{path}/k{index}"), found)} }
                    dd { {spans(value, &format!("{path}/v{index}"), found)} }
                }
            }
        },
        Block::Image {
            src,
            alt,
            width,
            height,
        } => image(path, src, alt, *width, *height, cx.shell),
        Block::Button { label, url } => rsx! {
            div { key: "{path}", class: "b", {action_link(label, url, &format!("{path}/btn"), found)} }
        },
        Block::Signature(blocks) => rsx! {
            div { key: "{path}", class: "b b-sig",
                {render_blocks(blocks, &format!("{path}.s"), in_quote, cx)}
            }
        },
        Block::Rule => rsx! { hr { key: "{path}", class: "b b-rule" } },
    }
}

fn heading(level: u8, inner: &[Span], path: &str, found: &Found) -> Element {
    // The reader's subject is already an h2. Message headings sit under it,
    // the way the parsed-body mockup draws h1 as h3.
    let class = format!("b b-h{level}");
    match level {
        1 => rsx! { h3 { key: "{path}", class: "{class}", {spans(inner, path, found)} } },
        2 => rsx! { h4 { key: "{path}", class: "{class}", {spans(inner, path, found)} } },
        3 => rsx! { h5 { key: "{path}", class: "{class}", {spans(inner, path, found)} } },
        _ => rsx! { h6 { key: "{path}", class: "{class}", {spans(inner, path, found)} } },
    }
}

fn list(path: &str, ordered: bool, items: &[Vec<Block>], in_quote: bool, cx: Ctx) -> Element {
    if ordered {
        rsx! {
            ol { key: "{path}", class: "b b-list", {list_items(path, items, in_quote, cx)} }
        }
    } else {
        rsx! {
            ul { key: "{path}", class: "b b-list", {list_items(path, items, in_quote, cx)} }
        }
    }
}

fn list_items(path: &str, items: &[Vec<Block>], in_quote: bool, cx: Ctx) -> Element {
    rsx! {
        for (index, item) in items.iter().enumerate() {
            li { key: "{index}",
                {render_blocks(item, &format!("{path}.{index}"), in_quote, cx)}
            }
        }
    }
}

/// A quote inside another quote is earlier in the chain, and starts folded — unless a search
/// or a find has a match inside it, which opens it so that the match can be seen.
fn quote_block(
    path: &str,
    in_quote: bool,
    attribution: &Option<Vec<Span>>,
    blocks: &[Block],
    cx: Ctx,
) -> Element {
    let Ctx {
        message_id,
        mut quotes,
        found,
        ..
    } = cx;
    let key = format!("{message_id}/{path}");
    let inner = format!("{path}.q");
    let folded =
        in_quote && !quotes.read().get(&key).copied().unwrap_or(false) && !found.inside(&inner);
    let count = 1 + quote_nodes(blocks);
    let label = if count == 1 {
        "1 earlier message".to_owned()
    } else {
        format!("{count} earlier messages")
    };
    rsx! {
        div { key: "{path}", class: "b b-quote",
            if let Some(who) = attribution {
                div { class: "who",
                    Glyph { icon: Icon::Corner }
                    {spans(who, &format!("{path}/who"), found)}
                }
            }
            if folded {
                button {
                    class: "fold",
                    r#type: "button",
                    aria_expanded: "false",
                    onclick: move |_| {
                        quotes.write().insert(key.clone(), true);
                    },
                    "{label}"
                }
            } else {
                div { class: "inner",
                    {render_blocks(blocks, &inner, true, cx)}
                }
            }
        }
    }
}

fn quote_nodes(blocks: &[Block]) -> usize {
    blocks.iter().map(quote_nodes_one).sum()
}

fn quote_nodes_one(block: &Block) -> usize {
    match block {
        Block::Quote { blocks, .. } => 1 + quote_nodes(blocks),
        Block::List { items, .. } => items.iter().map(|item| quote_nodes(item)).sum(),
        Block::Signature(blocks) => quote_nodes(blocks),
        _ => 0,
    }
}

fn action_link(label: &str, url: &SafeUrl, key: &str, found: &Found) -> Element {
    let (marks, numbering) = found.at(key);
    let text = label.to_owned();
    let href = url.as_str().to_owned();
    rsx! {
        a {
            class: "b-cta",
            href: "{url.as_str()}",
            target: "{LINK_TARGET}",
            rel: "{LINK_REL}",
            onpointerenter: move |_| super::super::hover::link_over(&text, &href),
            onpointerleave: move |_| super::super::hover::link_out(),
            {marked(label, marks, numbering)}
        }
    }
}

fn dir_name(dir: Dir) -> &'static str {
    match dir {
        Dir::Auto => "auto",
        Dir::Ltr => "ltr",
        Dir::Rtl => "rtl",
    }
}

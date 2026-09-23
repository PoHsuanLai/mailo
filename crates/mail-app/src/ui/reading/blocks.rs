//! One message body, drawn as blocks.
//!
//! Nothing here is the sender's markup. The Original frame, when the body is
//! laid out, is a separate sandboxed iframe that stays mounted: showing and
//! hiding it is a class, because mounting it again would reload the document.

use super::super::icon::{Glyph, Icon};
use super::spans::spans;
use super::table::table;
use crate::view::{Reading, Shell};
use dioxus::prelude::*;
use mail_domain::MessageId;
use mail_mime::{
    Block, Dir, Document, ImgSrc, LINK_REL, LINK_TARGET, Reached, SafeUrl, Shape, Span,
};
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
) -> Element {
    let frame = reading.frame_html().map(str::to_owned);
    let show_original = frame.is_some() && original.read().get(&message_id) == Some(&true);
    let document = reading.document().cloned();
    rsx! {
        if let Some(html) = frame {
            Sandbox { html, concealed: !show_original }
        }
        if let Some(document) = document {
            div {
                class: if show_original { "blocks is-hidden" } else { "blocks" },
                {body(message_id, &document, quotes, shell)}
                if document.reached != Reached::Nothing {
                    p { class: "b b-note", "This message was shortened to display it." }
                }
            }
        }
    }
}

fn body(
    message_id: MessageId,
    document: &Document,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
) -> Element {
    let machine = document.shape == Shape::Machine;
    rsx! {
        if machine {
            div { class: "b b-receipt",
                if let Some(action) = &document.primary {
                    div { class: "top",
                        {action_link(&action.label, &action.url)}
                    }
                }
                {render_blocks(&document.blocks, message_id, "0", false, quotes, shell)}
                p { class: "b-note", "reshaped: the action first, the rest as facts" }
            }
        } else {
            {render_blocks(&document.blocks, message_id, "0", false, quotes, shell)}
        }
        if document.shape == Shape::Layout {
            p { class: "b-note", "laid-out mail: shown as blocks; Original is the sandboxed frame" }
        }
    }
}

fn render_blocks(
    blocks: &[Block],
    message_id: MessageId,
    path: &str,
    in_quote: bool,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
) -> Element {
    rsx! {
        for (index, block) in blocks.iter().enumerate() {
            {one_block(
                block,
                message_id,
                &format!("{path}.{index}"),
                in_quote,
                quotes,
                shell,
            )}
        }
    }
}

fn one_block(
    block: &Block,
    message_id: MessageId,
    path: &str,
    in_quote: bool,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
) -> Element {
    match block {
        Block::Heading {
            level,
            spans: inner,
        } => heading(*level, inner, path),
        Block::Paragraph { spans: inner, dir } => rsx! {
            p { key: "{path}", class: "b b-p", dir: "{dir_name(*dir)}", {spans(inner)} }
        },
        Block::List { ordered, items } => {
            list(path, *ordered, items, message_id, in_quote, quotes, shell)
        }
        Block::Quote {
            attribution,
            blocks,
        } => quote_block(
            path,
            message_id,
            in_quote,
            attribution,
            blocks,
            quotes,
            shell,
        ),
        Block::Code { lang, text } => rsx! {
            pre { key: "{path}", class: "b b-code",
                if let Some(lang) = lang {
                    span { class: "lang", "{lang}" }
                }
                "{text}"
            }
        },
        Block::Table { head, rows } => table(path, head, rows),
        Block::Facts(pairs) => rsx! {
            dl { key: "{path}", class: "b b-kv",
                for (index, (label, value)) in pairs.iter().enumerate() {
                    dt { key: "{index}", {spans(label)} }
                    dd { {spans(value)} }
                }
            }
        },
        Block::Image {
            src,
            alt,
            width,
            height,
        } => image(path, src, alt, *width, *height, shell),
        Block::Button { label, url } => rsx! {
            div { key: "{path}", class: "b", {action_link(label, url)} }
        },
        Block::Signature(blocks) => rsx! {
            div { key: "{path}", class: "b b-sig",
                {render_blocks(blocks, message_id, &format!("{path}.s"), in_quote, quotes, shell)}
            }
        },
        Block::Rule => rsx! { hr { key: "{path}", class: "b b-rule" } },
    }
}

fn heading(level: u8, inner: &[Span], path: &str) -> Element {
    // The reader's subject is already an h2. Message headings sit under it,
    // the way the parsed-body mockup draws h1 as h3.
    let class = format!("b b-h{level}");
    match level {
        1 => rsx! { h3 { key: "{path}", class: "{class}", {spans(inner)} } },
        2 => rsx! { h4 { key: "{path}", class: "{class}", {spans(inner)} } },
        3 => rsx! { h5 { key: "{path}", class: "{class}", {spans(inner)} } },
        _ => rsx! { h6 { key: "{path}", class: "{class}", {spans(inner)} } },
    }
}

fn list(
    path: &str,
    ordered: bool,
    items: &[Vec<Block>],
    message_id: MessageId,
    in_quote: bool,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
) -> Element {
    if ordered {
        rsx! {
            ol { key: "{path}", class: "b b-list",
                {list_items(path, items, message_id, in_quote, quotes, shell)}
            }
        }
    } else {
        rsx! {
            ul { key: "{path}", class: "b b-list",
                {list_items(path, items, message_id, in_quote, quotes, shell)}
            }
        }
    }
}

fn list_items(
    path: &str,
    items: &[Vec<Block>],
    message_id: MessageId,
    in_quote: bool,
    quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
) -> Element {
    rsx! {
        for (index, item) in items.iter().enumerate() {
            li { key: "{index}",
                {render_blocks(item, message_id, &format!("{path}.{index}"), in_quote, quotes, shell)}
            }
        }
    }
}

/// A quote inside another quote is earlier in the chain, and starts folded.
fn quote_block(
    path: &str,
    message_id: MessageId,
    in_quote: bool,
    attribution: &Option<Vec<Span>>,
    blocks: &[Block],
    mut quotes: Signal<OpenQuotes>,
    shell: Signal<Shell>,
) -> Element {
    let key = format!("{message_id}/{path}");
    let folded = in_quote && !quotes.read().get(&key).copied().unwrap_or(false);
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
                    Glyph { icon: Icon::Corner, class: None }
                    {spans(who)}
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
                    {render_blocks(blocks, message_id, &format!("{path}.q"), true, quotes, shell)}
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

fn image(
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

fn action_link(label: &str, url: &SafeUrl) -> Element {
    rsx! {
        a {
            class: "b-cta",
            href: "{url.as_str()}",
            target: "{LINK_TARGET}",
            rel: "{LINK_REL}",
            "{label}"
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

//! Peek, the one preview: an attachment, an inline picture, a forwarded message or a link's
//! destination, in quire's `Peek` over the window (docs/design-language.md §0.5).
//!
//! What it shows is held in `Shell::preview`, so the window's keyboard gives it its keys before
//! anything else: Esc closes it, the left and right arrows move to the message's previous or next
//! stored attachment, the up and down arrows (and Page Up and Down) turn a PDF's pages, and
//! Primary+Return opens the item in its own app, as Open and Open Link do. Opening another
//! conversation or closing the reader closes it.
//!
//! The strip opens a part through [`open`], a click on an inline image in a message's frame the
//! part it shows through [`open_inline`], and the link menu a link's card through [`open_link`];
//! nothing else writes `Shell::preview`.

mod link;
mod own;
mod part;
mod previewer;
pub(in crate::ui) use part::sweep_left_open;
// anyview asks which parts mailo draws itself.
#[cfg(feature = "anyview")]
pub(super) use previewer::{HEAD, mailo_draws};

use dioxus::prelude::*;
use ds::prelude::Keys;
use mail_client::Client;
use mail_domain::MessageId;

use crate::ui::view::{LinkCard, Preview, Shell, Viewing};

/// Open attachment `index` of `message` in Peek, and give the window the keyboard, so Esc and the
/// arrows reach it.
pub(in crate::ui) fn open(mut shell: Signal<Shell>, message: MessageId, index: usize) {
    shell.write().preview = Some(Preview::Part(Viewing::of(message, index)));
    crate::ui::host::Host::focus_app();
}

/// Open in Peek the part of `message` (as stored) whose `Content-ID` is `cid`, an inline image
/// clicked in its frame; nothing when it has none. `cid` came from that message's own markup and
/// is only ever matched against that message's parts.
pub(in crate::ui) fn open_inline(
    shell: Signal<Shell>,
    client: &Client,
    message: MessageId,
    cid: &str,
) {
    let Ok(stored) = client.stored().message(message) else {
        return;
    };
    if let Some(index) = inline_index(&stored.attachments, cid) {
        open(shell, message, index);
    }
}

/// Where the part whose `Content-ID` is `cid` sits among `attachments`: the first, as the frame
/// embeds the first part a reference names (`mail_mime::embed_inline`).
fn inline_index(attachments: &[mail_domain::Attachment], cid: &str) -> Option<usize> {
    attachments.iter().position(|part| match &part.inline {
        mail_domain::Inline::Embedded { cid: own } => {
            own.trim_start_matches('<').trim_end_matches('>') == cid
        }
        mail_domain::Inline::Attached => false,
    })
}

/// Open a link's destination card in Peek.
pub(in crate::ui) fn open_link(mut shell: Signal<Shell>, card: LinkCard) {
    shell.write().preview = Some(Preview::Link(card));
    crate::ui::host::Host::focus_app();
}

/// Close Peek, and give the keyboard back to the window.
fn close(mut shell: Signal<Shell>) {
    shell.write().preview = None;
    crate::ui::host::Host::focus_app();
}

/// A press on one of Peek's buttons: `act`, then the keyboard back to the window, so Esc and the
/// arrows still reach Peek rather than the button.
fn pressed(act: impl FnOnce()) {
    act();
    crate::ui::host::Host::focus_app();
}

/// Turn a PDF's pages by `by`.
fn turn(mut shell: Signal<Shell>, by: i32) {
    let preview = shell.peek().preview.clone();
    if let Some(Preview::Part(viewing)) = preview {
        let turned = viewing.turned(by);
        if turned != viewing {
            shell.write().preview = Some(Preview::Part(turned));
        }
    }
}

/// Move to the message's stored attachment `by` places on, if there is one that way.
fn step(mut shell: Signal<Shell>, by: i32) {
    let preview = shell.peek().preview.clone();
    let Some(Preview::Part(viewing)) = preview else {
        return;
    };
    let client = crate::ui::live::client();
    if let Some(next) = part::stepped(&client, viewing, by) {
        shell.write().preview = Some(Preview::Part(next));
    }
}

/// A key while Peek is open. It takes every key, so nothing reaches the conversation behind it.
pub(in crate::ui) fn peek_key(shell: Signal<Shell>, keys: Keys, event: &KeyboardEvent) {
    if crate::ui::actions::is_open_in_app(keys, &event.key(), event.modifiers()) {
        let preview = shell.peek().preview.clone();
        match preview {
            Some(Preview::Part(viewing)) => part::open_in_app(viewing),
            Some(Preview::Link(card)) => {
                link::open(&card);
                close(shell);
            }
            None => {}
        }
        return;
    }
    match event.key().to_string().as_str() {
        "Escape" => close(shell),
        "ArrowRight" => step(shell, 1),
        "ArrowLeft" => step(shell, -1),
        "ArrowDown" | "PageDown" => turn(shell, 1),
        "ArrowUp" | "PageUp" => turn(shell, -1),
        _ => {}
    }
}

/// Peek, on what `shell.preview` holds. Mounted while it holds something.
#[component]
pub(in crate::ui) fn PeekView(shell: Signal<Shell>) -> Element {
    match shell.read().preview.clone() {
        Some(Preview::Part(_)) => rsx! { part::PartPeek { shell } },
        Some(Preview::Link(card)) => rsx! { link::LinkPeek { card, shell } },
        None => rsx! {},
    }
}

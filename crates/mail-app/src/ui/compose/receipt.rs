//! Asking for a read receipt from the composer: an item in the Sends menu, and a row that says
//! so while it is on.
//!
//! In the Sends menu because it is a choice about how the message goes out, beside when it goes;
//! a row of its own only while it is on, because most mail never asks and an always-present
//! "Receipt: no" row would be a question put to every message.

use dioxus::prelude::*;
use mail_domain::ReceiptRequest;

use super::super::icon::{Glyph, Icon};
use super::super::menu::{MenuItem, Right, Tile};
use super::page::Page;

/// The Sends menu's key for the receipt item. No `When` shares it.
pub(in crate::ui) const KEY: &str = "receipt";

/// What the item and the row call it.
pub(in crate::ui) const ASK: &str = "Ask for a read receipt";

/// The Sends menu's receipt item, checked while the draft asks.
pub(in crate::ui) fn item(receipt: ReceiptRequest) -> MenuItem {
    MenuItem {
        key: KEY.to_owned(),
        tile: Tile::Icon(Icon::Check),
        name: ASK.to_owned(),
        help: Some("their mail app may ask them first".to_owned()),
        right: Right::Check(receipt == ReceiptRequest::Requested),
        group: Some("Also".to_owned()),
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

/// The row under Sends while the draft asks, with the way to stop asking.
#[component]
pub(in crate::ui) fn ReceiptRow(page: Signal<Page>) -> Element {
    if page.read().receipt != ReceiptRequest::Requested {
        return rsx! {};
    }
    let stop = "Stop asking for a read receipt";
    rsx! {
        div { class: "prop-row", "data-row": "receipt",
            div { class: "k", Glyph { icon: Icon::Check, class: None }, "Receipt" }
            div { class: "v",
                span { class: "pchip receipt-chip",
                    "Asks for a read receipt"
                    button {
                        class: "x",
                        r#type: "button",
                        aria_label: "{stop}",
                        onclick: move |_| page.write().toggle_receipt(),
                        Glyph { icon: Icon::X, class: None }
                    }
                }
            }
        }
    }
}

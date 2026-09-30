//! Asking for a read receipt from the composer: an item in the Sends menu, and a row that says
//! so while it is on.
//!
//! In the Sends menu because it is a choice about how the message goes out, beside when it goes;
//! a row of its own only while it is on, because most mail never asks and an always-present
//! "Receipt: no" row would be a question put to every message.

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::components::fields::field_row::{FieldRow, RowLayout};
use ds::prelude::*;
use mail_domain::ReceiptRequest;

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
        FieldRow {
            label: "Receipt",
            layout: RowLayout::Form,
            common: super::props::row("receipt"),
            Chip { variant: ChipVariant::Neutral, text: "Asks for a read receipt".to_owned() }
            Button {
                bezel: Bezel::Toolbar,
                image: ImagePosition::Only,
                icon: Icon::X,
                label: stop,
                title: stop.to_owned(),
                onclick: super::super::press::on_primary(move || page.write().toggle_receipt()),
            }
        }
    }
}

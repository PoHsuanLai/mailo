//! Asking for a read receipt from the composer: a toggle under Send ▾, and a Receipt row that is
//! drawn only while it is on, whose menu turns it Off.
//!
//! Under Send ▾ because it is a choice about how the message goes out; a row of its own only while
//! it is on, because most mail never asks and an always-present "Receipt: no" row would be a
//! question put to every message.

use super::props::ChoiceRow;
use dioxus::prelude::*;
use ds::prelude::*;
use mail_domain::ReceiptRequest;

use super::super::menu::{MenuItem, Right, Tile};
use super::page::{Float, Page};

/// Send ▾'s key for the receipt item.
pub(in crate::ui) const KEY: &str = "receipt";

/// What Send ▾'s item calls it.
pub(in crate::ui) const ASK: &str = "Ask for Read Receipt";

/// The Receipt row's menu keys.
pub(in crate::ui) const ON_KEY: &str = "receipt:on";
pub(in crate::ui) const OFF_KEY: &str = "receipt:off";

/// The Receipt row's menu: "Requested", checked, and "Off", the way back to the default.
pub(in crate::ui) fn row_items(receipt: ReceiptRequest) -> Vec<MenuItem> {
    let row = |key: &str, name: &str, on: bool| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(Icon::Check),
        name: name.to_owned(),
        help: None,
        right: Right::Check(on),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    };
    vec![
        row(ON_KEY, "Requested", receipt == ReceiptRequest::Requested),
        row(OFF_KEY, "Off", receipt == ReceiptRequest::Unrequested),
    ]
}

/// A pick from the Receipt row's menu.
pub(in crate::ui) fn pick_row(page: &mut Page, key: &str) {
    let want = match key {
        ON_KEY => ReceiptRequest::Requested,
        OFF_KEY => ReceiptRequest::Unrequested,
        _ => page.receipt,
    };
    if page.receipt != want {
        page.toggle_receipt();
    }
    page.float = Float::Closed;
}

/// The Receipt row, only while the draft asks.
#[component]
pub(in crate::ui) fn ReceiptRow(page: Signal<Page>) -> Element {
    let receipt = page.read().receipt;
    if receipt != ReceiptRequest::Requested {
        return rsx! {};
    }
    rsx! {
        ChoiceRow {
            page,
            label: "Receipt",
            row: "receipt",
            value: "Requested".to_owned(),
            float: Float::Receipt,
            items: row_items(receipt),
            on_pick: move |key: String| {
                let mut page = page;
                pick_row(&mut page.write(), &key);
            },
        }
    }
}

//! The provider chip on an account tile or a row.

use super::Loaded;
use crate::provider::Provider;
use dioxus::prelude::*;

/// Where the chip sits, which only changes the class the stylesheet already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipPlace {
    /// The account tile.
    Tile,
    /// The provider name on a row.
    Row,
}

/// The icon the chip should draw, subscribed to the startup load and to a refresh.
pub(crate) fn current() -> Loaded {
    let live = try_consume_context::<Signal<Loaded>>();
    let once = try_consume_context::<Loaded>();
    if let Some(icons) = live {
        icons.read().clone()
    } else {
        once.unwrap_or_default()
    }
}

/// The mark on a tile or a row: the cached icon, or the letter.
#[component]
pub(crate) fn ProvChip(provider: Provider, marks: crate::view::Marks, place: ChipPlace) -> Element {
    let loaded = current();
    let uri = match marks {
        crate::view::Marks::Icons => loaded.uri(provider),
        crate::view::Marks::Letters => None,
    };
    let title = provider.title();
    let mark = provider.mark();
    let tint = provider.color().map(|color| format!("--pc:{color}"));
    let (icon_class, letter_class) = match place {
        ChipPlace::Tile => ("prov img on-tile", "prov on-tile"),
        ChipPlace::Row => ("prov img in-row", "prov in-row"),
    };
    rsx! {
        if let Some(uri) = uri {
            span { class: "{icon_class}", title: "{title}",
                img { src: "{uri}", alt: "" }
            }
        } else {
            span { class: "{letter_class}", title: "{title}", style: tint, "{mark}" }
        }
    }
}

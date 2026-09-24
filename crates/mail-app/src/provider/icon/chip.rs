//! The provider mark on a row or in a field: quire's `ProviderMark`, given the cached icon
//! when the setting says icons.

use super::Loaded;
use crate::provider::Provider;
use dioxus::prelude::*;
use ds::{ImageSource, MarkSize, MarkStyle, ProviderMark};

/// Where the chip sits, which is the mark's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChipPlace {
    /// The provider name on a row.
    Row,
    /// Inline in a field: the composer's From.
    Inline,
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

/// quire's name for `provider`: the two tables are the same six, letter for letter.
pub(crate) fn mark_of(provider: Provider) -> ds::Provider {
    match provider {
        Provider::Google => ds::Provider::Google,
        Provider::Microsoft => ds::Provider::Microsoft,
        Provider::Fastmail => ds::Provider::Fastmail,
        Provider::Icloud => ds::Provider::ICloud,
        Provider::Yahoo => ds::Provider::Yahoo,
        Provider::Imap => ds::Provider::Imap,
    }
}

/// How `provider` is drawn under the provider-marks setting: the cached icon when the setting
/// says icons and one is held, else the letter. Reads the startup load, so call it in a render.
pub(crate) fn mark_style(provider: Provider, marks: crate::view::Marks) -> MarkStyle {
    let uri = match marks {
        crate::view::Marks::Icons => current().uri(provider),
        crate::view::Marks::Letters => None,
    };
    uri.map_or(MarkStyle::Letter, |uri| MarkStyle::Image(ImageSource(uri)))
}

/// The mark on a tile or a row: the cached icon, or the letter.
#[component]
pub(crate) fn ProvChip(provider: Provider, marks: crate::view::Marks, place: ChipPlace) -> Element {
    let style = mark_style(provider, marks);
    let size = match place {
        ChipPlace::Row => MarkSize::Row,
        ChipPlace::Inline => MarkSize::Inline,
    };
    rsx! {
        ProviderMark { provider: mark_of(provider), size, style }
    }
}

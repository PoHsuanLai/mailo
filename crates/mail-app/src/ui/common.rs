//! What mailo puts on a quire component's own element.

use dioxus::prelude::*;
use ds::prelude::{InlineBanner, Severity};
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;

/// A component's own layout class: quire writes it after its own, never in place of one, so a
/// mailo rule can size or place the element without naming a `ds-` class.
pub(in crate::ui) fn classed(class: &str) -> Common {
    Common {
        extra_class: ExtraClass::parse(class).ok(),
        ..Common::default()
    }
}

/// A sheet's own class: it hangs from the card's top edge, which is where the frame's inset
/// ends, and not from the window's (quire's `Attach::Window` is the window's; a sheet has no
/// attachment to a pane, which is a quire request).
pub(in crate::ui) fn in_card() -> Common {
    classed("in-card")
}

/// What the last act on a page of Settings came to, under the group it was done in: quire's
/// banner, a failure as an alert and anything else as a status. Nothing said draws nothing.
#[component]
pub(in crate::ui) fn Told(said: Option<Result<String, String>>) -> Element {
    match said {
        None => rsx! {},
        Some(Ok(text)) => rsx! {
            InlineBanner { severity: Severity::Ok, text }
        },
        Some(Err(why)) => rsx! {
            InlineBanner { severity: Severity::Danger, text: why }
        },
    }
}

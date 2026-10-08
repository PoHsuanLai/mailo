//! What mailo puts on a quire component's own element.

use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::prelude::{Icon, InlineBanner, RowLeading, Severity, TileFace};
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::icon::family::PlateFamily;

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

/// What the last act on a page of Settings came to, at the head of the page's `Form`: quire's
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

/// A grouped row's leading tile, as System Settings leads a pane's row: `icon` in white on the
/// light stop of one of design/08's plate families, so mailo picks a family and never a colour.
pub(in crate::ui) fn tile(icon: Icon, family: PlateFamily) -> RowLeading {
    RowLeading::Tile(TileFace::Glyph(icon, family.stops().0))
}

/// A grouped row's leading avatar for a person or an account: the first letter of `shown` on the
/// hue `key` (an address) always gets.
pub(in crate::ui) fn person_tile(shown: &str, key: &str) -> RowLeading {
    RowLeading::Tile(TileFace::Avatar(AvatarFace {
        initial: shown
            .chars()
            .next()
            .and_then(|ch| ch.to_uppercase().next())
            .unwrap_or('?'),
        size: AvatarSize::Size28,
        tone: AvatarTone::Person(person_hue(key)),
        shape: AvatarShape::Round,
    }))
}

/// A row of mutually exclusive buttons. The caller's options say which is on, and the
/// pick comes back as that option's position. Quire's `SegmentedControl` is the control.
#[component]
pub(in crate::ui) fn Seg(
    label: String,
    options: Vec<(String, bool)>,
    on_pick: EventHandler<usize>,
) -> Element {
    let value = options.iter().position(|(_, on)| *on).unwrap_or(0);
    let choices = options
        .into_iter()
        .enumerate()
        .map(|(index, (name, _))| ds::prelude::Choice::new(index, name))
        .collect::<Vec<_>>();
    rsx! {
        ds::prelude::SegmentedControl::<usize> {
            label,
            choices,
            tracking: ds::components::controls::segmented::Tracking::SelectOne(value),
            onchange: move |index| on_pick.call(index),
        }
    }
}

//! What mailo puts on a quire component's own element.

use dioxus::prelude::*;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::{Alert, Icon, IconSize, RowLeading, TextLine, TileFace};
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::icon::family::PlateFamily;
use ds::style::icon::render::Glyph;
use mail_ui_model::copy;

/// A component's own layout class: quire writes it after its own, never in place of one, so a
/// mailo rule can size or place the element without naming a `ds-` class. quire refuses a `ds-`
/// class here (its prefix is reserved), and a refused class is dropped; a debug build says so,
/// since every class passed is a literal and a dropped one silently undoes its rule.
pub(in crate::ui) fn classed(class: &str) -> Common {
    let extra_class = ExtraClass::parse(class);
    debug_assert!(extra_class.is_ok(), "quire refuses the class {class:?}");
    Common {
        extra_class: extra_class.ok(),
        ..Common::default()
    }
}

/// A sheet's own class: it hangs from the card's top edge, which is where the frame's inset
/// ends, and not from the window's (quire's `Attach::Window` is the window's; a sheet has no
/// attachment to a pane, which is a quire request).
pub(in crate::ui) fn in_card() -> Common {
    classed("in-card")
}

/// A failure the person has to see, as a dialog: `title`, the reason under it, and OK, which
/// (as Escape does) calls `onclose`. Settings says a failure only so; nothing it says is a
/// caption or a banner.
#[component]
pub(in crate::ui) fn Problem(title: String, why: String, onclose: EventHandler<()>) -> Element {
    rsx! {
        Alert {
            title,
            message: Some(TextLine::from(why)),
            style: AlertStyle::Warning,
            buttons: vec![AlertButton::new(copy::controls::OK, AlertRole::Normal, onclose)],
            on_dismiss: Some(onclose),
        }
    }
}

/// The failure `said` holds, if it holds one, as a [`Problem`] titled `title`; OK clears it. A
/// success is not said: what it did shows on the page.
#[component]
pub(in crate::ui) fn Told(title: String, said: Signal<Option<Result<String, String>>>) -> Element {
    let Some(Err(why)) = said() else {
        return rsx! {};
    };
    let mut said = said;
    rsx! { Problem { title, why, onclose: move |()| said.set(None) } }
}

/// A grouped row's leading tile, as System Settings leads a pane's row: `icon` drawn in one of
/// design/08's plate families, in quire's paint for the scheme, so mailo picks a family and
/// never a colour.
pub(in crate::ui) fn tile(icon: Icon, family: PlateFamily) -> RowLeading {
    RowLeading::Tile(TileFace::Plate(icon, family))
}

/// What leads a row that adds something (Add Account…, New Space, New Rule, a key to import): a
/// bare plus, plain where the rows above it lead with colour (`style/settings.css`).
pub(in crate::ui) fn add_mark() -> RowLeading {
    RowLeading::Element(rsx! {
        span { class: "add-mark",
            Glyph { icon: Icon::Plus, size: IconSize::Compact }
        }
    })
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

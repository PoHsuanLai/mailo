//! A key's or certificate's actions, in one menu: hung under the ⋯ at the end of its row, or at
//! the pointer for a right click anywhere on the row, as a Finder row's are. The row stays a line
//! of what is known of the key, with one control in it rather than a button for each act.

use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::menus::item::item::MenuItem as Line;
use ds::host::measure::{Anchor, MountedRef};
use ds::prelude::*;
use ds::root::common::Common;

use super::super::menu::anchor_at;
use mail_ui_model::copy;

/// What a key's or certificate's menu can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Act {
    Copy,
    Save,
    ExportSecret,
    Verify,
    Trust,
    Distrust,
    Delete,
}

/// Where a row's menu is open: under its ⋯, or at a right click.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) enum Opened {
    Closed,
    Below,
    At(Point),
}

/// One row of a menu: `act`, called `name`, led by `icon`.
pub(in crate::ui) fn line(act: Act, name: &str, icon: Option<Icon>) -> Line<Act> {
    let line = Line::new(act, name);
    match icon {
        Some(icon) => line.with_image(MenuImage::Icon(icon)),
        None => line,
    }
}

/// The ⋯ at the end of a row, and the row's menu while it is open. `label` names the row's
/// actions for a screen reader ("Actions for 1A2B 3C4D"); the row sets `opened` to
/// [`Opened::At`] for a right click.
#[component]
pub(in crate::ui) fn RowActions(
    label: String,
    items: Vec<Line<Act>>,
    opened: Signal<Opened>,
    onpick: EventHandler<Act>,
) -> Element {
    let mut opened = opened;
    let mut at = use_signal(|| None::<MountedRef>);
    let now = opened();
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            icon: Some(IconSource::Glyph(Icon::Ellipsis)),
            label: copy::aria::MORE.to_owned(),
            title: Some(copy::tooltips::MORE.to_owned()),
            shown: Some(if now == Opened::Below { Shown::Visible } else { Shown::Hidden }),
            onclick: move |_| {
                let next = if *opened.peek() == Opened::Below { Opened::Closed } else { Opened::Below };
                opened.set(next);
            },
            common: Common {
                aria_label: Some(label.clone()),
                mounted: Some(EventHandler::new(move |event: MountedEvent| at.set(Some(MountedRef(event.data()))))),
                ..Common::default()
            },
        }
        if now != Opened::Closed {
            Menu::<Act> {
                placement: match now {
                    Opened::At(_) => MenuPlacement::Context,
                    _ => MenuPlacement::Popup,
                },
                anchor: match now {
                    Opened::At(point) => Anchor::Point(point),
                    _ => anchor_at(at()),
                },
                items,
                onpick: move |act: Act| {
                    opened.set(Opened::Closed);
                    onpick.call(act);
                },
                onclose: move |()| opened.set(Opened::Closed),
                common: Common { aria_label: Some(label.clone()), ..Common::default() },
            }
        }
    }
}

//! The Space's parts that open where its menu stood: its name, and its colour. Each changes the
//! draft as it is edited, so the frame follows at once, and keeps it when it closes.

use super::{change, close};
use crate::ui::menu::{MenuKey, menu_key};
use crate::ui::space::Spaces;
use crate::ui::space::edit::Draft;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::app::space_editor::DotIndex;
use ds::components::overlays::popover::Arrow;
use ds::focus::request::use_focus_request;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;

/// Where a part stands: below and after the point its menu opened at, as the menu did.
fn placed() -> Placement {
    Placement::new(Side::Bottom, Align::Start)
}

/// The Space's name, in a field that takes the keyboard. Return closes it, as a click outside or
/// Escape does; each keystroke renames the Space.
#[component]
pub(super) fn RenamePopover(
    at: Point,
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
) -> Element {
    // The name selected as the field opens, so typing replaces it, as a rename does.
    let focus = FieldFocus::Controlled(use_focus_request().with_select_all());
    let Some(name) = editing
        .read()
        .as_ref()
        .map(|draft| draft.space.name.clone())
    else {
        return rsx! {};
    };
    rsx! {
        Popover {
            anchor: Anchor::Point(at),
            placement: placed(),
            gap: Px(2.0),
            arrow: Arrow::None,
            common: Common { aria_label: Some("Rename Space".to_owned()), ..Common::default() },
            onclose: move |()| close(shell, editing, spaces),
            div { class: "space-part space-rename",
                TextField {
                    label: "Space name",
                    value: name,
                    focus,
                    oninput: move |typed: String| change(editing, spaces, |draft| draft.space.name = typed),
                    onkey: move |event: KeyboardEvent| {
                        if menu_key(&event.key().to_string()) == Some(MenuKey::Enter) {
                            event.prevent_default();
                            close(shell, editing, spaces);
                        }
                    },
                }
            }
        }
    }
}

/// The Space's colour: quire's `SpaceColour` (the field and its handles, the stops, the grain
/// and the presets), in the scheme the window is drawn in.
#[component]
pub(super) fn ColourPopover(
    at: Point,
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
) -> Element {
    let scheme = use_scope().scheme;
    let Some(draft) = editing.read().clone() else {
        return rsx! {};
    };
    rsx! {
        Popover {
            anchor: Anchor::Point(at),
            placement: placed(),
            gap: Px(2.0),
            arrow: Arrow::None,
            common: Common { aria_label: Some("Space colour".to_owned()), ..Common::default() },
            onclose: move |()| close(shell, editing, spaces),
            div { class: "space-part space-colour",
                SpaceColour {
                    look: draft.space.look.clone(),
                    scheme,
                    active_dot: DotIndex(u8::try_from(draft.active).unwrap_or(0)),
                    onchange: move |look: SpaceLook| change(editing, spaces, |draft| draft.space.look = look),
                    on_active_dot: move |dot: DotIndex| change(editing, spaces, |draft| draft.active = usize::from(dot.0)),
                }
            }
        }
    }
}

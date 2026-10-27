//! The quick filter in the list pane: the toolbar's Filter button, and the row of toggles it
//! shows under the toolbar. What the toggles mean is [`crate::ui::quick`]; this draws them and
//! turns them.

use crate::ui::copy;
use crate::ui::press::on_primary;
use crate::ui::quick::{Row, Toggle};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::controls::chip::{Chip, ChipVariant};
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;

/// Turn `toggle` on or off, from the first page of what it leaves.
pub(in crate::ui) fn toggle(mut shell: Signal<Shell>, mut pages: Signal<u32>, toggle: Toggle) {
    if !shell.peek().filterable() {
        return;
    }
    let next = shell.peek().quick.toggled(toggle);
    shell.write().quick = next;
    pages.set(1);
}

/// Every toggle off: the whole list again, from its first page.
pub(in crate::ui) fn clear(mut shell: Signal<Shell>, mut pages: Signal<u32>) {
    let next = shell.peek().quick.cleared();
    shell.write().quick = next;
    pages.set(1);
}

/// The toolbar's Filter button: shows the row of toggles, or hides it and turns them off. Pressed
/// in while a toggle narrows the list. Not drawn where the list holds no conversations.
#[component]
pub(super) fn FilterButton(shell: Signal<Shell>, mut pages: Signal<u32>) -> Element {
    if !shell.read().filterable() {
        return rsx! {};
    }
    let quick = shell.read().quick.clone();
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            size: ControlSize::Large,
            image: ImagePosition::Only,
            label: copy::aria::FILTER,
            icon: Some(IconSource::Glyph(Icon::Switches)),
            title: Some(copy::tooltips::FILTER.to_owned()),
            value: Some(if quick.is_active() { Check::On } else { Check::Off }),
            shown: Some(if quick.row == Row::Shown { Shown::Visible } else { Shown::Hidden }),
            onclick: on_primary(move || {
                let next = shell.peek().quick.flipped();
                let narrowed = shell.peek().quick.is_active();
                shell.write().quick = next;
                if narrowed {
                    pages.set(1);
                }
            }),
        }
    }
}

/// The row of toggles under the toolbar while the Filter button shows it: Unread, Starred and
/// Has Attachments, each pressed in while on, and Clear Filter while any is.
#[component]
pub(super) fn QuickRow(shell: Signal<Shell>, pages: Signal<u32>) -> Element {
    let quick = shell.read().quick.clone();
    if quick.row == Row::Hidden || !shell.read().filterable() {
        return rsx! {};
    }
    rsx! {
        div { class: "quick-filter", role: "group", aria_label: copy::aria::QUICK_FILTER,
            for one in Toggle::ALL {
                Chip {
                    key: "{one.term()}",
                    variant: ChipVariant::Neutral,
                    text: one.label().to_owned(),
                    pressed: Some(if quick.is_on(one) { Check::On } else { Check::Off }),
                    onpress: Some(EventHandler::new(on_primary(move || toggle(shell, pages, one)))),
                }
            }
            if quick.is_active() {
                Button {
                    bezel: Bezel::Inline,
                    label: copy::controls::CLEAR_FILTER,
                    onclick: on_primary(move || clear(shell, pages)),
                }
            }
        }
    }
}

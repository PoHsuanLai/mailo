//! What a quire control's press does in mailo.
//!
//! quire's `Button` and `IconButton` report every press: the primary button, a right-click and
//! a middle click alike (`ds::Press`). A mailo button only ever answered the primary one, so a
//! right-click on Delete must still do nothing.

use dioxus::prelude::*;

/// `act`, on a primary press (a click, or the keyboard) and on no other.
pub(super) fn on_primary(mut act: impl FnMut() + 'static) -> impl FnMut(ds::Press) + 'static {
    move |press: ds::Press| {
        if press.button == ds::PointerButton::Primary {
            act();
        }
    }
}

/// A sheet's Close, quire's button, with the key that does the same beside it: a `Button` draws
/// a label and an icon, so the `Esc` it used to hold now sits next to it.
#[component]
pub(super) fn SheetClose(
    #[props(default = "Close".to_owned())] label: String,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        span { class: "sheet-close",
            ds::Button {
                variant: ds::ButtonVariant::Mini,
                label,
                onclick: on_primary(move || on_close.call(())),
            }
            span { class: "k", "Esc" }
        }
    }
}

/// A button's availability from whether it may be pressed now.
pub(super) fn available(enabled: bool) -> ds::Availability {
    if enabled {
        ds::Availability::Enabled
    } else {
        ds::Availability::Disabled
    }
}

#[cfg(test)]
mod tests {
    use super::on_primary;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn only_a_primary_press_acts() {
        let count = Rc::new(Cell::new(0));
        let seen = count.clone();
        let mut act = on_primary(move || seen.set(seen.get() + 1));
        for (button, acts) in [
            (ds::PointerButton::Primary, true),
            (ds::PointerButton::Secondary, false),
            (ds::PointerButton::Middle, false),
        ] {
            let before = count.get();
            act(ds::Press {
                button,
                ..ds::Press::primary()
            });
            assert_eq!(count.get() > before, acts, "{button:?}");
        }
    }
}

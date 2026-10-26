//! What a quire control's press does in mailo.
//!
//! quire's `Button` and `IconButton` report every press: the primary button, a right-click and
//! a middle click alike (`Press`). A mailo button only ever answered the primary one, so a
//! right-click on Delete must still do nothing.

use dioxus::prelude::*;
use ds::base::press::{PointerButton, Press};
use ds::components::controls::button_model::Answers;
use ds::prelude::*;

/// `act`, on a primary press (a click, or the keyboard) and on no other.
pub(super) fn on_primary(mut act: impl FnMut() + 'static) -> impl FnMut(Press) + 'static {
    move |press: Press| {
        if press.button == PointerButton::Primary {
            act();
        }
    }
}

/// A sheet's Close: quire's push button answering Escape, as an `NSAlert`'s Cancel does, so
/// Escape and a click do the same and the sheet shows no key of its own.
#[component]
pub(super) fn SheetClose(
    #[props(default = "Close".to_owned())] label: String,
    on_close: EventHandler<()>,
) -> Element {
    rsx! {
        Button {
            label,
            answers: Answers::Escape,
            onclick: on_primary(move || on_close.call(())),
        }
    }
}

/// A button's availability from whether it may be pressed now.
pub(super) fn available(enabled: bool) -> Availability {
    if enabled {
        Availability::Enabled
    } else {
        Availability::Disabled
    }
}

#[cfg(test)]
mod tests {
    use super::on_primary;
    use ds::base::press::{PointerButton, Press};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn only_a_primary_press_acts() {
        let count = Rc::new(Cell::new(0));
        let seen = count.clone();
        let mut act = on_primary(move || seen.set(seen.get() + 1));
        for (button, acts) in [
            (PointerButton::Primary, true),
            (PointerButton::Secondary, false),
            (PointerButton::Middle, false),
        ] {
            let before = count.get();
            act(Press {
                button,
                ..Press::primary()
            });
            assert_eq!(count.get() > before, acts, "{button:?}");
        }
    }
}

//! Organise by Conversation: the place shown lists conversations, or each message on a row of
//! its own, and keeps the choice.
//!
//! Kept per place, beside what else the place is: a saved view in its own row in the client
//! ([`mail_domain::View::threading`]), as its grouping is; a mailbox, label or folder among
//! `window.single_messages` in `settings.toml` ([`crate::ui::prefs::change`]), as the list's
//! other choices are. The list's ⋯ and the search bar's panel both turn it here.

use crate::ui::view::Shell;
use dioxus::prelude::*;
use mail_core::conversation::{kept_single, saved_of, saved_place};
use mail_domain::Threading;

/// The other way of listing from `now`.
fn turned(now: Threading) -> Threading {
    match now {
        Threading::Threaded => Threading::Single,
        Threading::Single => Threading::Threaded,
    }
}

/// Turn Organise by Conversation the other way for the place shown, and keep it there. Nothing
/// where the place does not offer it ([`Shell::organises`]). The open conversation stays open,
/// shown whole: a message opened alone was opened from the rows that are going.
pub(in crate::ui) fn turn(mut shell: Signal<Shell>, mut revision: Signal<u64>) {
    let (place, at, next) = {
        let current = shell.peek();
        if !current.organises() {
            return;
        }
        let Some(place) = current.places.get(current.selected).cloned() else {
            return;
        };
        (place, current.selected, turned(current.threading()))
    };
    match saved_of(&place) {
        Some(view) => {
            let view = mail_domain::View {
                threading: next,
                ..view.clone()
            };
            let client = crate::ui::live::client();
            if let Err(why) = client.stored().put_view(&view) {
                log::warn!("Organise by Conversation was not kept in the view: {why}");
                return;
            }
            let mut write = shell.write();
            write.places[at] = saved_place(&view);
            write.focus = None;
        }
        None => {
            {
                let mut write = shell.write();
                write.single = kept_single(&write.single, &place, next);
                write.focus = None;
            }
            // Shown at once whatever the write does; a window with nowhere to write keeps it
            // until it closes.
            if let Err(why) = crate::ui::prefs::change(|settings| {
                let window = &mut settings.window;
                window.single_messages = kept_single(&window.single_messages, &place, next);
            }) {
                log::warn!("Organise by Conversation was not kept: {why}");
            }
        }
    }
    revision += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_twice_comes_back() {
        for now in [Threading::Threaded, Threading::Single] {
            assert_ne!(turned(now), now, "{now:?}");
            assert_eq!(turned(turned(now)), now, "{now:?}");
        }
    }
}

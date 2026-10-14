//! Switching Space: where each Space was left.
//!
//! quire's kit switches (`SpacesHandle::switch`): it remembers where the window is for the Space
//! being left, writes `spaces.json`, cross-fades the frame, and hands back where the Space
//! arrived at was left. This is mailo's side of that: what "where" is ([`recall_of`]), and
//! showing it again ([`restore`]). The Space's accounts follow on their own (`App` copies the
//! current Space's scope into the shell whenever the Spaces change).

use crate::ui::space::Recall;
use crate::ui::view::{PageMenu, Shell};

/// Where `shell` is, as the Space it is showing will remember it.
pub(super) fn recall_of(shell: &Shell) -> Recall {
    Recall {
        place: shell
            .places
            .get(shell.selected)
            .map(crate::ui::view::place_key)
            .unwrap_or_default(),
        open: shell.open,
    }
}

/// Show in `shell` where `recall` says a Space was left.
///
/// A place that no longer exists is the first place: a Space can lose a label while you are
/// elsewhere, and coming back must not show a list that cannot exist. The place is found by its
/// key, so a label called Done comes back as the label and not as the Done place; a name stored
/// before keys, and one the sidebar no longer uses (Archive, Starred), is read as the place it
/// meant. The whole Space is
/// shown: a Space is the only narrowing of accounts there is.
pub(super) fn restore(shell: &mut Shell, recall: &Recall) {
    shell.selected = crate::ui::view::find_place(&shell.places, &recall.place).unwrap_or(0);
    shell.open = recall.open;
    shell.picked = crate::ui::selection::Picked::none();
    shell.show_remote_images = false;
    shell.search.clear();
    shell.page_menu = PageMenu::Closed;
    shell.snoozing = None;
    shell.labelling = None;
}

#[cfg(test)]
mod tests;

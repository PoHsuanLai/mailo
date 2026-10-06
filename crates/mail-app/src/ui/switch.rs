//! Switching Space: which mail the list shows, and where each Space was left.
//!
//! [`switch`] is the decision, on plain values. [`go`] is the same thing on the window's
//! signals, plus the write to `spaces.json`; the frame repaints itself from the Spaces, and
//! quire's root cross-fades the Space's colour.

use super::frame::keep;
use crate::ui::space::edit::Draft;
use crate::ui::space::{self, Recall, Space, Spaces};
use crate::ui::view::{PageMenu, Shell};
use dioxus::prelude::*;

/// The Space ⌘ and a digit ask for: `"1"` is the first. `None` for any other key.
pub(super) fn space_key(key: &str) -> Option<usize> {
    let mut chars = key.chars();
    let digit = chars.next()?.to_digit(10)?;
    if chars.next().is_some() || digit == 0 {
        return None;
    }
    usize::try_from(digit - 1).ok()
}

/// Where `shell` is, as the Space it is showing will remember it.
fn recall_of(shell: &Shell) -> Recall {
    Recall {
        place: shell
            .places
            .get(shell.selected)
            .map(|place| place.name.clone())
            .unwrap_or_default(),
        open: shell.open,
        account: shell.account.clone(),
    }
}

/// Show `space` in `shell`, where `recall` says it was left.
///
/// A place that no longer exists is the first place, and an account tile outside the
/// Space's scope is every account: a Space can lose a label or an account while you are
/// elsewhere, and coming back must not show a list that cannot exist.
pub(super) fn restore(shell: &mut Shell, space: &Space, recall: &Recall) {
    shell.scope = space.scope.clone();
    shell.account = recall
        .account
        .clone()
        .filter(|id| shell.scope.shows(id.clone()));
    shell.selected = shell
        .places
        .iter()
        .position(|place| place.name == recall.place)
        .unwrap_or(0);
    shell.open = recall.open;
    shell.picked = crate::ui::selection::Picked::none();
    shell.show_remote_images = false;
    shell.search.clear();
    shell.page_menu = PageMenu::Closed;
    shell.snoozing = None;
    shell.labelling = None;
}

/// Leave the current Space for the one at `index`.
///
/// The Space being left remembers its place, open thread and account tile; the one arrived
/// at gets its own back. `Moved::No` when `index` is the current Space or past the last, which
/// changes nothing.
pub(super) fn switch(spaces: &mut Spaces, shell: &mut Shell, index: usize) -> Moved {
    let from = spaces.current;
    if index >= spaces.spaces.len() || index == from {
        return Moved::No;
    }
    spaces.recall.insert(from, recall_of(shell));
    spaces.current = index;
    let recall = spaces.recall.get(&index).cloned().unwrap_or_default();
    restore(shell, &spaces.current_space(), &recall);
    Moved::Yes
}

/// Whether a switch changed the Space on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Moved {
    /// Another Space is showing now.
    Yes,
    /// Nothing changed.
    No,
}

/// The window's side of a switch: the decision, then the file.
pub(super) fn go(
    mut spaces: Signal<Spaces>,
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    index: usize,
) {
    let moved = {
        let mut all = spaces.write();
        let mut showing = shell.write();
        switch(&mut all, &mut showing, index)
    };
    if moved == Moved::No {
        return;
    }
    // No repaint to ask for: the window's `Ds` root reads the current Space and cross-fades its
    // frame to the new one by itself.
    pages.set(1);
    keep(&spaces.read());
}

/// "+": a new Space from the next preset, switched to, with the editor open on it.
///
/// It is written before the editor opens, so Esc in the editor returns to this Space as it
/// was made rather than removing it.
pub(super) fn add(
    mut spaces: Signal<Spaces>,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    mut editing: Signal<Option<Draft>>,
) {
    let made = space::new_space(&spaces.read());
    let index = {
        let mut all = spaces.write();
        all.spaces.push(made.clone());
        all.spaces.len() - 1
    };
    go(spaces, shell, pages, index);
    editing.set(Some(Draft::open(index, made)));
}

/// Open the editor on the current Space.
pub(super) fn edit(spaces: Signal<Spaces>, mut editing: Signal<Option<Draft>>) {
    let index = spaces.read().current;
    let space = spaces.read().current_space();
    editing.set(Some(Draft::open(index, space)));
}

#[cfg(test)]
mod tests;

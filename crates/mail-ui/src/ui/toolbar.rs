//! The icon buttons of the two toolbars, the pick bar over the list and the reader's, drawn from
//! the same data as the row's menu.
//!
//! A button is an action's glyph ([`op_icon`], the one the menu's row wears), the word the
//! caller names it by (the menu's, from [`mail_client::view::says`]), and the key that does the
//! same thing ([`shortcut_for`]), so a toolbar and the menu beside it never disagree about what
//! a button is called, looks like or is bound to.

use super::row::act::op_icon;
use crate::ui::view::Shortcut;
use dioxus::prelude::*;
use ds::base::press::Press;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::OpKind;

/// The key that does what `kind` does, for a tooltip's hint. Both sides of a pair share one key,
/// as the keymap's toggles do. `None` for what no key reaches.
pub(in crate::ui) fn shortcut_for(kind: OpKind) -> Option<Shortcut> {
    match kind {
        OpKind::Reply => Some(Shortcut::Reply),
        OpKind::ReplyAll => Some(Shortcut::ReplyAll),
        OpKind::Forward => Some(Shortcut::Forward),
        OpKind::Archive => Some(Shortcut::Archive),
        OpKind::Trash => Some(Shortcut::Trash),
        OpKind::Spam => Some(Shortcut::Spam),
        OpKind::MarkRead | OpKind::MarkUnread => Some(Shortcut::ToggleRead),
        OpKind::Star | OpKind::Unstar => Some(Shortcut::ToggleStar),
        OpKind::Pin => Some(Shortcut::TogglePin),
        OpKind::Mute => Some(Shortcut::ToggleMute),
        OpKind::Restore
        | OpKind::Destroy
        | OpKind::AddLabel
        | OpKind::RemoveLabel
        | OpKind::Snooze
        | OpKind::FollowUp => None,
    }
}

/// One toolbar button: what it does, as a menu row would say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) struct ToolButton {
    pub icon: Icon,
    pub label: &'static str,
    pub shortcut: Option<Shortcut>,
}

impl ToolButton {
    /// The button for `kind`, named `label`: the menu's glyph and the key that does the same.
    pub(in crate::ui) fn of(kind: OpKind, label: &'static str) -> Self {
        Self {
            icon: op_icon(kind),
            label,
            shortcut: shortcut_for(kind),
        }
    }
}

/// `tool` as a toolbar button. `aria` is what a screen reader says, which a bar over several
/// conversations says with their count.
#[component]
pub(in crate::ui) fn Tool(
    tool: ToolButton,
    aria: Option<String>,
    onclick: EventHandler<Press>,
) -> Element {
    rsx! {
        Button {
            bezel: Bezel::Toolbar,
            image: ImagePosition::Only,
            size: ControlSize::Large,
            label: tool.label.to_owned(),
            icon: Some(IconSource::Glyph(tool.icon)),
            title: Some(tool.label.to_owned()),
            title_shortcut: tool.shortcut.and_then(crate::ui::actions::tip),
            common: Common { aria_label: aria, ..Common::default() },
            onclick,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pair_shares_the_key_of_its_two_sides() {
        assert_eq!(shortcut_for(OpKind::Star), shortcut_for(OpKind::Unstar));
        assert_eq!(
            shortcut_for(OpKind::MarkRead),
            shortcut_for(OpKind::MarkUnread)
        );
        assert_eq!(shortcut_for(OpKind::Restore), None);
    }
}

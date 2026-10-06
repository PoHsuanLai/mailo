//! What a row's actions do once picked from its menu: the op, at once, over the selection
//! when the row is picked; or the menu, sheet or composer the op needs first.

use super::super::motion::{act_kind, act_kind_all};
use super::super::ops::{composes, start_composing};
use super::super::picks::{mute_all, with_selection};
use super::super::text::label;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::prelude::*;
use mail_domain::*;
use mail_store::SqliteStore;
use std::sync::Arc;

/// Which of the row's actions was picked from its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pressed {
    Op(OpKind),
    MoveTo,
}

/// One of the row's actions picked: the op it names, at once. Label, snooze and move open their
/// menus (picked while open, they close); a reply opens the composer on its draft.
pub(super) fn press(
    mut shell: Signal<Shell>,
    mut revision: Signal<u64>,
    id: ThreadId,
    pressed: Pressed,
) {
    let kind = match pressed {
        Pressed::Op(kind) => kind,
        Pressed::MoveTo => {
            let already = shell.peek().filing == Some(id);
            shell.write().filing = if already { None } else { Some(id) };
            return;
        }
    };
    let store = consume_context::<Arc<SqliteStore>>();
    if kind == OpKind::AddLabel {
        let already = shell.peek().labelling == Some(id);
        shell.write().labelling = if already { None } else { Some(id) };
        return;
    }
    if kind == OpKind::Snooze {
        let already = shell.peek().snoozing == Some(id);
        shell.write().snoozing = if already { None } else { Some(id) };
        return;
    }
    // Never at once: the sheet names how much and asks, over this row and every one picked.
    if kind == OpKind::Destroy {
        crate::ui::destroy::ask_chosen(&store, shell, &with_selection(shell, id));
        return;
    }
    // An op may take the row out of the list. Take the keyboard back before it can leave
    // with the row (FINDINGS F172).
    if composes(kind).is_none() {
        crate::ui::host::Host::focus_app();
    }
    // Mute takes its direction from the conversations it reaches, so a picked row mutes or
    // unmutes the whole selection as one gesture.
    if kind == OpKind::Mute {
        mute_all(&store, shell, revision, &with_selection(shell, id));
        return;
    }
    match composes(kind) {
        Some(what) => match start_composing(&store, id, what) {
            Ok(draft) => {
                shell.write().compose(&draft);
                revision += 1;
            }
            Err(why) => eprintln!("reply: {why}"),
        },
        // A press on a picked row acts on everything picked, as one gesture. An op that needs
        // more than the button (a pin's rank) stays with its own row.
        None if crate::ui::view::op_for(kind).is_some() => {
            act_kind_all(&store, shell, revision, &with_selection(shell, id), kind);
        }
        None => {
            act_kind(&store, shell, revision, id, kind);
        }
    }
}

/// An action's name. Mute says what picking it does to this row, as Read and Star do by being
/// two kinds.
pub(super) fn strip_label(kind: OpKind, muted: bool) -> &'static str {
    match kind {
        OpKind::Mute if muted => "Unmute",
        other => label(other),
    }
}

pub(super) fn op_icon(kind: OpKind) -> Icon {
    match kind {
        OpKind::Archive => Icon::Archive,
        OpKind::Trash => Icon::Trash,
        OpKind::Restore => Icon::Corner,
        OpKind::Spam => Icon::OctagonAlert,
        OpKind::MarkRead => Icon::MailOpen,
        OpKind::MarkUnread => Icon::Mail,
        OpKind::Star | OpKind::Unstar => Icon::Star,
        OpKind::AddLabel | OpKind::RemoveLabel => Icon::Tag,
        OpKind::Snooze => Icon::Clock,
        OpKind::Pin => Icon::Pin,
        OpKind::Mute => Icon::BellOff,
        OpKind::FollowUp => Icon::Bell,
        OpKind::Destroy => Icon::Trash,
        OpKind::Reply => Icon::Reply,
        OpKind::ReplyAll => Icon::ReplyAll,
        OpKind::Forward => Icon::Forward,
    }
}

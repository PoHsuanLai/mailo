//! What a row's actions do once picked from its menu: the op, at once, over the selection
//! when the row is picked; or the menu, sheet or composer the op needs first.

use super::super::motion::{act_all, act_kind, act_kind_all};
use super::super::ops::{Writes, composes, open_composer};
use super::super::picks::{mute_all, with_selection};
use crate::ui::view::{Further, FurtherMenu, Opened, Shell};
use dioxus::prelude::*;
use ds::prelude::*;
use mail_client::Client;
use mail_domain::*;

/// Which of the row's actions was picked from its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Pressed {
    Op(OpKind),
    MoveTo,
}

/// The further menu `pressed` opens before it acts, if it opens one.
pub(in crate::ui) fn further(pressed: Pressed) -> Option<Further> {
    match pressed {
        Pressed::MoveTo => Some(Further::Move),
        Pressed::Op(OpKind::AddLabel) => Some(Further::Label),
        Pressed::Op(OpKind::Snooze) => Some(Further::Snooze),
        Pressed::Op(_) => None,
    }
}

/// One of a conversation's actions picked, from its row (`opened`) or the reader: the op it
/// names, at once. Label, snooze and move open their menus where they were asked for (picked
/// while open, they close); a reply opens the composer on its draft.
pub(in crate::ui) fn press(
    mut shell: Signal<Shell>,
    id: ThreadId,
    pressed: Pressed,
    opened: Opened,
) {
    let client = crate::ui::live::client();
    // Offered as the way out of a state (Unsnooze, Stop Reminding): at once, with no menu.
    if let Pressed::Op(kind) = pressed
        && turn_off(&client, shell, id, kind)
    {
        return;
    }
    if let Some(further) = further(pressed) {
        shell.write().toggle_further(FurtherMenu {
            thread: id,
            further,
            opened,
        });
        return;
    }
    let Pressed::Op(kind) = pressed else {
        return;
    };
    // Never at once: the sheet names how much and asks, over this row and every one picked.
    if kind == OpKind::Destroy {
        crate::ui::destroy::ask_chosen(&client, shell, &with_selection(shell, id));
        return;
    }
    // An op may take the row out of the list. Take the keyboard back before it can leave
    // with the row.
    if composes(kind).is_none() {
        crate::ui::host::Host::focus_app();
    }
    // Mute takes its direction from the conversations it reaches, so a picked row mutes or
    // unmutes the whole selection as one gesture.
    if kind == OpKind::Mute {
        mute_all(&client, shell, &with_selection(shell, id));
        return;
    }
    match composes(kind) {
        Some(what) => {
            // A refusal is the composer's to say (`open_composer` logs it).
            let _ = open_composer(&client, shell, Writes::On(id, what));
        }
        // A press on a picked row acts on everything picked, as one gesture. An op that needs
        // more than the button (a pin's rank) stays with its own row.
        None if mail_client::conversation::op_for(kind).is_some() => {
            act_kind_all(&client, shell, &with_selection(shell, id), kind);
        }
        None => {
            act_kind(&client, shell, id, kind);
        }
    }
}

/// `kind` picked where the conversation is offered it as the way out of a state it is in
/// ([`mail_client::conversation::turns_off`]): that, for it and for whatever is picked with it and in the
/// same state, as one gesture. Returns whether it was; when not, `kind` opens its menu of times.
pub(in crate::ui) fn turn_off(
    client: &Client,
    shell: Signal<Shell>,
    id: ThreadId,
    kind: OpKind,
) -> bool {
    // Only these two turn a state off; anything else needs no load to say no.
    if !matches!(kind, OpKind::Snooze | OpKind::FollowUp) {
        return false;
    }
    let now = chrono::Utc::now();
    let off = |thread: ThreadId| {
        let loaded = client.stored().thread(thread).ok()?;
        mail_client::conversation::turns_off(kind, &loaded.summary, now)
    };
    if off(id).is_none() {
        return false;
    }
    let ops = with_selection(shell, id)
        .into_iter()
        .filter_map(|thread| off(thread).map(|op| (thread, op)))
        .collect();
    act_all(client, shell, ops);
    true
}

pub(in crate::ui) fn op_icon(kind: OpKind) -> Icon {
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

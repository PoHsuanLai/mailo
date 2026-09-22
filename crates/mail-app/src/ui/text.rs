use crate::view::Stamp;
use chrono::Local;
use mail_domain::*;

pub(super) fn draft_state(state: &SendState) -> &'static str {
    match state {
        SendState::Editing => "draft",
        SendState::Queued => "queued",
        SendState::Sending => "sending",
        SendState::Failed { .. } => "failed",
        SendState::Sent { .. } => "sent",
    }
}

pub(super) fn from_name(message: &Message) -> String {
    message.from.name.clone().unwrap_or_default()
}

/// Each attachment as the name it would be written under and a readable size.
///
/// The name is `attach::safe_name`, not the claim: showing `../../escape.pdf` would describe
/// something that does not happen, and the pane and the command must agree.
pub(super) fn attached(message: &Message) -> Vec<(usize, (String, String))> {
    message
        .attachments
        .iter()
        .enumerate()
        .map(|(index, a)| {
            (
                index,
                (
                    crate::attach::safe_name(&a.name),
                    crate::attach::human_size(a.size),
                ),
            )
        })
        .collect()
}

pub(super) fn address(message: &Message) -> String {
    format!(" <{}>", message.from.email)
}

pub(super) fn stamp(message: &Message) -> String {
    crate::view::stamp(message.date, &Local, Stamp::Full)
}

pub(super) fn sender(summary: &ThreadSummary) -> String {
    summary
        .from
        .name
        .clone()
        .unwrap_or_else(|| summary.from.email.clone())
}

pub(super) fn label(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Archive => "Archive",
        OpKind::Trash => "Trash",
        OpKind::Restore => "Restore",
        OpKind::Spam => "Spam",
        OpKind::MarkRead => "Read",
        OpKind::MarkUnread => "Unread",
        OpKind::Star => "Star",
        OpKind::Unstar => "Unstar",
        OpKind::AddLabel | OpKind::RemoveLabel => "Label",
        OpKind::Snooze => "Snooze",
        OpKind::Pin => "Pin",
        OpKind::Reply => "Reply",
        OpKind::ReplyAll => "Reply all",
        OpKind::Forward => "Forward",
    }
}

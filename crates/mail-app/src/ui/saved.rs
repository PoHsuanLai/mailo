//! Saved views, as the editor holds one and as the store keeps it.
//!
//! A view is a search that stays: its filter, how its list is grouped, and which actions its
//! rows' menu offers beside the ones every row's menu has. It is a sidebar place ([`crate::ui::view::saved_place`]) and a row in
//! the store ([`mail_core::Store::views`]). Free of Dioxus, like [`crate::ui::view`]: turning what
//! was typed into a [`View`] can be wrong without a window.

use crate::ui::view::{Place, Shell, saved_of, saved_place};
use chrono::TimeZone;
use mail_core::views::{Draft, Refusal};
use mail_domain::{GroupKey, LabelId, OpKind, Property, View, ViewId};

pub use mail_core::views::written;

/// A view being made or changed, as the editor holds it: [`mail_core::views::Draft`] and what the
/// last Save or Delete refused, in words.
///
/// The draft's fields read and write through this (`draft.name`, `draft.hover`), so the editor
/// holds one value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDraft {
    pub draft: Draft,
    /// What Save or Delete last refused, in words.
    pub refused: Option<String>,
}

impl std::ops::Deref for ViewDraft {
    type Target = Draft;

    fn deref(&self) -> &Draft {
        &self.draft
    }
}

impl std::ops::DerefMut for ViewDraft {
    fn deref_mut(&mut self) -> &mut Draft {
        &mut self.draft
    }
}

impl ViewDraft {
    /// A new view of what `search` finds: "Save as view" with the search box's words, or an
    /// empty box for a view started from nothing.
    pub fn from_search(search: &str) -> Self {
        Self {
            draft: Draft::from_search(search),
            refused: None,
        }
    }

    /// `view`, opened for editing, its filter written back as words where they say exactly it.
    pub fn of<Tz: TimeZone>(view: &View, labels: &[(String, LabelId)], zone: &Tz) -> Self {
        Self {
            draft: Draft::of(view, labels, zone),
            refused: None,
        }
    }

    /// The view this draft saves as, or why it cannot be one yet, in words.
    pub fn build<Tz: TimeZone>(
        &self,
        labels: &[(String, LabelId)],
        zone: &Tz,
    ) -> Result<View, String> {
        self.draft.build(labels, zone).map_err(|refusal| {
            match refusal {
                Refusal::NoName => "Give the view a name: it is what the sidebar calls it.",
                Refusal::NoQuery => "Say what the view lists, as you would search for it.",
            }
            .to_owned()
        })
    }
}

/// The sidebar after `view` was kept: its place, new at the end or changed where it was, and
/// shown, with the search that made it cleared and the editor closed.
///
/// Done here rather than waiting for the window to read the store again, so the list is the
/// view's the moment Save is pressed. Saved views are the sidebar's last places, so adding one
/// moves no other place's index.
pub fn show_kept(shell: &mut Shell, view: &View) {
    let place = saved_place(view);
    let at = match shell.places.iter().position(|p| is_view(p, view.id)) {
        Some(at) => {
            shell.places[at] = place;
            at
        }
        None => {
            shell.places.push(place);
            shell.places.len() - 1
        }
    };
    shell.search.clear();
    shell.view_editor = None;
    shell.select(at);
}

/// The sidebar after the view `id` was forgotten: its place gone, and the inbox shown if it was
/// the one being shown. The place chosen before stays chosen otherwise.
pub fn show_forgotten(shell: &mut Shell, id: ViewId) {
    shell.view_editor = None;
    let Some(at) = shell.places.iter().position(|p| is_view(p, id)) else {
        return;
    };
    shell.places.remove(at);
    match shell.selected.cmp(&at) {
        std::cmp::Ordering::Equal => shell.select(0),
        std::cmp::Ordering::Greater => shell.selected -= 1,
        std::cmp::Ordering::Less => {}
    }
}

fn is_view(place: &Place, id: ViewId) -> bool {
    saved_of(place).is_some_and(|view| view.id == id)
}

/// The actions a view's rows can be given, in the order the editor offers them.
///
/// Star is not here: every row draws its own star. Mark read stands for the pair, and is drawn
/// as whichever the conversation needs ([`mail_core::view::hover_in`]).
pub const HOVER_CHOICES: [OpKind; 10] = [
    OpKind::Archive,
    OpKind::Trash,
    OpKind::Spam,
    OpKind::MarkRead,
    OpKind::Snooze,
    OpKind::AddLabel,
    OpKind::Pin,
    OpKind::Mute,
    OpKind::Reply,
    OpKind::Forward,
];

/// What the editor calls a hover button.
pub fn hover_name(kind: OpKind) -> &'static str {
    match kind {
        OpKind::Archive => "Archive",
        OpKind::Trash => "Trash",
        OpKind::Restore => "Restore",
        OpKind::Spam => "Spam",
        OpKind::MarkRead | OpKind::MarkUnread => "Read or unread",
        OpKind::Star | OpKind::Unstar => "Star",
        OpKind::AddLabel => "Label",
        OpKind::RemoveLabel => "Remove label",
        OpKind::Snooze => "Snooze",
        OpKind::Pin => "Pin",
        OpKind::Mute => "Mute",
        OpKind::FollowUp => "Remind me",
        OpKind::Destroy => "Delete forever",
        OpKind::Reply => "Reply",
        OpKind::ReplyAll => "Reply all",
        OpKind::Forward => "Forward",
    }
}

/// Every grouping the editor offers, as `(key, name, grouping)`: none, the fixed ones, then
/// one per label in `labels`.
pub fn group_choices(labels: &[(String, LabelId)]) -> Vec<(String, String, Option<GroupKey>)> {
    let fixed = [
        None,
        Some(GroupKey::Read),
        Some(GroupKey::Star),
        Some(GroupKey::Property(Property::Date)),
        Some(GroupKey::Property(Property::From)),
        Some(GroupKey::Property(Property::Attachments)),
        Some(GroupKey::Mailbox),
    ];
    // A label listed twice (one name on two accounts is two labels, but one id is one label)
    // is offered once.
    let mut ids: Vec<LabelId> = Vec::new();
    for (_, id) in labels {
        if !ids.contains(id) {
            ids.push(*id);
        }
    }
    fixed
        .into_iter()
        .chain(ids.into_iter().map(|id| Some(GroupKey::Label(id))))
        .map(|key| {
            (
                group_word(key.as_ref()),
                group_name(key.as_ref(), labels),
                key,
            )
        })
        .collect()
}

/// A grouping's stable key in a menu.
pub fn group_word(key: Option<&GroupKey>) -> String {
    match key {
        None => "none".to_owned(),
        Some(GroupKey::Read) => "read".to_owned(),
        Some(GroupKey::Star) => "star".to_owned(),
        Some(GroupKey::Mailbox) => "mailbox".to_owned(),
        Some(GroupKey::Label(id)) => format!("label:{id}"),
        Some(GroupKey::Property(property)) => format!("by:{property:?}").to_lowercase(),
    }
}

/// What a grouping is called, a label by its name.
pub fn group_name(key: Option<&GroupKey>, labels: &[(String, LabelId)]) -> String {
    match key {
        None => "No grouping".to_owned(),
        Some(GroupKey::Read) => "Unread, then read".to_owned(),
        Some(GroupKey::Star) => "Starred, then the rest".to_owned(),
        Some(GroupKey::Mailbox) => "Where it is".to_owned(),
        Some(GroupKey::Label(id)) => {
            let name = labels
                .iter()
                .find(|(_, known)| known == id)
                .map_or("a label", |(name, _)| name.as_str());
            format!("Labelled {name}, and not")
        }
        Some(GroupKey::Property(property)) => match property {
            Property::Date => "Today, yesterday, this week, older",
            Property::From | Property::Sender => "Who sent it",
            Property::Subject => "Subject",
            Property::Size => "Size",
            Property::Attachments => "With attachments, then without",
            Property::Pin => "Pinned, then the rest",
        }
        .to_owned(),
    }
}

#[cfg(test)]
mod tests;

//! Saved views, as the editor holds one and as the store keeps it.
//!
//! A view is a search that stays: its filter, how its list is grouped, and which actions its
//! rows' menu offers beside the ones every row's menu has. It is a sidebar place ([`crate::ui::view::saved_place`]) and a row in
//! the store ([`mail_core::Store::views`]). Free of Dioxus, like [`crate::ui::view`]: turning what
//! was typed into a [`View`] can be wrong without a window.

use crate::ui::view::{Place, Shell, saved_of, saved_place};
use chrono::TimeZone;
use mail_domain::{
    Filter, GroupKey, LabelId, OpKind, Property, Sort, SortDir, Threading, View, ViewId, ViewKind,
};

/// A view being made or changed, as the editor's fields hold it.
///
/// The filter is words, not a [`Filter`], for the reason the composer's recipients are strings:
/// what is being typed is not a filter yet. It is read once, on Save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewDraft {
    /// The view being edited, or `None` for a new one.
    pub id: Option<ViewId>,
    pub name: String,
    /// What the view lists, in the search language.
    pub query: String,
    /// The saved filter, when it could not be written back as words ([`written`]). Kept as it
    /// is while [`Self::query`] stays empty, so opening such a view to rename it does not change
    /// what it lists.
    pub kept: Option<Filter>,
    pub group: Option<GroupKey>,
    /// The actions a row's menu offers, in the order they were chosen (the menu keeps its own
    /// order). Empty is the usual set. Named `hover` for the strip that once drew them, as the
    /// store keeps it.
    pub hover: Vec<OpKind>,
    pub dir: SortDir,
    /// What Save or Delete last refused, in words.
    pub refused: Option<String>,
}

impl ViewDraft {
    /// A new view of what `search` finds: "Save as view" with the search box's words, or an
    /// empty box for a view started from nothing.
    pub fn from_search(search: &str) -> Self {
        let search = search.trim();
        Self {
            id: None,
            name: search.to_owned(),
            query: search.to_owned(),
            kept: None,
            group: None,
            hover: Vec::new(),
            dir: SortDir::Desc,
            refused: None,
        }
    }

    /// `view`, opened for editing, its filter written back as words where they say exactly it.
    pub fn of<Tz: TimeZone>(view: &View, labels: &[(String, LabelId)], zone: &Tz) -> Self {
        let words = written(&view.filter, labels, zone);
        Self {
            id: Some(view.id),
            name: view.name.clone(),
            kept: words.is_none().then(|| view.filter.clone()),
            query: words.unwrap_or_default(),
            group: view.group_by.clone(),
            hover: view.hover.clone(),
            dir: view.sort.dir,
            refused: None,
        }
    }

    /// Put a hover button in, or take it out.
    pub fn toggle_hover(&mut self, kind: OpKind) {
        match self.hover.iter().position(|one| *one == kind) {
            Some(at) => {
                self.hover.remove(at);
            }
            None => self.hover.push(kind),
        }
    }

    /// The view this draft saves as, or why it cannot be one yet.
    pub fn build<Tz: TimeZone>(
        &self,
        labels: &[(String, LabelId)],
        zone: &Tz,
    ) -> Result<View, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Give the view a name: it is what the sidebar calls it.".to_owned());
        }
        let words = self.query.trim();
        let filter = match (words.is_empty(), &self.kept) {
            (false, _) => {
                mail_core::query::parse_with(words, zone, &mail_core::query::named(labels))
            }
            (true, Some(kept)) => kept.clone(),
            // An empty search means "no filter" in the box, and a view of everything is the
            // Archive with extra steps; saying so beats saving a view nobody meant.
            (true, None) => {
                return Err("Say what the view lists, as you would search for it.".to_owned());
            }
        };
        Ok(View {
            id: self.id.unwrap_or_else(ViewId::generate),
            name: name.to_owned(),
            kind: ViewKind::Query,
            filter,
            sort: Sort {
                property: Property::Date,
                dir: self.dir,
            },
            group_by: self.group.clone(),
            threading: Threading::Threaded,
            shown: Vec::new(),
            hover: self.hover.clone(),
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

/// `filter` in the search language, when those words read back as exactly `filter`.
///
/// Written by [`mail_core::rules::condition`] and then read again, so a filter the language cannot
/// say — `All`, a folder, a date that is not a midnight here, a label since renamed — is `None`
/// rather than words that would quietly save as something else.
pub fn written<Tz: TimeZone>(
    filter: &Filter,
    labels: &[(String, LabelId)],
    zone: &Tz,
) -> Option<String> {
    let name = |id: LabelId| {
        labels
            .iter()
            .find(|(_, known)| *known == id)
            .map(|(name, _)| name.clone())
            .unwrap_or_default()
    };
    let words = mail_core::rules::condition(filter, &name);
    let again = mail_core::query::parse_with(&words, zone, &mail_core::query::named(labels));
    (again == *filter).then_some(words)
}

/// The actions a view's rows can be given, in the order the editor offers them.
///
/// Star is not here: every row draws its own star. Mark read stands for the pair, and is drawn
/// as whichever the conversation needs ([`crate::ui::view::hover_in`]).
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

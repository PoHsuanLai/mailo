//! Saved views, as a person makes one: a search that stays.
//!
//! A [`Draft`] is a view being made or changed, as an editor holds it; [`Draft::build`] reads what
//! was typed once, on Save, and says in a value ([`Refusal`]) why it cannot be a view yet. The
//! window words the refusal and draws the editor; nothing here draws.

use chrono::TimeZone;
use mail_domain::{
    Filter, GroupKey, LabelId, Property, Sort, SortDir, Threading, View, ViewId, ViewKind,
};

/// A view being made or changed.
///
/// The filter is words, not a [`Filter`], for the reason the composer's recipients are strings:
/// what is being typed is not a filter yet. It is read once, on Save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
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
    /// What the view's list sorts by, and which way.
    pub property: Property,
    pub dir: SortDir,
    /// Conversations, or a message to a row. Not a field of the sheet: the list's Organise by
    /// Conversation sets it, and an edit keeps what it was.
    pub threading: Threading,
}

/// Why a draft is not a view yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// It has no name: the name is what a list of places calls it.
    NoName,
    /// It says nothing about what it lists, and a view of everything is not what was meant.
    NoQuery,
}

impl Draft {
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
            property: Property::Date,
            dir: SortDir::Desc,
            threading: Threading::Threaded,
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
            property: view.sort.property,
            dir: view.sort.dir,
            threading: view.threading,
        }
    }

    /// The view this draft saves as, or why it cannot be one yet.
    pub fn build<Tz: TimeZone>(
        &self,
        labels: &[(String, LabelId)],
        zone: &Tz,
    ) -> Result<View, Refusal> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(Refusal::NoName);
        }
        let words = self.query.trim();
        let filter = match (words.is_empty(), &self.kept) {
            (false, _) => crate::query::parse_with(words, zone, &crate::query::named(labels)),
            (true, Some(kept)) => kept.clone(),
            // An empty search means "no filter" in the box, and a view of everything is the
            // Archive with extra steps; refusing beats saving a view nobody meant.
            (true, None) => return Err(Refusal::NoQuery),
        };
        Ok(View {
            id: self.id.unwrap_or_else(ViewId::generate),
            name: name.to_owned(),
            kind: ViewKind::Query,
            filter,
            sort: Sort {
                property: self.property,
                dir: self.dir,
            },
            group_by: self.group.clone(),
            threading: self.threading,
            hover: Vec::new(),
        })
    }
}

/// `filter` in the search language, when those words read back as exactly `filter`.
///
/// Written by [`crate::rules::condition`] and then read again, so a filter the language cannot
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
    let words = crate::rules::condition(filter, &name);
    let again = crate::query::parse_with(&words, zone, &crate::query::named(labels));
    (again == *filter).then_some(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn labels() -> Vec<(String, LabelId)> {
        vec![(
            "Travel".to_owned(),
            LabelId::from_uuid(uuid::Uuid::from_u128(7)),
        )]
    }

    fn saved(name: &str, query: &str) -> View {
        let mut draft = Draft::from_search(query);
        draft.name = name.to_owned();
        draft.build(&labels(), &Utc).unwrap()
    }

    #[test]
    fn a_view_saved_from_a_search_lists_what_the_search_box_would() {
        let draft = Draft::from_search("  from:shop.example is:unread  ");
        assert_eq!(
            draft.name, "from:shop.example is:unread",
            "named after the search at first"
        );
        let view = draft.build(&labels(), &Utc).unwrap();
        let typed = crate::query::parse_with(
            "from:shop.example is:unread",
            &Utc,
            &crate::query::named(&labels()),
        );
        assert_eq!(view.filter, typed);
        assert_eq!(view.kind, ViewKind::Query, "a saved search takes no drop");
        assert_eq!(view.sort.property, Property::Date);
        assert_eq!(view.sort.dir, SortDir::Desc);
        assert_eq!(
            view.threading,
            Threading::Threaded,
            "conversations at first"
        );
    }

    #[test]
    fn a_view_keeps_the_order_it_was_saved_in() {
        let mut view = saved("Big", "has:attachment");
        view.sort = Sort {
            property: Property::Size,
            dir: SortDir::Desc,
        };
        let draft = Draft::of(&view, &labels(), &Utc);
        assert_eq!((draft.property, draft.dir), (Property::Size, SortDir::Desc));
        assert_eq!(draft.build(&labels(), &Utc).unwrap().sort, view.sort);
    }

    #[test]
    fn a_view_needs_a_name_and_something_to_list() {
        let mut draft = Draft::from_search("is:starred");
        draft.name = "   ".to_owned();
        assert_eq!(draft.build(&labels(), &Utc), Err(Refusal::NoName));
        draft.name = "Stars".to_owned();
        draft.query = String::new();
        assert_eq!(draft.build(&labels(), &Utc), Err(Refusal::NoQuery));
    }

    #[test]
    fn an_edited_view_keeps_its_id_and_reads_back_as_the_words_it_was_saved_from() {
        let view = saved("Trips", "label:travel -is:read");
        let draft = Draft::of(&view, &labels(), &Utc);
        assert_eq!(draft.id, Some(view.id));
        assert_eq!(draft.query, "label:Travel -is:read");
        assert_eq!(draft.kept, None);
        assert_eq!(
            draft.build(&labels(), &Utc).unwrap(),
            view,
            "saving it again changes nothing"
        );
    }

    #[test]
    fn an_edited_view_keeps_a_message_to_a_row() {
        let mut view = saved("Trips", "label:travel");
        view.threading = Threading::Single;
        let mut draft = Draft::of(&view, &labels(), &Utc);
        draft.name = "Travel".to_owned();
        let again = draft.build(&labels(), &Utc).unwrap();
        assert_eq!(again.threading, Threading::Single, "a rename keeps it");
    }

    #[test]
    fn a_filter_the_search_language_cannot_say_is_kept_as_it_was() {
        let mut view = saved("Everything", "is:starred");
        view.filter = Filter::All;
        let mut draft = Draft::of(&view, &labels(), &Utc);
        assert_eq!(
            draft.query, "",
            "no words that would save as something else"
        );
        assert_eq!(draft.kept, Some(Filter::All));
        draft.name = "All of it".to_owned();
        let again = draft.build(&labels(), &Utc).unwrap();
        assert_eq!(
            again.filter,
            Filter::All,
            "a rename leaves what it lists alone"
        );
        assert_eq!(again.name, "All of it");
    }

    #[test]
    fn written_words_are_only_offered_when_they_read_back_the_same() {
        const CASES: &[&str] = &[
            "from:ada subject:lunch",
            "is:unread -in:spam has:attachment",
            "label:travel",
            "\"due date\"",
        ];
        for typed in CASES {
            let filter = crate::query::parse_with(typed, &Utc, &crate::query::named(&labels()));
            let words = written(&filter, &labels(), &Utc)
                .unwrap_or_else(|| panic!("{typed} is not offered as words"));
            let again = crate::query::parse_with(&words, &Utc, &crate::query::named(&labels()));
            assert_eq!(again, filter, "{typed} -> {words}");
        }
        // A label nothing knows by name any more cannot be written back.
        let gone = Filter::HasLabel(LabelId::from_uuid(uuid::Uuid::from_u128(99)));
        assert_eq!(written(&gone, &labels(), &Utc), None);
    }
}

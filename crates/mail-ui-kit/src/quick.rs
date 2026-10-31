//! The list's quick filter: Unread, Starred and Has Attachments, toggled from a row under the
//! list's header, as Thunderbird's Quick Filter bar and Apple Mail's filter button offer them.
//!
//! Each toggle is a search term the window already reads (`is:unread`, `is:starred`,
//! `has:attachment`), parsed by the same [`mail_core::query`] a typed search is, and the toggles
//! on are joined with `And` onto whatever the list shows: a place, a saved view, or a search. The
//! store pages a filtered list as it pages any other query, and the sidebar's counts never see
//! it.
//!
//! Per window and per visit, as Apple Mail's is: going to another place drops it
//! ([`crate::view::Shell::select`]), and it is written nowhere.

use mail_domain::Filter;
use mail_ui_model::copy;

/// One toggle of the quick filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Unread,
    Starred,
    Attachments,
}

impl Toggle {
    /// Every toggle, in the order the row draws them.
    pub const ALL: [Toggle; 3] = [Toggle::Unread, Toggle::Starred, Toggle::Attachments];

    /// The search term the toggle is: what typing it into the search asks for.
    pub fn term(self) -> &'static str {
        match self {
            Toggle::Unread => "is:unread",
            Toggle::Starred => "is:starred",
            Toggle::Attachments => "has:attachment",
        }
    }

    /// What the toggle keeps: its term, read as a search reads it.
    pub fn filter(self) -> Filter {
        mail_client::query::parse_with(self.term(), &chrono::Utc, &mail_client::query::unnamed)
    }

    /// Its words on the row.
    pub fn label(self) -> &'static str {
        match self {
            Toggle::Unread => copy::controls::FILTER_UNREAD,
            Toggle::Starred => copy::controls::FILTER_STARRED,
            Toggle::Attachments => copy::controls::FILTER_ATTACHMENTS,
        }
    }
}

/// Whether the row of toggles is drawn under the list's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Row {
    #[default]
    Hidden,
    Shown,
}

/// The quick filter: its row, and the toggles on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Quick {
    pub row: Row,
    /// The toggles on, in [`Toggle::ALL`]'s order.
    on: Vec<Toggle>,
}

impl Quick {
    /// Whether `toggle` is on.
    pub fn is_on(&self, toggle: Toggle) -> bool {
        self.on.contains(&toggle)
    }

    /// Whether any toggle is on: the list is narrowed.
    pub fn is_active(&self) -> bool {
        !self.on.is_empty()
    }

    /// The toggles on, in the row's order.
    pub fn on(&self) -> &[Toggle] {
        &self.on
    }

    /// `toggle` switched. A toggle turned on shows the row, so what narrows the list is always
    /// on screen.
    pub fn toggled(&self, toggle: Toggle) -> Self {
        let flip = |one: &Toggle| (*one == toggle) != self.is_on(*one);
        let on: Vec<Toggle> = Toggle::ALL.into_iter().filter(flip).collect();
        let row = if on.contains(&toggle) {
            Row::Shown
        } else {
            self.row
        };
        Self { row, on }
    }

    /// Every toggle off, the row left as it is.
    pub fn cleared(&self) -> Self {
        Self {
            row: self.row,
            on: Vec::new(),
        }
    }

    /// The header's Filter button: the row shown, or hidden with every toggle off, so a hidden
    /// row never narrows the list.
    pub fn flipped(&self) -> Self {
        match self.row {
            Row::Hidden => Self {
                row: Row::Shown,
                on: self.on.clone(),
            },
            Row::Shown => Self::default(),
        }
    }

    /// `filter`, narrowed by the toggles on.
    pub fn narrow(&self, filter: Filter) -> Filter {
        if self.on.is_empty() {
            return filter;
        }
        let mut clauses = vec![filter];
        clauses.extend(self.on.iter().map(|toggle| toggle.filter()));
        Filter::And(clauses)
    }

    /// The toggles on as a narrowing of their own, for a list built from a scope (the search's
    /// candidates). `None` while none is on.
    pub fn filter(&self) -> Option<Filter> {
        match self.on.as_slice() {
            [] => None,
            [one] => Some(one.filter()),
            several => Some(Filter::And(
                several.iter().map(|toggle| toggle.filter()).collect(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::{ReadState, Star};

    /// Each toggle is the term a search reads, and the filter that term is.
    #[test]
    fn each_toggle_is_its_search_term() {
        let cases = [
            (Toggle::Unread, "is:unread", Filter::Read(ReadState::Unread)),
            (
                Toggle::Starred,
                "is:starred",
                Filter::Starred(Star::Starred),
            ),
            (Toggle::Attachments, "has:attachment", Filter::HasAttachment),
        ];
        for (toggle, term, want) in cases {
            assert_eq!(toggle.term(), term, "{toggle:?}");
            assert_eq!(toggle.filter(), want, "{toggle:?}");
        }
    }

    /// The toggles on are joined with `And` onto the place's filter, in the row's order however
    /// they were turned on; none on leaves the place as it is.
    #[test]
    fn the_toggles_on_narrow_the_place() {
        let place = || Filter::Account(mail_domain::testing::account());
        let on = |toggles: &[Toggle]| {
            toggles
                .iter()
                .fold(Quick::default(), |quick, toggle| quick.toggled(*toggle))
        };
        let unread = || Filter::Read(ReadState::Unread);
        let starred = || Filter::Starred(Star::Starred);
        // A case's name, the toggles pressed in turn, and the clauses they add.
        type Case = (&'static str, &'static [Toggle], Option<Vec<Filter>>);
        let cases: [Case; 5] = [
            ("none", &[], None),
            ("unread", &[Toggle::Unread], Some(vec![unread()])),
            (
                "attachments then unread",
                &[Toggle::Attachments, Toggle::Unread],
                Some(vec![unread(), Filter::HasAttachment]),
            ),
            (
                "all three",
                &[Toggle::Starred, Toggle::Attachments, Toggle::Unread],
                Some(vec![unread(), starred(), Filter::HasAttachment]),
            ),
            (
                "unread twice is off",
                &[Toggle::Unread, Toggle::Unread],
                None,
            ),
        ];
        for (name, toggles, want) in cases {
            let quick = on(toggles);
            let place = place();
            let narrowed = quick.narrow(place.clone());
            match &want {
                None => {
                    assert_eq!(narrowed, place, "{name}");
                    assert_eq!(quick.filter(), None, "{name}");
                    assert!(!quick.is_active(), "{name}");
                }
                Some(clauses) => {
                    let mut all = vec![place];
                    all.extend(clauses.iter().cloned());
                    assert_eq!(narrowed, Filter::And(all), "{name}");
                    let alone = match clauses.as_slice() {
                        [one] => one.clone(),
                        _ => Filter::And(clauses.clone()),
                    };
                    assert_eq!(quick.filter(), Some(alone), "{name}");
                }
            }
        }
    }

    /// A toggle turned on shows the row; Clear leaves the row; the Filter button hides the row
    /// only with every toggle off.
    #[test]
    fn the_row_shows_whatever_narrows_the_list() {
        let shown = Quick::default().flipped();
        assert_eq!(shown.row, Row::Shown);
        assert!(!shown.is_active());

        let unread = Quick::default().toggled(Toggle::Unread);
        assert_eq!(unread.row, Row::Shown, "a toggle on with the row hidden");
        assert_eq!(unread.on(), [Toggle::Unread]);

        let cleared = unread.cleared();
        assert_eq!(cleared.row, Row::Shown, "Clear hid the row");
        assert!(!cleared.is_active());

        let hidden = unread.flipped();
        assert_eq!(hidden, Quick::default(), "hiding the row kept a toggle on");
    }
}

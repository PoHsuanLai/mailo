//! The Space's menu as data: what it lists for a Space, and what a pick does to it. Pure, so the
//! menu's rows and every pick are table tests.

use crate::ui::space::{Member, Space, with_member};
use crate::ui::view::SpaceShowing;
use ds::components::menus::item::item::{AfterPick, MenuItem};
use ds::prelude::{Availability, Check, Theme, Word};
use ds::style::space::look::CardAccent;
use porter_core::AccountId;

/// A row of the Space's menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Pick {
    /// Rename…: a field at the pointer.
    Rename,
    /// Colour…: the colour field, stops, grain and presets at the pointer.
    Colour,
    /// Appearance ▸ one of quire's themes.
    Theme(Theme),
    /// Accent inside the card ▸ the Space's colour, or the person's accent.
    Accent(CardAccent),
    /// Accounts ▸ one account, in or out of the Space.
    Account(AccountId),
    /// New Space.
    New,
    /// Delete Space…: a confirmation at the pointer.
    Delete,
}

/// What a pick asks of the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Then {
    /// Open one of the Space's parts at the pointer.
    Open(SpaceShowing),
    /// The Space changed: keep it.
    Kept,
    /// Make a new Space and switch to it.
    NewSpace,
}

/// What the accent rows are called.
fn accent_name(accent: CardAccent) -> &'static str {
    match accent {
        CardAccent::SpaceHue => "Space Colour",
        CardAccent::Chosen => "Your Accent",
    }
}

/// A row that opens `children` beside it.
fn submenu(title: &str, children: Vec<MenuItem<Pick>>) -> MenuItem<Pick> {
    MenuItem::Submenu {
        title: title.to_owned(),
        image: None,
        availability: Availability::Enabled,
        children,
    }
}

fn checked(on: bool) -> Check {
    if on { Check::On } else { Check::Off }
}

/// The menu for `space`, one of `count` Spaces, over `accounts` (each with how the window names
/// it). Delete is left out for the last Space, as a context menu leaves out what cannot be
/// picked; the account rows stay open on a pick, so several can be switched.
pub(in crate::ui) fn rows(
    space: &Space,
    count: usize,
    accounts: &[(AccountId, String)],
) -> Vec<MenuItem<Pick>> {
    let themes = Theme::ALL
        .iter()
        .map(|theme| {
            MenuItem::new(Pick::Theme(*theme), theme.label())
                .with_check(checked(space.look.theme == *theme))
        })
        .collect();
    let accents = [CardAccent::SpaceHue, CardAccent::Chosen]
        .into_iter()
        .map(|accent| {
            MenuItem::new(Pick::Accent(accent), accent_name(accent))
                .with_check(checked(space.look.card_accent == accent))
        })
        .collect();
    let mut out = vec![
        MenuItem::new(Pick::Rename, "Rename\u{2026}"),
        MenuItem::new(Pick::Colour, "Colour\u{2026}"),
        submenu("Appearance", themes),
        submenu("Accent Inside the Card", accents),
    ];
    if !accounts.is_empty() {
        let members = accounts
            .iter()
            .map(|(id, name)| {
                MenuItem::new(Pick::Account(id.clone()), name.clone())
                    .with_check(checked(space.scope.shows(id.clone())))
                    .with_after(AfterPick::KeepOpen)
            })
            .collect();
        out.push(submenu("Accounts", members));
    }
    out.push(MenuItem::Separator);
    out.push(MenuItem::new(Pick::New, "New Space"));
    if count > 1 {
        out.push(MenuItem::new(Pick::Delete, "Delete Space\u{2026}"));
    }
    out
}

/// Do `pick` to `space`, `all` being every account there is, in order.
pub(in crate::ui) fn apply(pick: Pick, space: &mut Space, all: &[AccountId]) -> Then {
    match pick {
        Pick::Rename => Then::Open(SpaceShowing::Rename),
        Pick::Colour => Then::Open(SpaceShowing::Colour),
        Pick::Delete => Then::Open(SpaceShowing::Delete),
        Pick::New => Then::NewSpace,
        Pick::Theme(theme) => {
            space.look.theme = theme;
            Then::Kept
        }
        Pick::Accent(accent) => {
            space.look.card_accent = accent;
            Then::Kept
        }
        Pick::Account(id) => {
            let member = if space.scope.shows(id.clone()) {
                Member::Out
            } else {
                Member::In
            };
            space.scope = with_member(&space.scope, id, member, all);
            Then::Kept
        }
    }
}

#[cfg(test)]
#[path = "pick_tests.rs"]
mod tests;

use super::{Pick, Then, apply, rows};
use crate::ui::space::{Scope, Space};
use crate::ui::view::SpaceShowing;
use ds::components::menus::item::item::MenuItem;
use ds::prelude::{Check, Theme};
use ds::style::space::look::CardAccent;
use mail_domain::id::account_id_from_uuid;
use porter_core::AccountId;

fn id(n: u128) -> AccountId {
    account_id_from_uuid(uuid::Uuid::from_u128(n))
}

fn space(scope: Scope) -> Space {
    Space {
        scope,
        ..Space::default()
    }
}

/// Each row's title, a submenu's children as `title/child`, and `✓` on a checked one.
fn titles(items: &[MenuItem<Pick>]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        match item {
            MenuItem::Item { title, check, .. } => out.push(match check {
                Some(Check::On) => format!("{title} \u{2713}"),
                _ => title.clone(),
            }),
            MenuItem::Submenu {
                title, children, ..
            } => {
                for child in titles(children) {
                    out.push(format!("{title}/{child}"));
                }
            }
            MenuItem::Separator => out.push("-".to_owned()),
            MenuItem::Header(_) | MenuItem::Info { .. } => {}
        }
    }
    out
}

/// A Space, how many there are, the accounts there are, and the rows its menu should list.
type Case<'a> = (&'a Space, usize, &'a [(AccountId, String)], &'a [&'a str]);

#[test]
fn the_menu_lists_the_space_s_parts_and_checks_what_it_has() {
    let accounts = vec![
        (id(1), "a@x.test".to_owned()),
        (id(2), "b@x.test".to_owned()),
    ];
    let mut one = space(Scope::Accounts(vec![id(2)]));
    one.look.theme = Theme::Dark;
    one.look.card_accent = CardAccent::Chosen;
    let cases: &[Case] = &[
        (
            &one,
            2,
            &accounts,
            &[
                "Rename\u{2026}",
                "Colour\u{2026}",
                "Appearance/System",
                "Appearance/Light",
                "Appearance/Dark \u{2713}",
                "Accent Inside the Card/Space Colour",
                "Accent Inside the Card/Your Accent \u{2713}",
                "Accounts/a@x.test",
                "Accounts/b@x.test \u{2713}",
                "-",
                "New Space",
                "Delete Space\u{2026}",
            ],
        ),
        // The last Space cannot be deleted, and no accounts means no Accounts submenu.
        (
            &one,
            1,
            &[],
            &[
                "Rename\u{2026}",
                "Colour\u{2026}",
                "Appearance/System",
                "Appearance/Light",
                "Appearance/Dark \u{2713}",
                "Accent Inside the Card/Space Colour",
                "Accent Inside the Card/Your Accent \u{2713}",
                "-",
                "New Space",
            ],
        ),
    ];
    for (space, count, accounts, want) in cases {
        assert_eq!(titles(&rows(space, *count, accounts)), *want);
    }
}

#[test]
fn a_pick_changes_the_space_or_opens_a_part() {
    let all = [id(1), id(2)];
    let cases: &[(Scope, Pick, Then, Scope)] = &[
        (
            Scope::All,
            Pick::Rename,
            Then::Open(SpaceShowing::Rename),
            Scope::All,
        ),
        (
            Scope::All,
            Pick::Colour,
            Then::Open(SpaceShowing::Colour),
            Scope::All,
        ),
        (
            Scope::All,
            Pick::Delete,
            Then::Open(SpaceShowing::Delete),
            Scope::All,
        ),
        (Scope::All, Pick::New, Then::NewSpace, Scope::All),
        (
            Scope::All,
            Pick::Account(id(1)),
            Then::Kept,
            Scope::Accounts(vec![id(2)]),
        ),
        (
            Scope::Accounts(vec![id(2)]),
            Pick::Account(id(1)),
            Then::Kept,
            Scope::All,
        ),
    ];
    for (scope, pick, then, after) in cases {
        let mut one = space(scope.clone());
        assert_eq!(apply(pick.clone(), &mut one, &all), *then, "{pick:?}");
        assert_eq!(one.scope, *after, "{pick:?}");
    }
    let mut one = space(Scope::All);
    assert_eq!(apply(Pick::Theme(Theme::Light), &mut one, &all), Then::Kept);
    assert_eq!(one.look.theme, Theme::Light);
    assert_eq!(
        apply(Pick::Accent(CardAccent::SpaceHue), &mut one, &all),
        Then::Kept
    );
    assert_eq!(one.look.card_accent, CardAccent::SpaceHue);
}

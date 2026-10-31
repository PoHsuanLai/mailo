//! An account tile's right-click menu: Sync Now; Sign In Again… while the account waits for one;
//! Remove from This Space, which may leave it in no Space; and Account Settings…. The All
//! tile's menu is Sync Now, for every account the Space shows. What the rows are is data
//! ([`items`]); [`TileMenu`] draws them at the press and does the pick.

use crate::ui::fetching::Fetching;
use crate::ui::reauth::Reauth;
use dioxus::prelude::*;
use ds::components::menus::item::item::MenuItem;
use ds::host::measure::Anchor;
use ds::prelude::*;
use ds::root::common::Common;
use mail_ui_kit::space::{Handle, Member, with_member};
use mail_ui_kit::view::Shell;
use mail_ui_model::copy;
use porter_core::AccountId;

/// What a row of the menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pick {
    SyncNow,
    SignInAgain,
    RemoveFromSpace,
    Settings,
}

/// What the menu knows of the tile's account: whether it waits for a sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Standing {
    pub signed_out: bool,
}

/// The rows for a tile: the All tile's (`None`) or an account's.
pub(super) fn items(standing: Option<Standing>) -> Vec<MenuItem<Pick>> {
    let sync = MenuItem::new(Pick::SyncNow, copy::menus::SYNC_NOW)
        .with_image(MenuImage::Icon(Icon::Refresh));
    let Some(standing) = standing else {
        return vec![sync];
    };
    let mut rows = vec![sync];
    if standing.signed_out {
        rows.push(
            MenuItem::new(Pick::SignInAgain, copy::menus::SIGN_IN_AGAIN)
                .with_image(MenuImage::Icon(Icon::Key)),
        );
    }
    rows.push(MenuItem::Separator);
    rows.push(MenuItem::new(
        Pick::RemoveFromSpace,
        copy::menus::REMOVE_FROM_SPACE,
    ));
    rows.push(MenuItem::Separator);
    rows.push(
        MenuItem::new(Pick::Settings, copy::menus::ACCOUNT_SETTINGS)
            .with_image(MenuImage::Icon(Icon::Settings)),
    );
    rows
}

/// The menu a right click on a tile opens, at `at`: `account` is the tile's (`None` for All).
#[component]
pub(super) fn TileMenu(
    shell: Signal<Shell>,
    handle: Handle,
    account: Option<AccountId>,
    at: Point,
    onclose: EventHandler<()>,
) -> Element {
    let standing = account.as_ref().map(|id| {
        let signed_out = try_consume_context::<Fetching>()
            .is_some_and(|fetching| fetching.needs().iter().any(|(need, _)| need == id));
        Standing { signed_out }
    });
    rsx! {
        Menu::<Pick> {
            placement: MenuPlacement::Context,
            anchor: Anchor::Point(at),
            items: items(standing),
            unavailable: ContextUnavailable::Dim,
            common: Common { aria_label: Some(copy::aria::ACCOUNT_MENU.to_owned()), ..Common::default() },
            onpick: move |pick: Pick| {
                onclose.call(());
                picked(pick, shell, handle, account.clone());
            },
            onclose: move |()| onclose.call(()),
        }
    }
}

/// Do what `pick` asks of `account`'s tile (`None` for All).
fn picked(pick: Pick, mut shell: Signal<Shell>, handle: Handle, account: Option<AccountId>) {
    match (pick, account) {
        (Pick::SyncNow, None) => crate::ui::fetching::sync_now(&shell.peek()),
        (Pick::SyncNow, Some(id)) => crate::ui::fetching::sync_account(id),
        (Pick::SignInAgain, Some(id)) => {
            if let Some(reauth) = try_consume_context::<Reauth>() {
                reauth.ask_again(&[id]);
            }
        }
        (Pick::RemoveFromSpace, Some(id)) => {
            let current = handle.spaces().peek().current().id;
            handle.apply(current, |space| {
                space.payload.accounts =
                    with_member(&space.payload.accounts, id.clone(), Member::Out);
            });
            // A tile pressed to narrow the list to it narrows nothing once it is gone.
            if shell.peek().account.as_ref() == Some(&id) {
                shell.write().account = None;
            }
        }
        (Pick::Settings, Some(id)) => crate::ui::settings_window::open_account(id),
        (_, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Pick, Standing, items};
    use ds::components::menus::item::item::MenuItem;
    use ds::prelude::Availability;

    /// Each row's pick and whether it can be picked, a separator as `None`.
    fn rows(standing: Option<Standing>) -> Vec<Option<(Pick, bool)>> {
        items(standing)
            .into_iter()
            .map(|item| match item {
                MenuItem::Item {
                    value,
                    availability,
                    ..
                } => Some((value, availability == Availability::Enabled)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_tile_s_menu_offers_what_its_account_can_do() {
        use Pick::*;
        let cases = [
            (None, vec![Some((SyncNow, true))]),
            (
                Some(Standing { signed_out: false }),
                vec![
                    Some((SyncNow, true)),
                    None,
                    Some((RemoveFromSpace, true)),
                    None,
                    Some((Settings, true)),
                ],
            ),
            (
                Some(Standing { signed_out: true }),
                vec![
                    Some((SyncNow, true)),
                    Some((SignInAgain, true)),
                    None,
                    Some((RemoveFromSpace, true)),
                    None,
                    Some((Settings, true)),
                ],
            ),
        ];
        for (standing, want) in cases {
            assert_eq!(rows(standing), want, "{standing:?}");
        }
    }
}

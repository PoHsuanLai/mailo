//! The account tiles at the top of the sidebar: one per account the Space shows, with its unread
//! count and its provider's mark, and "All" before them when there are several.
//!
//! A press on a tile narrows the list to that account inside the Space (`Shell::account`); All
//! is every account the Space shows. Each tile carries its account's trouble as quire's warning
//! mark, its status line's words as the mark's tip; pressing the mark asks again about a sign-in
//! put off, or opens the account's settings. The "+" after them brings in an account the Space
//! does not show yet, or adds a new one ([`super::join`]). A right click on a tile opens its
//! menu ([`super::tile_menu`]).

use super::join::{self, Plus};
use super::tile_menu::TileMenu;
use crate::ui::copy;
use crate::ui::data::account_rows;
use crate::ui::fetching::{Fetching, StatusLine, Tone};
use crate::ui::menu::Floating;
use crate::ui::provider_chip::{MarkAt, mark_of, mark_style};
use crate::ui::reauth::Reauth;
use crate::ui::space::{Handle, Space};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::app::pin_tile::{PinFace, PinStatus};
use ds::components::app::pin_tiles::{PinAdd, PinItem, PinMenu, PinTiles};
use ds::components::content::provider_mark::{MarkProvider, MarkStyle};
use ds::host::measure::MountedRef;
use ds::prelude::Point;
use ds::root::common::Common;
use mail_client::query::{Account, Counted, Counts};
use mail_core::SqliteStore;
use mail_core::profile::ProfileId;
use mail_core::provider::{Provider, provider};
use mail_domain::{Filter, ReadState};
use porter_core::AccountId;
use std::sync::Arc;

/// One account's tile, as the store has it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct TileRow {
    pub id: AccountId,
    /// What the tile is called: the address, or "Local folders".
    pub name: String,
    pub unread: u64,
    /// Its provider; none for local folders, which are on no provider.
    pub via: Option<Provider>,
    /// Whether the person paused its syncing, which its mark says while nothing worse does.
    pub paused: bool,
}

/// What a tile's count counts: its account's unread.
pub(super) fn tile_filter(account: AccountId) -> Filter {
    Filter::And(vec![
        Filter::Read(ReadState::Unread),
        Filter::Account(account),
    ])
}

/// The tiles for `space`: each account it shows, in the order they were put in it, that the
/// store still has, with the unread `counted` has for it (none until counted).
pub(super) fn tile_rows(
    accounts: &[Account],
    counted: Option<&Counted>,
    space: &Space,
) -> Vec<TileRow> {
    space
        .payload
        .accounts
        .iter()
        .filter_map(|id| accounts.iter().find(|account| account.id == *id))
        .map(|account| TileRow {
            id: account.id.clone(),
            name: crate::ui::data::named(account),
            unread: counted
                .and_then(|counted| Counts::of(counted, &tile_filter(account.id.clone())))
                .unwrap_or(0),
            via: if account.local {
                None
            } else {
                account.plan.as_ref().map(provider)
            },
            paused: account.paused,
        })
        .collect()
}

/// What a tile's mark says of its account's status line: nothing while all is well or busy (a
/// sync shows as the foot's turning arrows), else the warning, with the line as its words. An
/// account the person paused says so while nothing worse is wrong, and its mark, like any
/// other, opens the account's page, where the switch is.
pub(super) fn status_of(line: Option<&StatusLine>, paused: bool) -> PinStatus {
    match line {
        Some(line) if line.tone != Tone::Plain => PinStatus::Attention {
            why: line.text.clone(),
        },
        _ if paused => PinStatus::Attention {
            why: copy::status::account::PAUSED.to_owned(),
        },
        _ => PinStatus::Quiet,
    }
}

/// What the "All" tile says: one account in trouble is that account's words; several are
/// counted.
pub(super) fn all_status(statuses: &[PinStatus]) -> PinStatus {
    let troubled: Vec<&String> = statuses
        .iter()
        .filter_map(|status| match status {
            PinStatus::Attention { why } => Some(why),
            PinStatus::Quiet | PinStatus::Busy(_) => None,
        })
        .collect();
    match troubled.as_slice() {
        [] => PinStatus::Quiet,
        [one] => PinStatus::Attention {
            why: (*one).clone(),
        },
        many => PinStatus::Attention {
            why: copy::status::accounts_need_attention(many.len()),
        },
    }
}

/// An unread count as a tile's badge holds it.
fn count_of(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// An account's colour: quire's person hue for its name, the one its avatar has everywhere.
fn colour_of(name: &str) -> String {
    crate::ui::command::avatar_color(name)
}

#[component]
pub(super) fn AccountTiles(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    handle: Handle,
    rows: Vec<TileRow>,
) -> Element {
    // The "+" menu while it is open: the accounts it offers, hung from the row of tiles.
    let mut joining = use_signal(|| None::<Vec<(AccountId, String)>>);
    // The add menu while it is open: the providers that sign in straight in the browser.
    let mut adding = use_signal(|| None::<Vec<(porter_provider::Issuer, String)>>);
    let mut row_at = use_signal(|| None::<MountedRef>);
    // A tile's right-click menu while it is open: which tile (`None` for All), and where.
    let mut menu = use_signal(|| None::<(Option<AccountId>, Point)>);
    let several = rows.len() > 1;
    let marks = shell.read().appearance.marks;
    let fetching = try_use_context::<Fetching>();
    let now = crate::ui::clock::now();
    let statuses: Vec<PinStatus> = rows
        .iter()
        .map(|row| {
            let line = fetching.and_then(|fetching| fetching.account_line(&row.id, now));
            status_of(line.as_ref(), row.paused)
        })
        .collect();
    let mut items: Vec<PinItem<Option<AccountId>>> = Vec::new();
    if several {
        let all: u64 = rows.iter().map(|row| row.unread).sum();
        items.push(
            PinItem::new(None, PinFace::All)
                .unread(count_of(all))
                .mark(MarkStyle::Letter)
                .status(all_status(&statuses)),
        );
    }
    for (row, status) in rows.iter().zip(statuses) {
        // Local folders are on no provider: quire's neutral folder mark.
        let (provider, mark) = match row.via {
            Some(via) => (mark_of(via), mark_style(via, marks, MarkAt::Tile)),
            None => (MarkProvider::Local, MarkStyle::Letter),
        };
        let face = PinFace::Account {
            initial: super::panes::initial(&row.name),
            colour: super::panes::hex_colour(&colour_of(&row.name)),
            provider,
            address: Some(row.name.clone()),
        };
        items.push(
            PinItem::new(Some(row.id.clone()), face)
                .unread(count_of(row.unread))
                .mark(mark)
                .status(status),
        );
    }
    let selected = if several {
        shell.read().account.clone()
    } else {
        rows.first().map(|row| row.id.clone())
    };
    let shown: Vec<AccountId> = rows.iter().map(|row| row.id.clone()).collect();
    let in_tiles = shown.clone();
    rsx! {
        PinTiles::<Option<AccountId>> {
            label: copy::aria::ACCOUNTS_IN_SPACE,
            items,
            selected: Some(selected),
            add: PinAdd {
                label: copy::aria::ADD_ACCOUNT.to_owned(),
                hint: Some(copy::tooltips::ADD_ACCOUNT.to_owned()),
                onadd: EventHandler::new(move |()| {
                    let store = consume_context::<Arc<SqliteStore>>();
                    let all: Vec<(AccountId, String)> = account_rows(&store)
                        .into_iter()
                        .map(|row| (row.id.clone(), row.shown()))
                        .collect();
                    match join::plus(&shown, &all) {
                        Plus::AddNew => crate::ui::add_account::add_from(crate::ui::add_account::Join::OnScreen, |offered| {
                            adding.set(Some(offered))
                        }),
                        Plus::Offer(outside) => joining.set(Some(outside)),
                    }
                }),
            },
            common: Common {
                mounted: Some(EventHandler::new(move |event: MountedEvent| {
                    row_at.set(Some(MountedRef(event.data())));
                })),
                ..Common::default()
            },
            onmenu: Some(EventHandler::new(move |asked: PinMenu<Option<AccountId>>| {
                menu.set(Some((asked.key, asked.at)));
            })),
            onpick: move |account: Option<AccountId>| {
                if several && shell.peek().account != account {
                    shell.write().account = account;
                    pages.set(1);
                }
            },
            // The mark is a button: an account waiting for a sign-in that was put off is asked
            // about again; any other trouble is the account's settings'.
            onstatus: move |account: Option<AccountId>| {
                let asked: Vec<AccountId> = match &account {
                    Some(id) => vec![id.clone()],
                    None => in_tiles.clone(),
                };
                let needing: Vec<AccountId> = try_consume_context::<Fetching>()
                    .map(|fetching| fetching.needs())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(id, _)| id)
                    .filter(|id| asked.contains(id))
                    .collect();
                let reasked = try_consume_context::<Reauth>()
                    .is_some_and(|reauth| reauth.ask_again(&needing));
                if !reasked {
                    match account {
                        Some(id) => crate::ui::settings_window::open_account(id),
                        None => crate::ui::settings_window::open(),
                    }
                }
            },
        }
        if let Some((account, at)) = menu() {
            TileMenu { shell, handle, account, at, onclose: move |()| menu.set(None) }
        }
        if let Some(outside) = joining() {
            Floating {
                anchor: row_at(),
                title: copy::titles::ADD_TO_SPACE.to_owned(),
                items: join::items(&outside, colour_of),
                on_pick: move |key: String| {
                    joining.set(None);
                    match join::picked(&key, &outside) {
                        Some(account) => bring_in(handle, account),
                        None => crate::ui::add_account::add_from(crate::ui::add_account::Join::OnScreen, |offered| {
                            adding.set(Some(offered))
                        }),
                    }
                },
                on_close: move |_| joining.set(None),
            }
        }
        if let Some(redirects) = adding() {
            crate::ui::add_account::AddMenu {
                anchor: row_at(),
                redirects,
                join: crate::ui::add_account::Join::OnScreen,
                on_close: move |()| adding.set(None),
            }
        }
    }
}

/// Put `account` in the Space on screen, and write the Spaces.
fn bring_in(handle: Handle, account: AccountId) {
    let current = handle.spaces().peek().current().id;
    handle.apply(current, |space| {
        space.payload.widen(&ProfileId::default(), account);
    });
}

#[cfg(test)]
mod tests {
    use super::{all_status, status_of};
    use crate::ui::copy;
    use crate::ui::fetching::{StatusLine, Tone};
    use ds::components::app::pin_tile::PinStatus;

    fn line(text: &str, tone: Tone) -> StatusLine {
        StatusLine {
            text: text.to_owned(),
            tip: text.to_owned(),
            progress: None,
            tone,
        }
    }

    fn attention(why: &str) -> PinStatus {
        PinStatus::Attention {
            why: why.to_owned(),
        }
    }

    #[test]
    fn a_tile_carries_its_account_s_trouble_and_nothing_else() {
        // (the account's status line, the tile's mark)
        let cases = [
            (None, PinStatus::Quiet),
            (
                Some(line("Updated just now", Tone::Plain)),
                PinStatus::Quiet,
            ),
            (
                Some(line("Downloading 3 of 40", Tone::Plain)),
                PinStatus::Quiet,
            ),
            (
                Some(line("Not updated since 14:05", Tone::Warn)),
                attention("Not updated since 14:05"),
            ),
            (
                Some(line("Sign in again to keep receiving mail.", Tone::Danger)),
                attention("Sign in again to keep receiving mail."),
            ),
        ];
        for (line, want) in cases {
            assert_eq!(status_of(line.as_ref(), false), want, "{line:?}");
        }
    }

    #[test]
    fn a_paused_tile_says_so_unless_something_worse_is_wrong() {
        let paused = attention(copy::status::account::PAUSED);
        assert_eq!(status_of(None, true), paused);
        assert_eq!(
            status_of(Some(&line("Updated just now", Tone::Plain)), true),
            paused
        );
        assert_eq!(
            status_of(Some(&line("Sign in again.", Tone::Danger)), true),
            attention("Sign in again.")
        );
    }

    /// A pressed tile narrows the list to its account inside the Space; one whose account the
    /// Space no longer shows narrows nothing.
    #[test]
    fn a_pressed_tile_wins_over_the_space_only_while_the_space_shows_it() {
        use crate::ui::space::Scope;
        use porter_core::AccountId;
        let account = |n: u128| -> AccountId {
            mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(n))
        };
        let space = Scope::Accounts(vec![account(1), account(2)]);
        // (the pressed tile, the accounts listed)
        let cases = [
            (None, space.clone()),
            (Some(account(2)), Scope::Accounts(vec![account(2)])),
            (Some(account(3)), space.clone()),
        ];
        for (pressed, want) in cases {
            let shell = crate::ui::view::Shell {
                scope: space.clone(),
                account: pressed.clone(),
                ..crate::ui::view::Shell::default()
            };
            assert_eq!(shell.listed_scope(), want, "{pressed:?}");
        }
    }

    #[test]
    fn the_all_tile_speaks_for_the_accounts_in_the_space() {
        let quiet = PinStatus::Quiet;
        // (each tile's mark, the All tile's)
        let cases = [
            (vec![quiet.clone(), quiet.clone()], PinStatus::Quiet),
            (
                vec![quiet.clone(), attention("Offline.")],
                attention("Offline."),
            ),
            (
                vec![attention("Offline."), attention("Sign in again.")],
                attention("2 accounts need attention."),
            ),
        ];
        for (statuses, want) in cases {
            assert_eq!(all_status(&statuses), want, "{statuses:?}");
        }
    }
}

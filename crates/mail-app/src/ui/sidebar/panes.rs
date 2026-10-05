//! Account tiles, places, labels and pinned people.

use super::super::data::account_rows;
use super::super::motion::{drag, motion};
use super::join::{self, Plus};
use super::tagged;
use crate::ui::fetching::{Fetching, Mark, account_mark_local};
use crate::ui::menu::Floating;
use crate::ui::provider_chip::{mark_of, mark_style};
use crate::ui::space::{self, Pinned, Space, Spaces};
use crate::ui::view::{Shell, Source, folder_of, is_label_place, saved_of};
use dioxus::prelude::*;
use ds::base::vocab::RowState;
use ds::components::app::pin_tile::{PinFace, PinStatus};
use ds::components::app::pin_tiles::{PinAdd, PinItem, PinTiles};
use ds::components::content::avatar::{AvatarFace, AvatarShape, AvatarSize, AvatarTone};
use ds::components::content::provider_mark::{MarkProvider, MarkStyle};
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::hex::{Colour, Hex};
use ds::style::tokens::person::PersonSwatch;
use mail_core::provider::{Provider, provider};
use mail_core::query::{self};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Counts {
    /// Each account's id, its name as the tile says it, its unread count, and its provider —
    /// none for local folders, which are on no provider.
    pub rows: Vec<(AccountId, String, u64, Option<Provider>)>,
    pub all: u64,
    pub pins: Vec<u64>,
}

pub(super) fn counts(store: &SqliteStore, space: &Space) -> Counts {
    let rows = account_rows(store);
    let shown: Vec<_> = rows
        .into_iter()
        .filter(|row| space.scope.shows(row.id))
        .collect();
    let labels = query::known_labels(store);
    let scope = scope_filter(space);
    let now = chrono::Utc::now();
    let unread = |extra: Filter| {
        let mut parts = vec![Filter::Read(ReadState::Unread), extra];
        if let Some(scope) = scope.clone() {
            parts.push(scope);
        }
        store.count(&Filter::And(parts), now).unwrap_or(0)
    };
    let all = match &scope {
        Some(scope) => store
            .count(
                &Filter::And(vec![Filter::Read(ReadState::Unread), scope.clone()]),
                now,
            )
            .unwrap_or(0),
        None => store
            .count(&Filter::Read(ReadState::Unread), now)
            .unwrap_or(0),
    };
    let rows = shown
        .iter()
        .map(|row| {
            let n = store
                .count(
                    &Filter::And(vec![
                        Filter::Read(ReadState::Unread),
                        Filter::Account(row.id),
                    ]),
                    now,
                )
                .unwrap_or(0);
            let via = (!row.is_local()).then(|| provider(&row.plan));
            (row.id, row.shown(), n, via)
        })
        .collect();
    let pins = space
        .pins
        .iter()
        .map(|pin| unread(pin_filter(pin, &labels)))
        .collect();
    Counts { rows, all, pins }
}

fn scope_filter(space: &Space) -> Option<Filter> {
    space.scope.filter()
}

fn pin_filter(pin: &Pinned, labels: &[(String, LabelId)]) -> Filter {
    match pin {
        Pinned::Person { email, .. } => Filter::From(TextMatch::Contains(email.clone())),
        Pinned::Search { query, .. } => {
            query::parse_with(query, &chrono::Local, &query::named(labels))
        }
    }
}

fn initial(text: &str) -> char {
    super::today::initial(text)
}

/// A stored `#rrggbb` as quire's colour. A Space file hand-edited to something else draws in
/// the first swatch rather than failing.
pub(in crate::ui) fn hex_colour(text: &str) -> Colour {
    Colour::Solid(Hex::parse(text).unwrap_or_else(|| PersonSwatch::nth(0).hex()))
}

fn place_icon(name: &str) -> Icon {
    match name {
        "Inbox" => Icon::Inbox,
        "Starred" => Icon::Star,
        "Snoozed" => Icon::Clock,
        "Archive" => Icon::Archive,
        "Trash" => Icon::Trash,
        "Drafts" => Icon::FilePen,
        "Sent" => Icon::Send,
        "Spam" => Icon::OctagonAlert,
        "Pinned" => Icon::Pin,
        "Waiting" => Icon::Bell,
        _ => Icon::Tag,
    }
}

#[component]
pub(super) fn AccountTiles(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    spaces: Signal<Spaces>,
    space: Space,
    counted: Counts,
) -> Element {
    // The "+" menu while it is open: the accounts it offers, hung from the row of tiles.
    let mut joining = use_signal(|| None::<Vec<(AccountId, String)>>);
    let mut row_at = use_signal(|| None::<MountedRef>);
    let colors = space.clone();
    let several = counted.rows.len() > 1;
    let marks = shell.read().appearance.marks;
    // One tile per account (and "All" before them when there are several), each with its own
    // provider's mark: quire's `PinTiles`, keyed by the account, `None` being all of them.
    let mut items: Vec<PinItem<Option<AccountId>>> = Vec::new();
    // Each tile carries its account's mark, as Mail's sidebar carries it beside the account:
    // the spinner while it syncs, the warning when it needs the person.
    let statuses: Vec<PinStatus> = counted.rows.iter().map(|row| status_of(row.0)).collect();
    if several {
        items.push(
            PinItem::new(None, PinFace::All)
                .unread(count_of(counted.all))
                .mark(MarkStyle::Letter)
                .status(all_status(&statuses)),
        );
    }
    for (index, row) in counted.rows.iter().enumerate() {
        let (id, address, unread, via) = (row.0, row.1.clone(), row.2, row.3);
        // Local folders are on no provider: quire's neutral folder mark.
        let (provider, mark) = match via {
            Some(via) => (mark_of(via), mark_style(via, marks)),
            None => (MarkProvider::Local, MarkStyle::Letter),
        };
        let face = PinFace::Account {
            initial: initial(&address),
            colour: hex_colour(&space::avatar_color(&space, id, index)),
            provider,
            address: Some(address),
        };
        items.push(
            PinItem::new(Some(id), face)
                .unread(count_of(unread))
                .mark(mark)
                .status(statuses.get(index).cloned().unwrap_or_default()),
        );
    }
    let selected = if several {
        shell.read().account
    } else {
        counted.rows.first().map(|row| row.0)
    };
    rsx! {
        PinTiles::<Option<AccountId>> {
            label: "Accounts in this Space",
            items,
            selected: Some(selected),
            add: PinAdd {
                label: "Add account".to_owned(),
                hint: Some("Add account\u{2026}".to_owned()),
                onadd: EventHandler::new(move |()| {
                    let store = consume_context::<std::sync::Arc<SqliteStore>>();
                    let all: Vec<(AccountId, String)> = account_rows(&store)
                        .into_iter()
                        .map(|row| (row.id, row.shown()))
                        .collect();
                    match join::plus(&spaces.peek().current_space(), &all) {
                        Plus::AddNew => super::super::add_account::open(shell),
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
            onpick: move |account: Option<AccountId>| {
                shell.write().account = account;
                pages.set(1);
            },
            // The mark is a button: it opens the Connection Doctor, which lists every account.
            onstatus: move |_: Option<AccountId>| super::super::doctor::open(shell),
        }
        if let Some(outside) = joining() {
            Floating {
                anchor: row_at(),
                title: "Add to this Space".to_owned(),
                items: {
                    // The swatch an account takes is by its place among every account, as its
                    // tile's is: two uncoloured accounts do not share one.
                    let store = consume_context::<std::sync::Arc<SqliteStore>>();
                    let all: Vec<AccountId> =
                        account_rows(&store).into_iter().map(|row| row.id).collect();
                    join::items(&outside, |id| {
                        let index = all.iter().position(|each| *each == id).unwrap_or(0);
                        space::avatar_color(&colors, id, index)
                    })
                },
                on_pick: move |key: String| {
                    joining.set(None);
                    match join::picked(&key, &outside) {
                        Some(account) => bring_in(shell, spaces, account),
                        None => super::super::add_account::open(shell),
                    }
                },
                on_close: move |_| joining.set(None),
            }
        }
    }
}

/// Put `account` in the current Space's scope, write the Spaces, and show it.
fn bring_in(mut shell: Signal<Shell>, mut spaces: Signal<Spaces>, account: AccountId) {
    let widened = {
        let mut all = spaces.write();
        let current = all.current;
        all.spaces
            .get_mut(current)
            .is_some_and(|space| super::super::add_account::flow::widen(space, account))
    };
    if widened {
        super::super::frame::keep(&spaces.read());
        shell.write().scope = spaces.read().current_space().scope;
    }
}

/// What an account's tile says of its fetching.
fn status_of(account: AccountId) -> PinStatus {
    let Some(fetching) = try_consume_context::<Fetching>() else {
        return PinStatus::Quiet;
    };
    match fetching.link(account).map(|link| account_mark_local(&link)) {
        None | Some(Mark::Quiet) => PinStatus::Quiet,
        Some(Mark::Busy) => PinStatus::Busy(fetching.op(account)),
        Some(Mark::Warn(why) | Mark::Offline(why)) => PinStatus::Attention { why },
    }
}

/// What the "All" tile says: an account that needs the person first, else any that is working.
pub(super) fn all_status(statuses: &[PinStatus]) -> PinStatus {
    let troubled: Vec<&String> = statuses
        .iter()
        .filter_map(|status| match status {
            PinStatus::Attention { why } => Some(why),
            PinStatus::Quiet | PinStatus::Busy(_) => None,
        })
        .collect();
    match troubled.as_slice() {
        [] => statuses
            .iter()
            .find(|status| matches!(status, PinStatus::Busy(_)))
            .cloned()
            .unwrap_or_default(),
        [one] => PinStatus::Attention {
            why: (*one).clone(),
        },
        many => PinStatus::Attention {
            why: format!("{} accounts need attention.", many.len()),
        },
    }
}

/// An unread count as a tile's badge holds it.
fn count_of(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A place list's keys: its section headings and its places by index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PlaceKey {
    Head(&'static str),
    Place(usize),
}

#[component]
pub(super) fn PlaceList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
    folded: Vec<LabelId>,
) -> Element {
    let places = shell.read().places.clone();
    // Labels, folders and then saved views follow the default places; folders are drawn under
    // Folders.
    let split = places
        .iter()
        .position(|place| {
            is_label_place(place) || folder_of(place).is_some() || saved_of(place).is_some()
        })
        .unwrap_or(places.len());
    let views: Vec<(usize, String)> = places
        .iter()
        .enumerate()
        .filter(|(_, place)| saved_of(place).is_some())
        .map(|(index, place)| (index, place.name.clone()))
        .collect();
    // A label that is also a mailbox is drawn once, under Folders. See `folder_tree::arrange`.
    let labels: Vec<(usize, String)> = places
        .iter()
        .enumerate()
        .skip(split)
        .filter(|(_, place)| match &place.source {
            Source::Mail(Filter::HasLabel(id)) => !folded.contains(id),
            _ => false,
        })
        .map(|(index, place)| (index, place.name.clone()))
        .collect();
    let selected = shell.read().selected;
    let listed = |index: usize| {
        index < split
            || labels.iter().any(|(at, _)| *at == index)
            || views.iter().any(|(at, _)| *at == index)
    };
    let mut items = vec![ListItem::heading(
        PlaceKey::Head("Places"),
        rsx! { SectionHeader { title: "Places" } },
    )];
    for (index, place) in places.iter().take(split).enumerate() {
        items.push(place_item(
            index,
            &place.name,
            place_icon(&place.name),
            shell,
            pages,
            badges,
        ));
    }
    if !labels.is_empty() {
        items.push(ListItem::heading(
            PlaceKey::Head("Labels"),
            rsx! { SectionHeader { title: "Labels" } },
        ));
        for (index, name) in &labels {
            items.push(place_item(*index, name, Icon::Tag, shell, pages, badges));
        }
    }
    if !views.is_empty() {
        items.push(ListItem::heading(
            PlaceKey::Head("Views"),
            rsx! { SectionHeader { title: "Views" } },
        ));
        for (index, name) in &views {
            items.push(place_item(*index, name, Icon::Search, shell, pages, badges));
        }
    }
    rsx! {
        List::<PlaceKey> {
            label: "Places",
            items,
            style: ListStyle::SourceList,
            cursor: (listed(selected) && !shell.read().from_today).then_some(PlaceKey::Place(selected)),
            onselect: move |key: PlaceKey| {
                if let PlaceKey::Place(index) = key {
                    shell.write().select(index);
                    pages.set(1);
                }
            },
        }
    }
}

/// One place's row: quire's `Row` in the source list, its unread count a badge, outlined while
/// a dragged row could land on it and lit under the pointer.
fn place_item(
    index: usize,
    name: &str,
    icon: Icon,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
) -> ListItem<PlaceKey> {
    let content = rsx! {
        PlaceRow { index, name: name.to_owned(), icon, shell, pages, badges }
    };
    ListItem::row(PlaceKey::Place(index), name, content)
}

#[component]
fn PlaceRow(
    index: usize,
    name: String,
    icon: Icon,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    badges: Memo<Vec<Option<u64>>>,
) -> Element {
    let on = shell.read().place_selected(index);
    let count = badges().get(index).copied().flatten();
    let state = use_hook(motion);
    let accepts = shell.read().places.get(index).is_some_and(drag::accepts);
    let drop = state.map_or(DropState::Idle, |state| drop_of(&state, index, accepts));
    rsx! {
        Row {
            leading: RowLeading::Icon(icon),
            title: name.clone(),
            common: tagged("place", name),
            accessory: count.map_or(Accessory::None, |count| Accessory::Badge(count_of(count))),
            state: RowState {
                selection: if on { Selection::Selected } else { Selection::Unselected },
                drop,
                ..RowState::default()
            },
            onclick: move |_| {
                shell.write().select(index);
                pages.set(1);
            },
            onpointerenter: move |_| {
                if accepts {
                    drag::over(Some(index), index);
                }
            },
            onpointerleave: move |_| drag::over(None, index),
        }
    }
}

/// A place's part in a drag: outlined while a dragged row could land on it, lit under the
/// pointer.
fn drop_of(state: &super::super::motion::Motion, index: usize, accepts: bool) -> DropState {
    match &*state.drag.read() {
        drag::Drag::Live { target, .. } if accepts => {
            if *target == Some(index) {
                DropState::Target
            } else {
                DropState::Accepts
            }
        }
        _ => DropState::Idle,
    }
}

#[component]
pub(super) fn PinnedList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    space: Space,
    pins: Vec<u64>,
) -> Element {
    // No pins, no heading: a source list does not show an empty group.
    if space.pins.is_empty() {
        return rsx! {};
    }
    let mut items = vec![ListItem::heading(
        "head".to_owned(),
        rsx! { SectionHeader { title: "Pinned" } },
    )];
    for (index, pin) in space.pins.iter().enumerate() {
        let name = match pin {
            Pinned::Person { name, .. } | Pinned::Search { name, .. } => name.clone(),
        };
        let query = match pin {
            Pinned::Person { email, .. } => format!("from:{email}"),
            Pinned::Search { query, .. } => query.clone(),
        };
        let avatar = AvatarFace {
            initial: initial(&name),
            size: AvatarSize::Size16,
            tone: AvatarTone::Account(PersonSwatch::nth(index + 3).colour()),
            shape: AvatarShape::Square,
        };
        let n = pins.get(index).copied().unwrap_or(0);
        let content = rsx! {
            Row {
                leading: RowLeading::Avatar(avatar),
                title: name.clone(),
                accessory: if n > 0 { Accessory::Badge(count_of(n)) } else { Accessory::None },
                onclick: move |_| {
                    shell.write().search = query.clone();
                    pages.set(1);
                },
            }
        };
        items.push(ListItem::row(format!("pin-{index}-{name}"), name, content));
    }
    rsx! {
        List::<String> {
            label: "Pinned",
            items,
            style: ListStyle::SourceList,
        }
    }
}

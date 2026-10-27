//! Favourites, the places and labels.

use super::super::motion::{drag, motion};
use super::lit::{Landing, Under, lit};
use super::tagged;
use crate::ui::copy;
use crate::ui::space::{Pinned, Space};
use crate::ui::text::sender;
use crate::ui::view::{Shell, SideList, Source, folder_of, is_label_place, saved_of};
use dioxus::prelude::*;
use ds::base::vocab::{Emphasis, RowState};
use ds::components::content::avatar::{AvatarFace, AvatarShape, AvatarSize, AvatarTone};
use ds::components::lists::list::model::{ListItem, ListStyle};
use ds::prelude::*;
use ds::style::tokens::hex::{Colour, Hex};
use ds::style::tokens::person::PersonSwatch;
use mail_core::mailbox_filter::{for_role, pending_snooze};
use mail_core::query::{self};
use mail_core::{SqliteStore, Store};
use mail_domain::*;

/// The most pinned conversations Favourites shows: the most recently pinned. Every pinned
/// conversation is in Pinned.
pub(super) const FAVOURITES_SHOWN: u32 = 8;

/// A pinned conversation as Favourites draws it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Favourite {
    pub thread: ThreadId,
    pub title: String,
    pub initial: char,
    pub unread: bool,
}

#[derive(Clone, PartialEq)]
pub(super) struct Counts {
    /// The Space's pinned people and searches: each one's unread count.
    pub pins: Vec<u64>,
    /// The pinned conversations in the Space's accounts, most recently pinned first.
    pub pinned: Vec<Favourite>,
}

pub(super) fn counts(store: &SqliteStore, space: &Space) -> Counts {
    let labels = query::known_labels(store);
    let scope = space.payload.scope().filter();
    let now = chrono::Utc::now();
    let unread = |extra: Filter| {
        let mut parts = vec![Filter::Read(ReadState::Unread), extra];
        if let Some(scope) = scope.clone() {
            parts.push(scope);
        }
        store.count(&Filter::And(parts), now).unwrap_or(0)
    };
    let pins = space
        .payload
        .pins
        .iter()
        .map(|pin| unread(pin_filter(pin, &labels)))
        .collect();
    let pinned = store
        .threads(
            &Query {
                filter: mail_core::conversation::scoped(scope.clone(), Filter::Pinned),
                sort: Sort {
                    property: Property::Pin,
                    dir: SortDir::Desc,
                },
                page: PageReq {
                    after: None,
                    limit: FAVOURITES_SHOWN,
                },
            },
            now,
        )
        .map(|page| page.items)
        .unwrap_or_default()
        .into_iter()
        .map(|summary| Favourite {
            thread: summary.id,
            title: titled(&summary.subject),
            initial: initial(&sender(&summary)),
            unread: summary.read == ReadState::Unread,
        })
        .collect();
    Counts { pins, pinned }
}

/// A conversation's subject as a sidebar row says it.
pub(super) fn titled(subject: &str) -> String {
    mail_domain::subject_or_none(subject).into_owned()
}

fn pin_filter(pin: &Pinned, labels: &[(String, LabelId)]) -> Filter {
    match pin {
        Pinned::Person { email, .. } => Filter::From(TextMatch::Contains(email.clone())),
        Pinned::Search { query, .. } => {
            query::parse_with(query, &chrono::Local, &query::named(labels))
        }
    }
}

/// The first character of `name`, upper-cased, or `?` for an empty name.
pub(super) fn initial(name: &str) -> char {
    name.chars()
        .next()
        .and_then(|ch| ch.to_uppercase().next())
        .unwrap_or('?')
}

/// A sidebar row's face: one letter on the `index`th swatch, the sidebar's rounded square.
pub(super) fn face(initial: char, index: usize) -> AvatarFace {
    AvatarFace {
        initial,
        size: AvatarSize::Size16,
        tone: AvatarTone::Account(PersonSwatch::nth(index).colour()),
        shape: AvatarShape::Square,
    }
}

/// A stored `#rrggbb` as quire's colour. A Space file hand-edited to something else draws in
/// the first swatch rather than failing.
pub(in crate::ui) fn hex_colour(text: &str) -> Colour {
    Colour::Solid(Hex::parse(text).unwrap_or_else(|| PersonSwatch::nth(0).hex()))
}

/// A built-in place's glyph, found by what the place lists rather than by what it is called: the
/// sidebar's and ⌘K's.
pub(in crate::ui) fn place_icon(source: &Source) -> Icon {
    let role = |role: MailboxRole| *source == Source::Mail(for_role(role));
    match source {
        Source::Drafts => Icon::FilePen,
        Source::Waiting => Icon::Bell,
        Source::History => Icon::RotateLeft,
        Source::Scheduled => Icon::Send,
        Source::Mail(Filter::Starred(Star::Starred)) => Icon::Star,
        Source::Mail(Filter::Pinned) => Icon::Pin,
        Source::Mail(filter) if *filter == pending_snooze() => Icon::Clock,
        _ if role(MailboxRole::Inbox) => Icon::Inbox,
        _ if role(MailboxRole::Archive) => Icon::Archive,
        _ if role(MailboxRole::Trash) => Icon::Trash,
        _ if role(MailboxRole::Sent) => Icon::Send,
        _ if role(MailboxRole::Spam) => Icon::OctagonAlert,
        _ => Icon::Tag,
    }
}

/// An unread count as a badge holds it.
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
    /// Whether a message is waiting for its time in this Space, which is when Scheduled is drawn.
    scheduled: bool,
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
    // mailo's own places, History among them. Scheduled only while a message is waiting.
    let own: Vec<usize> = (0..split)
        .filter(|index| scheduled || places[*index].source != Source::Scheduled)
        .collect();
    let selected = shell.read().selected;
    let listed = |index: usize| {
        own.contains(&index)
            || labels.iter().any(|(at, _)| *at == index)
            || views.iter().any(|(at, _)| *at == index)
    };
    let mut items = vec![ListItem::heading(
        PlaceKey::Head("places"),
        rsx! { SectionHeader { title: copy::headers::PLACES } },
    )];
    for index in &own {
        let place = &places[*index];
        items.push(place_item(
            *index,
            &place.name,
            place_icon(&place.source),
            shell,
            pages,
            badges,
        ));
    }
    if !labels.is_empty() {
        items.push(ListItem::heading(
            PlaceKey::Head("labels"),
            rsx! { SectionHeader { title: copy::headers::LABELS } },
        ));
        for (index, name) in &labels {
            items.push(place_item(*index, name, Icon::Tag, shell, pages, badges));
        }
    }
    if !views.is_empty() {
        items.push(ListItem::heading(
            PlaceKey::Head("views"),
            rsx! { SectionHeader { title: copy::headers::VIEWS } },
        ));
        for (index, name) in &views {
            items.push(place_item(*index, name, Icon::Search, shell, pages, badges));
        }
    }
    rsx! {
        List::<PlaceKey> {
            label: copy::aria::PLACES,
            items,
            style: ListStyle::SourceList,
            cursor: (listed(selected) && shell.read().from_side.is_none()).then_some(PlaceKey::Place(selected)),
            onselect: move |key: PlaceKey| {
                if let PlaceKey::Place(index) = key {
                    super::go_to(shell, pages, index);
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
                super::go_to(shell, pages, index);
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
        drag::Drag::Live { target, .. } if accepts => lit(
            Landing::Possible,
            if *target == Some(index) {
                Under::Yes
            } else {
                Under::No
            },
        ),
        _ => lit(Landing::Impossible, Under::No),
    }
}

/// A Favourites row's key: a pinned conversation, or one of the Space's pinned people and
/// searches by its place in the Space's list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FavouriteKey {
    Head,
    Thread(ThreadId),
    Pin(usize),
}

/// Favourites, at the top of the sidebar: the conversations pinned most recently, then the
/// Space's pinned people and searches. Drawn only once something is pinned. A conversation with
/// unread mail is drawn heavier, and a person or search is badged only while it holds unread.
#[component]
pub(super) fn Favourites(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    space: Space,
    counted: Counts,
) -> Element {
    // Only which pinned conversation is open is read, so another write to the shell (a key in
    // the search box) does not draw Favourites again.
    let open = use_memo(move || shell.read().selected_in(SideList::Favourites));
    // No pins, no heading: a source list does not show an empty group.
    if counted.pinned.is_empty() && space.payload.pins.is_empty() {
        return rsx! {};
    }
    let mut items = vec![ListItem::heading(
        FavouriteKey::Head,
        rsx! { SectionHeader { title: copy::headers::FAVOURITES } },
    )];
    for (index, pinned) in counted.pinned.iter().enumerate() {
        let thread = pinned.thread;
        let content = rsx! {
            Row {
                leading: RowLeading::Avatar(face(pinned.initial, index)),
                title: pinned.title.clone(),
                common: tagged("favourite", pinned.title.clone()),
                state: RowState {
                    selection: if open() == Some(thread) { Selection::Selected } else { Selection::Unselected },
                    emphasis: if pinned.unread { Emphasis::Strong } else { Emphasis::Plain },
                    ..RowState::default()
                },
                onclick: move |_| shell.write().open_from(thread, SideList::Favourites),
            }
        };
        items.push(ListItem::row(
            FavouriteKey::Thread(thread),
            pinned.title.clone(),
            content,
        ));
    }
    let first = counted.pinned.len();
    for (index, pin) in space.payload.pins.iter().enumerate() {
        let name = match pin {
            Pinned::Person { name, .. } | Pinned::Search { name, .. } => name.clone(),
        };
        let query = match pin {
            Pinned::Person { email, .. } => format!("from:{email}"),
            Pinned::Search { query, .. } => query.clone(),
        };
        let n = counted.pins.get(index).copied().unwrap_or(0);
        let content = rsx! {
            Row {
                leading: RowLeading::Avatar(face(initial(&name), first + index + 3)),
                title: name.clone(),
                accessory: if n > 0 { Accessory::Badge(count_of(n)) } else { Accessory::None },
                onclick: move |_| {
                    shell.write().search = query.clone();
                    pages.set(1);
                },
            }
        };
        items.push(ListItem::row(FavouriteKey::Pin(index), name, content));
    }
    rsx! {
        List::<FavouriteKey> {
            label: copy::aria::FAVOURITES,
            items,
            style: ListStyle::SourceList,
        }
    }
}

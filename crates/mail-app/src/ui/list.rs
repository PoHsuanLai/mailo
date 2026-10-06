//! The list pane.
//!
//! The search box, what the pane says when it has nothing to show, the rows, and the way to
//! ask for another page. Split from [`super::app`] (`CONVENTIONS.md` §8). The queries stay in
//! `App`; this reads the memos it is handed rather than cloning their answers in the parent.

use super::common::classed;
use super::data::{AccountRow, account_rows, syncs_nothing};
use super::list_search::{Marking, RowHit, Scope, row_hit};
use super::motion::{Ghost, Toast};
use super::ops::start_new;
use super::page::PageMenus;
use super::picks::PickBar;
use super::press::on_primary;
use super::row::{DraftRow, MailRow};
use super::server_search::{Asked, ServerSearch, found_threads};
use super::view_groups::group_list;
use crate::ui::fetching::{CANNOT_LOAD, Fetching, HasRows, ListFace, list_face, sync_availability};
use crate::ui::view::{Nothing, Shell};
use dioxus::prelude::*;
use ds::components::chrome::toolbar::view::Toolbar;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::lists::virtual_list::{RowHeight, VirtualList};
use ds::components::overlays::empty_state::EmptyForm;
use ds::prelude::*;
use mail_core::fetch::Link;
use mail_core::provider::provider;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use porter_core::AccountId;
use std::collections::BTreeMap;
use std::sync::Arc;

mod first_sync;
mod status;

use self::first_sync::FirstSyncRows;
use self::status::ListStatus;

/// How far apart the list's rows are, their gap included: quire's card-density `ThreadRow` (three
/// lines and their padding, 61 px) and the 5 px under it.
const ROW_PITCH: f32 = 66.0;

/// A `SectionHeader`'s height: its eyebrow line and its padding (27.05 px drawn).
const HEADING_PITCH: f32 = 27.0;

/// How tall `slot`'s item is. Each is held to its height (`style/list.css`), so a font that sets a
/// line a fraction taller cannot push the rows off the offsets the window is computed from.
fn pitch(slot: &Slot) -> f32 {
    match slot {
        Slot::Draft(_) | Slot::Top(_) | Slot::Thread(_) | Slot::Server(_) => ROW_PITCH,
        Slot::Band(_) | Slot::TopHeading | Slot::NewestHeading | Slot::ServerHeading => {
            HEADING_PITCH
        }
    }
}

/// The rows the list mounts beyond the viewport, on each side. The `VirtualDom` fixtures
/// have no layout, so a viewport there is never measured and would show the overscan alone: they
/// mount every row, and the windowing itself is tested on Blitz.
#[cfg(not(test))]
const OVERSCAN: usize = 4;
#[cfg(test)]
const OVERSCAN: usize = usize::MAX / 2;

/// What a list item is, by identity: the same key on every render, so quire's `VirtualList` keeps a
/// row where it is, plays an exit for one that stops being listed and an entrance for a new one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Slot {
    /// A draft, by its id.
    Draft(String),
    /// A conversation in a search's top results.
    Top(ThreadId),
    /// A conversation in the date-ordered rows.
    Thread(ThreadId),
    /// A band's title.
    Band(String),
    /// A search's two headings.
    TopHeading,
    NewestHeading,
    /// The heading over what a search of the server found.
    ServerHeading,
    /// A conversation a search of the server found that the list does not already hold.
    Server(ThreadId),
}

/// The conversations and drafts for wherever the shell is looking.
#[component]
pub(super) fn ThreadList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
    in_a_field: Signal<bool>,
    threads: Memo<Vec<ThreadSummary>>,
    drafts: Memo<Vec<Draft>>,
    nothing: Memo<Nothing>,
    more: Memo<bool>,
    marking: Memo<Marking>,
    /// A search's top results, drawn above the date-ordered rows.
    top: Memo<Vec<ThreadSummary>>,
    /// Whether the next page has been asked for and is on its way.
    paging: Memo<bool>,
    /// Whether the sidebar is folded away; while it is, the header offers the way back.
    side_hidden: Signal<bool>,
    /// Which question the rows answer (`list_query::ListView::asked`).
    question: ReadSignal<u64>,
) -> Element {
    let rows = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        account_rows(&store)
    });
    // Each account's server asked for a searched line, and what it answered.
    let asked = use_signal(Vec::<Asked>::new);
    // What the servers found for the line shown that the list does not already hold, newest
    // first, as the store has each conversation now.
    let found = use_memo(move || {
        let _ = revision();
        let line = marking.read().line.clone();
        if line.is_empty() {
            return Vec::new();
        }
        let listed: Vec<ThreadId> = threads.read().iter().map(|t| t.id).collect();
        let store = consume_context::<Arc<SqliteStore>>();
        found_threads(&asked.read(), &line, &listed)
            .into_iter()
            .filter_map(|thread| store.thread(thread).ok())
            .map(|thread| thread.summary)
            .collect::<Vec<ThreadSummary>>()
    });
    // Which of the conversations drawn came here because a search of the server found them.
    let from_server = use_memo(move || {
        let _ = revision();
        let ids: Vec<ThreadId> = threads
            .read()
            .iter()
            .chain(found.read().iter())
            .map(|t| t.id)
            .collect();
        let store = consume_context::<Arc<SqliteStore>>();
        store.found_in(&ids).unwrap_or_default()
    });
    let place = shell
        .read()
        .places
        .get(shell.read().selected)
        .map(|place| place.name.clone())
        .unwrap_or_else(|| "Inbox".to_owned());
    let address = shell.read().account.clone().and_then(|id| {
        shell
            .read()
            .accounts
            .iter()
            .find(|(_, account)| *account == id)
            .map(|(address, _)| address.clone())
            .or_else(|| {
                rows()
                    .iter()
                    .find(|row| row.id == id)
                    .map(|row| row.shown())
            })
    });
    let address = address.or_else(|| {
        let known: Vec<(AccountId, String)> = rows()
            .into_iter()
            .map(|row| (row.id, row.address))
            .collect();
        super::folder_open::title_address(&shell.read(), &known)
    });
    let names: BTreeMap<LabelId, String> = shell
        .read()
        .labels
        .iter()
        .map(|(name, id)| (*id, name.clone()))
        .collect();
    // Local folders are never synced: with only them in view there is no Sync, and no word of one.
    let quiet = syncs_nothing(&rows(), shell.read().account.clone(), &shell.read().scope);
    // Said by the links of the accounts in view, and said of nothing when they have none.
    let fetching = try_consume_context::<Fetching>();
    let links: Vec<Link> = fetching
        .map(|fetching| {
            fetching
                .links_in_view(&shell.read())
                .into_iter()
                .map(|(_, link)| link)
                .collect()
        })
        .unwrap_or_default();
    let link_refs: Vec<&Link> = links.iter().collect();
    let sync_state = sync_availability(&link_refs);
    let search_note = marking.read().note();
    let invalid = matches!(marking.read().scope, Scope::Invalid(_));
    let accounts = rows();
    let highlight = marking.read().highlight.clone();
    let brought = from_server();
    let dress_row =
        |summary: &ThreadSummary| dress(summary, &accounts, &names, &highlight, &brought);
    // Where a search's list ends, each account in view with a server offers to search it.
    let line = marking.read().line.clone();
    let servers: Vec<(AccountId, String)> = if line.is_empty() || more() {
        Vec::new()
    } else {
        accounts
            .iter()
            .filter(|row| match shell.read().account.clone() {
                Some(id) => row.id == id,
                None => shell.read().scope.shows(row.id.clone()),
            })
            .filter(|row| mail_core::server_search::searchable(&row.plan))
            .map(|row| (row.id.clone(), row.shown()))
            .collect()
    };
    let server_rows = found();
    let state = use_hook(super::motion::motion);
    let shown = threads();
    let keys: Vec<ThreadId> = shown.iter().map(|summary| summary.id).collect();
    let picking = !shell.read().picked.chosen(&keys).is_empty();
    // Picked, or open with nothing picked: asked of the shell against what is listed.
    let selection_of = |id: ThreadId| {
        if shell.read().is_selected(id, &keys) {
            Selection::Selected
        } else {
            Selection::Unselected
        }
    };
    if let Some(state) = state {
        let mut order = state.order;
        order.set(keys.clone());
    }
    // The list's items, top to bottom: drafts, a search's top results, then the bands, then
    // what a search of the server found that the list does not hold.
    let mut items: Vec<ListItem<Slot>> = Vec::new();
    for draft in drafts() {
        let id = draft.id.to_string();
        let label = if draft.subject.is_empty() {
            "(no subject)".to_owned()
        } else {
            draft.subject.clone()
        };
        items.push(ListItem::row(
            Slot::Draft(id),
            label,
            rsx! { DraftRow { draft, shell } },
        ));
    }
    let strip = top();
    if !strip.is_empty() {
        items.push(ListItem::heading(
            Slot::TopHeading,
            rsx! { div { class: "band", SectionHeader { title: "Top results".to_owned() } } },
        ));
        for summary in strip {
            let Dress { via, chips, hit } = dress_row(&summary);
            let id = summary.id;
            let label = summary.subject.clone();
            let selection = selection_of(id);
            items.push(ListItem::row(
                Slot::Top(id),
                label,
                rsx! { MailRow { summary, shell, revision, chips, via, hit, selection } },
            ));
        }
        items.push(ListItem::heading(
            Slot::NewestHeading,
            rsx! { div { class: "band", SectionHeader { title: "Newest first".to_owned() } } },
        ));
    }
    for band in group_list(
        shown,
        &shell.read().grouping(),
        &names,
        chrono::Utc::now(),
        &chrono::Local,
    ) {
        if let Some(title) = band.title {
            items.push(ListItem::heading(
                Slot::Band(title.clone()),
                rsx! { div { class: "band", SectionHeader { title } } },
            ));
        }
        for summary in band.threads {
            let Dress { via, chips, hit } = dress_row(&summary);
            let id = summary.id;
            let label = summary.subject.clone();
            let selection = selection_of(id);
            items.push(ListItem::row(
                Slot::Thread(id),
                label,
                rsx! { MailRow { summary, shell, revision, chips, via, hit, selection } },
            ));
        }
    }
    if !server_rows.is_empty() {
        items.push(ListItem::heading(
            Slot::ServerHeading,
            rsx! { div { class: "band", SectionHeader { title: "From the server".to_owned() } } },
        ));
        for summary in server_rows {
            let Dress { via, chips, hit } = dress_row(&summary);
            let id = summary.id;
            let label = summary.subject.clone();
            let selection = selection_of(id);
            items.push(ListItem::row(
                Slot::Server(id),
                label,
                rsx! { MailRow { summary, shell, revision, chips, via, hit, selection } },
            ));
        }
    }
    let cursor = shell.read().open.map(Slot::Thread);
    let open_slot = move |slot: Slot| {
        if let Slot::Thread(id) | Slot::Top(id) | Slot::Server(id) = slot {
            shell.write().open(id);
        }
    };
    // Windowed: only the rows near the viewport are mounted, so a folder of ten thousand draws as
    // cheaply as one of twenty. Each item's height is known before it is drawn — a mail or draft
    // row, or a heading — and each is held to it (`style/list.css`).
    let keys: Vec<Slot> = items.iter().map(|item| item.key.clone()).collect();
    let drawn: std::collections::HashMap<Slot, Element> = items
        .into_iter()
        .map(|item| (item.key, item.content))
        .collect();
    let row = Callback::new(move |slot: Slot| drawn.get(&slot).cloned().unwrap_or_else(|| rsx! {}));
    let height = RowHeight::PerKey(Callback::new(|slot: Slot| Px(pitch(&slot))));
    let nothing_here = threads().is_empty() && drafts().is_empty() && found().is_empty();
    let has_rows = if nothing_here {
        HasRows::No
    } else {
        HasRows::Yes
    };
    let face = list_face(has_rows, &link_refs, &nothing());
    let phase = match &face {
        ListFace::FirstSync => Phase::Loading(
            fetching
                .map(|fetching| fetching.op_in_view(&shell.read()))
                .unwrap_or_default(),
        ),
        ListFace::CannotLoad(problem) => Phase::Failed {
            title: CANNOT_LOAD.to_owned(),
            description: Some(problem.description().into()),
        },
        ListFace::Rows | ListFace::Empty | ListFace::NoMatch(_) | ListFace::NoAccount => {
            Phase::Ready
        }
    };
    // Beside Retry in the failure `Loadable` draws: a way into the Connection Doctor.
    let doctor_action: Option<Element> = matches!(face, ListFace::CannotLoad(_)).then(|| {
        rsx! {
            Button {
                label: "Connection Doctor\u{2026}",
                onclick: on_primary(move || super::doctor::open(shell)),
            }
        }
    });
    let empty_form = match nothing() {
        Nothing::NoMatch(_) => EmptyForm::NoResults,
        Nothing::NoAccount | Nothing::EmptyFolder => EmptyForm::Empty,
    };
    // With no account the way out is a button, not a command to type.
    let empty_action: Option<Element> = matches!(nothing(), Nothing::NoAccount).then(|| {
        rsx! {
            Button {
                label: "Add Account\u{2026}",
                onclick: on_primary(move || super::add_account::open(shell)),
            }
        }
    });
    let retry = EventHandler::new(move |()| super::fetching::sync_now(&shell.read()));
    rsx! {
        div { class: "list-col",
            // The list's header is quire's 52 px `Toolbar`. While anything listed is picked, the
            // bar is the selection's: its count and its actions, and nothing else.
            Toolbar::<()> {
                onpick: move |_| {},
                center: rsx! {
                    div { class: if picking { "list-head picking" } else { "list-head" },
                        // The sidebar's own toggle lives in its footer, which is gone with it.
                        if side_hidden() {
                            Button {
                                bezel: Bezel::Toolbar,
                                image: ImagePosition::Only,
                                label: "Show sidebar",
                                icon: Some(IconSource::Glyph(Icon::PanelLeft)),
                                title: Some("Show the sidebar (\u{2303}\u{2318}S)".to_owned()),
                                onclick: on_primary(move || side_hidden.set(false)),
                            }
                        }
                        if picking {
                            PickBar { shell, revision, threads }
                        } else {
                            div { class: "list-title",
                                Label { text: place.clone(), style: LabelStyle::Title, common: classed("ds-truncate") }
                                if let Some(address) = address {
                                    Label {
                                        text: address,
                                        role: LabelRole::Tertiary,
                                        style: LabelStyle::Caption,
                                        common: classed("ds-truncate"),
                                    }
                                }
                            }
                            ListStatus { shell }
                            if let Some(said) = search_note {
                                Label {
                                    text: said,
                                    role: LabelRole::Tertiary,
                                    style: LabelStyle::Footnote,
                                    common: classed(if invalid { "status bad" } else { "status" }),
                                }
                            }
                            div { class: "bar-tools",
                                PageMenus { shell }
                                // Trash and Spam alone: everything in them, deleted forever, once asked.
                                super::destroy::EmptyButton { shell }
                                // A search can be kept as a view, and a view shown can be changed.
                                if !shell.read().search.trim().is_empty() {
                                    Button {
                                        bezel: Bezel::Toolbar,
                                        image: ImagePosition::Only,
                                        label: "Save as view",
                                        icon: Some(IconSource::Glyph(Icon::Plus)),
                                        title: Some("Keep this search in the sidebar".to_owned()),
                                        onclick: on_primary(move || {
                                            let search = shell.peek().search.clone();
                                            super::views::open_new(shell, &search);
                                        }),
                                    }
                                } else if let Some(view) = shell.read().saved_view().cloned() {
                                    Button {
                                        bezel: Bezel::Toolbar,
                                        image: ImagePosition::Only,
                                        label: "Edit view",
                                        icon: Some(IconSource::Glyph(Icon::Settings)),
                                        title: Some("Change or delete this view".to_owned()),
                                        onclick: on_primary(move || super::views::open_edit(shell, &view)),
                                    }
                                }
                                if !quiet {
                                    Button {
                                        bezel: Bezel::Toolbar,
                                        image: ImagePosition::Only,
                                        label: "Sync now",
                                        icon: Some(IconSource::Glyph(Icon::Refresh)),
                                        availability: sync_state,
                                        onclick: on_primary(move || super::fetching::sync_now(&shell.read())),
                                    }
                                }
                                Button {
                                    bezel: Bezel::Toolbar,
                                    image: ImagePosition::Only,
                                    label: "Compose",
                                    icon: Some(IconSource::Glyph(Icon::Pen)),
                                    title: Some("New message (\u{2318}N)".to_owned()),
                                    onclick: on_primary(move || {
                                        let store = consume_context::<Arc<SqliteStore>>();
                                        let known = shell.peek().accounts.clone();
                                        match start_new(&store, &known) {
                                            Ok(draft) => {
                                                shell.write().compose(&draft);
                                                revision += 1;
                                            }
                                            Err(why) => eprintln!("compose: {why}"),
                                        }
                                    }),
                                }
                            }
                        }
                    }
                },
            }
            TextField {
                kind: FieldKind::Search,
                label: "Search all mail".to_owned(),
                placeholder: "Search all mail".to_owned(),
                value: shell.read().search.clone(),
                oninput: move |value: String| {
                    shell.write().search = value;
                    pages.set(1);
                },
                onfocus: move |()| in_a_field.set(true),
                onblur: move |()| in_a_field.set(false),
                common: classed("search"),
            }
            // quire's list: it keeps each row by its key, so a row that leaves plays its exit and
            // the rows below close the gap. A `Loadable` decides what the pane holds: outline rows
            // while the first mail comes, the failure when no account can be reached, else the
            // rows, which fade in when they arrive. Keyed by the question, so another place or
            // another search is a new list, from its top, not this one losing every row through
            // its exit.
            div { class: "list",
                Loadable {
                    phase,
                    placeholder: rsx! { FirstSyncRows {} },
                    onretry: retry,
                    action: doctor_action,
                    VirtualList::<Slot> {
                        key: "{question}",
                        label: place.clone(),
                        keys,
                        row,
                        height,
                        overscan: OVERSCAN,
                        cursor,
                        onselect: open_slot,
                        // The next page is asked for as the end of the list comes near.
                        near_end: move |()| {
                            if more() && !paging() {
                                pages += 1;
                            }
                        },
                    }
                    if nothing_here {
                        EmptyState {
                            form: empty_form,
                            title: nothing().message(),
                            action: empty_action,
                        }
                    }
                }
            }
            if !servers.is_empty() {
                ServerSearch { input: line.clone(), accounts: servers, asked, revision }
            }
            Toast { shell, revision }
            Ghost {}
        }
    }
}

/// What a row shows beside its summary: the provider mark, the label chips, the search marks.
struct Dress {
    via: Option<mail_core::provider::Provider>,
    chips: Vec<String>,
    hit: Option<RowHit>,
}

/// The chip on a conversation a search of the server brought here.
pub(super) const FROM_SERVER: &str = "from the server";

fn dress(
    summary: &ThreadSummary,
    accounts: &[AccountRow],
    names: &BTreeMap<LabelId, String>,
    highlight: &mail_core::search::Highlight,
    from_server: &[ThreadId],
) -> Dress {
    let mut chips: Vec<String> = summary
        .labels
        .iter()
        .filter_map(|id| names.get(id).cloned())
        .collect();
    if from_server.contains(&summary.id) {
        chips.push(FROM_SERVER.to_owned());
    }
    Dress {
        via: accounts
            .iter()
            .find(|row| row.id == summary.account)
            .map(|row| provider(&row.plan)),
        chips,
        hit: row_hit(summary, highlight),
    }
}

#[cfg(test)]
mod tests;

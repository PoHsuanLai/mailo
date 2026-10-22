//! The list pane.
//!
//! The search box, what the pane says when it has nothing to show, the rows, and the way to
//! ask for another page. Split from [`super::app`] (`CONVENTIONS.md` §8). The queries stay in
//! `App`; this reads the memos it is handed rather than cloning their answers in the parent.

use super::common::classed;
use super::data::{AccountRow, account_rows, syncs_nothing};
use super::list_search::{Marking, RowHit, Scope, row_hit};
use super::motion::{Ghost, Toast};
use super::ops::{Writes, open_composer};
use super::page::ListMore;
use super::picks::PickBar;
use super::press::on_primary;
use super::row::listed::Listed;
use super::row::{AccountMark, ListRow};
use super::server_search::{Asked, ServerSearch, found_threads};
use super::view_groups::group_list;
use crate::ui::copy;
use crate::ui::fetching::{CANNOT_LOAD, Fetching, HasRows, ListFace, Need, list_face};
use crate::ui::reauth::{Reauth, SIGN_IN};
use crate::ui::view::{Nothing, Shell};
use dioxus::prelude::*;
use ds::components::app::thread_height::{ThreadLines, thread_card_gap, thread_card_height};
use ds::components::chrome::toolbar::view::Toolbar;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::lists::virtual_list::{RowHeight, VirtualList};
use ds::components::overlays::empty_state::EmptyForm;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::fetch::Link;
use mail_core::provider::provider;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use porter_core::AccountId;
use std::collections::BTreeMap;
use std::sync::Arc;

mod first_sync;
mod status;

use self::first_sync::FirstSyncRows;
use self::status::ListProgress;

/// A `SectionHeader`'s height: its eyebrow line and its padding (27.05 px drawn).
const HEADING_PITCH: f32 = 27.0;

/// How tall an item is: a row with `lines` of text, or a heading (`None`). Each is held to its
/// height (`style/list.css`), so a font that sets a line a fraction taller cannot push the rows
/// off the offsets the window is computed from. A row's is quire's card `ThreadRow` with its
/// lines of text and the gap under it, both computed from quire's own tokens, so a change to its
/// type scale moves the pitch with it. A row is as tall as what it draws: one with no snippet
/// leaves no gap where a snippet would be.
fn pitch(lines: Option<ThreadLines>) -> f32 {
    match lines {
        Some(lines) => thread_card_height(lines).0 + thread_card_gap().0,
        None => HEADING_PITCH,
    }
}

/// How many lines a row draws: its snippet is the third, when the list shows snippets and it has
/// one.
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
    /// The heading over a search's top results.
    TopHeading,
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
    let place = shell
        .read()
        .places
        .get(shell.read().selected)
        .map(|place| place.name.clone())
        .unwrap_or_else(|| copy::titles::INBOX.to_owned());
    let names: BTreeMap<LabelId, String> = shell
        .read()
        .labels
        .iter()
        .map(|(name, id)| (*id, name.clone()))
        .collect();
    // Local folders are never synced: with only them in view the ⋯ has no Sync Now.
    let quiet = syncs_nothing(&rows(), &shell.read().scope);
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
    let search_note = marking.read().note();
    let invalid = matches!(marking.read().scope, Scope::Invalid(_));
    let accounts = rows();
    let highlight = marking.read().highlight.clone();
    // A row names its account only while the list shows more than one.
    let several = accounts
        .iter()
        .filter(|row| shell.read().scope.shows(row.id.clone()))
        .count()
        > 1;
    let dress_row =
        |summary: &ThreadSummary| dress(summary, &accounts, several, &names, &highlight);
    // Where a search's list ends, each account in view with a server offers to search it.
    let line = marking.read().line.clone();
    let servers: Vec<(AccountId, String)> = if line.is_empty() || more() {
        Vec::new()
    } else {
        accounts
            .iter()
            .filter(|row| shell.read().scope.shows(row.id.clone()))
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
    // Each row's lines, by its key: what its pitch is computed from. A heading has none.
    let snippets = shell.read().parts.snippet.shown();
    let mut lines: std::collections::HashMap<Slot, ThreadLines> = std::collections::HashMap::new();
    let composing = shell.read().composing.as_ref().map(|open| open.draft);
    let now = chrono::Utc::now();
    let hours = super::clock::hours();
    // Every row, a conversation or a draft, is the one `ListRow`, drawn from its `RowModel`.
    let mut push_row = |slot: Slot,
                        listed: Listed,
                        dress: Dress,
                        selection: Selection,
                        items: &mut Vec<ListItem<Slot>>| {
        let Dress { via, chips, hit } = dress;
        let model = listed.row(hit.as_ref(), snippets, now, &chrono::Local, hours);
        lines.insert(slot.clone(), model.lines);
        let label = listed.subject();
        items.push(ListItem::row(
            slot,
            label,
            rsx! { ListRow { listed, model, shell, revision, chips, via, selection } },
        ));
    };
    // A draft is selected while its composer is open.
    for draft in drafts() {
        let selection = if composing == Some(draft.id) {
            Selection::Selected
        } else {
            Selection::Unselected
        };
        let slot = Slot::Draft(draft.id.to_string());
        push_row(
            slot,
            Listed::Draft(draft),
            Dress::default(),
            selection,
            &mut items,
        );
    }
    let mut mail_row = |slot: Slot, summary: ThreadSummary, items: &mut Vec<ListItem<Slot>>| {
        let dress = dress_row(&summary);
        let selection = selection_of(summary.id);
        push_row(slot, Listed::Thread(summary), dress, selection, items);
    };
    let strip = top();
    if !strip.is_empty() {
        items.push(ListItem::heading(
            Slot::TopHeading,
            rsx! { div { class: "band", SectionHeader { title: copy::headers::TOP_RESULTS.to_owned() } } },
        ));
        for summary in strip {
            mail_row(Slot::Top(summary.id), summary, &mut items);
        }
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
            mail_row(Slot::Thread(summary.id), summary, &mut items);
        }
    }
    if !server_rows.is_empty() {
        items.push(ListItem::heading(
            Slot::ServerHeading,
            rsx! { div { class: "band", SectionHeader { title: copy::headers::FROM_SERVER.to_owned() } } },
        ));
        for summary in server_rows {
            mail_row(Slot::Server(summary.id), summary, &mut items);
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
    let height = RowHeight::PerKey(Callback::new(move |slot: Slot| {
        Px(pitch(lines.get(&slot).copied()))
    }));
    let nothing_here = threads().is_empty() && drafts().is_empty() && found().is_empty();
    let has_rows = if nothing_here {
        HasRows::No
    } else {
        HasRows::Yes
    };
    let face = list_face(has_rows, &link_refs, &nothing());
    // When no account can be loaded because one must sign in, the pane names that account and
    // offers the sign-in itself.
    let sign_in: Option<(String, Need)> = match &face {
        ListFace::CannotLoad(problem) => problem.need().and_then(|need| {
            let (id, _) = fetching?
                .needs_in_view(&shell.read())
                .into_iter()
                .find(|(_, waiting)| *waiting == need)?;
            rows()
                .into_iter()
                .find(|row| row.id == id)
                .map(|row| (row.shown(), need))
        }),
        _ => None,
    };
    let phase = match &face {
        ListFace::FirstSync => Phase::Loading(
            fetching
                .map(|fetching| fetching.op_in_view(&shell.read()))
                .unwrap_or_default(),
        ),
        ListFace::CannotLoad(problem) => Phase::Failed {
            title: CANNOT_LOAD.to_owned(),
            description: Some(
                problem
                    .description(
                        sign_in
                            .as_ref()
                            .map_or(copy::errors::THIS_ACCOUNT, |(name, _)| name),
                    )
                    .into(),
            ),
        },
        ListFace::Rows | ListFace::Empty | ListFace::NoMatch(_) | ListFace::NoAccount => {
            Phase::Ready
        }
    };
    // Beside Retry in the failure `Loadable` draws, or instead of it, where Retry cannot help.
    let reauth = try_consume_context::<Reauth>();
    let sign_in_action: Option<Element> = sign_in.clone().and_then(|(name, need)| {
        let reauth = reauth?;
        Some(rsx! {
            Button {
                label: SIGN_IN,
                common: crate::ui::sidebar::tagged("sign-in", "mailbox"),
                onclick: on_primary(move || reauth.sign_in(&name, need)),
            }
        })
    });
    let empty_form = match nothing() {
        Nothing::NoMatch(_) => EmptyForm::NoResults,
        Nothing::NoAccount | Nothing::EmptyFolder => EmptyForm::Empty,
    };
    // With no account the way out is a button, not a command to type.
    let empty_action: Option<Element> = matches!(nothing(), Nothing::NoAccount).then(|| {
        rsx! {
            Button {
                label: copy::controls::ADD_ACCOUNT,
                onclick: on_primary(super::add_account::open),
            }
        }
    });
    let retry = EventHandler::new(move |()| super::fetching::sync_now(&shell.read()));
    rsx! {
        div { class: "list-col",
            // The list's header is quire's 52 px `Toolbar`: the place's title, New Message, the
            // search and a ⋯ for the rest (`page::ListMore`). How fresh the mail is, and any
            // warning, is the sidebar's sync mark's to say. While anything listed is picked, the
            // bar is the selection's: its count and its actions, and nothing else.
            Toolbar::<()> {
                onpick: move |_| {},
                center: rsx! {
                    div { class: if picking { "list-head picking" } else { "list-head" },
                        // The sidebar's own toggle lives in its footer, which is gone with it.
                        if side_hidden() {
                            Button {
                                bezel: Bezel::Toolbar,
                                size: ControlSize::Large,
                                image: ImagePosition::Only,
                                label: copy::aria::SHOW_SIDEBAR,
                                icon: Some(IconSource::Glyph(Icon::PanelLeft)),
                                title: Some(copy::tooltips::SHOW_SIDEBAR.to_owned()),
                                title_shortcut: crate::ui::actions::tip_standard(chordkit::StandardAction::ToggleSidebar),
                                onclick: on_primary(move || side_hidden.set(false)),
                            }
                        }
                        if picking {
                            PickBar { shell, revision, threads }
                        } else {
                            div { class: "list-title",
                                Label { text: place.clone(), style: LabelStyle::Title, common: classed("truncate") }
                            }
                            if let Some(said) = search_note {
                                Label {
                                    text: said,
                                    role: LabelRole::Tertiary,
                                    style: LabelStyle::Footnote,
                                    common: classed(if invalid { "status bad" } else { "status" }),
                                }
                            }
                            div { class: "bar-tools",
                                // Trash and Spam alone: everything in them, deleted forever, once asked.
                                super::destroy::EmptyButton { shell }
                                // A search can be kept as a view, and a view shown can be changed.
                                if !shell.read().search.trim().is_empty() {
                                    Button {
                                        bezel: Bezel::Toolbar,
                                        size: ControlSize::Large,
                                        image: ImagePosition::Only,
                                        label: copy::aria::SAVE_AS_VIEW,
                                        icon: Some(IconSource::Glyph(Icon::Plus)),
                                        title: Some(copy::tooltips::SAVE_AS_VIEW.to_owned()),
                                        onclick: on_primary(move || {
                                            let search = shell.peek().search.clone();
                                            super::views::open_new(shell, &search);
                                        }),
                                    }
                                } else if let Some(view) = shell.read().saved_view().cloned() {
                                    Button {
                                        bezel: Bezel::Toolbar,
                                        size: ControlSize::Large,
                                        image: ImagePosition::Only,
                                        label: copy::aria::EDIT_VIEW,
                                        icon: Some(IconSource::Glyph(Icon::Settings)),
                                        title: Some(copy::tooltips::EDIT_VIEW.to_owned()),
                                        onclick: on_primary(move || super::views::open_edit(shell, &view)),
                                    }
                                }
                                Button {
                                    bezel: Bezel::Toolbar,
                                    size: ControlSize::Large,
                                    image: ImagePosition::Only,
                                    label: copy::aria::NEW_MESSAGE,
                                    icon: Some(IconSource::Glyph(Icon::Pen)),
                                    title: Some(copy::tooltips::NEW_MESSAGE.to_owned()),
                                    title_shortcut: crate::ui::actions::tip_standard(chordkit::StandardAction::New),
                                    onclick: on_primary(move || {
                                        let store = consume_context::<Arc<SqliteStore>>();
                                        if open_composer(&store, shell, Writes::New).is_ok() {
                                            revision += 1;
                                        }
                                    }),
                                }
                            }
                            // The search, at the toolbar's trailing end where the Mac keeps it: a
                            // magnifier that brings up the search panel, or the search shown. The
                            // ⋯ closes the row: Group, Row Properties and Sync Now.
                            super::command::SearchBox { shell, pages }
                            ListMore { shell, syncs: !quiet, side_hidden }
                        }
                    }
                },
            }
            // Only while the first mail comes: after it, how a sync goes is the sync mark's.
            if face == ListFace::FirstSync {
                ListProgress { shell }
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
                    onretry: if sign_in.is_some() { None } else { Some(retry) },
                    action: sign_in_action,
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

/// What a row shows beside its summary: its account's mark, the label chips, the search marks.
/// A draft's is empty.
#[derive(Default)]
struct Dress {
    via: Option<AccountMark>,
    chips: Vec<String>,
    hit: Option<RowHit>,
}

/// `summary`'s dress. Its account is named only when `several` accounts are in view.
fn dress(
    summary: &ThreadSummary,
    accounts: &[AccountRow],
    several: bool,
    names: &BTreeMap<LabelId, String>,
    highlight: &mail_core::search::Highlight,
) -> Dress {
    let chips: Vec<String> = summary
        .labels
        .iter()
        .filter_map(|id| names.get(id).cloned())
        .collect();
    let via = several
        .then(|| accounts.iter().find(|row| row.id == summary.account))
        .flatten()
        .map(|row| AccountMark {
            provider: provider(&row.plan),
            name: row.shown(),
        });
    Dress {
        via,
        chips,
        hit: row_hit(summary, highlight),
    }
}

#[cfg(test)]
mod tests;

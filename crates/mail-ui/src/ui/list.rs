//! The list pane.
//!
//! The search box, what the pane says when it has nothing to show, the rows, and the way to
//! ask for another page. Split from [`super::app`]. The queries stay in
//! `App`; this reads the memos it is handed rather than cloning their answers in the parent.

use super::common::classed;
use super::data::{AccountRow, syncs_nothing};
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
use crate::ui::fetching::{Fetching, HasRows, ListFace, Need, list_face};
use crate::ui::reauth::{Reauth, SIGN_IN};
use crate::ui::view::{Nothing, Shell};
use dioxus::prelude::*;
use ds::components::app::thread_height::{ThreadLines, thread_card_gap, thread_card_height};
use ds::components::chrome::toolbar::view::Toolbar;
use ds::components::content::label::{Label, LabelStyle};
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::lists::virtual_list::{RowHeight, VirtualList};
use ds::components::overlays::empty_state::EmptyForm;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_client::fetch::Link;
use mail_client::provider::provider;
use mail_domain::*;
use mail_ui_model::copy;
use porter_core::AccountId;
use std::collections::BTreeMap;

mod first_sync;
pub(in crate::ui) mod quick;
mod status;

use self::first_sync::FirstSyncRows;
use self::quick::{FilterButton, QuickRow};
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
    /// A message in the date-ordered rows, while the place lists a message to a row.
    Message(MessageId),
    /// A band's title.
    Band(String),
    /// The heading over a search's top results.
    TopHeading,
    /// The heading over what a search of the server found.
    ServerHeading,
    /// A conversation a search of the server found that the list does not already hold.
    Server(ThreadId),
}

/// A row of the date-ordered list: a conversation, or one message of it while the place lists a
/// message to a row. Grouped as its summary is ([`group_list`]).
struct Row {
    summary: ThreadSummary,
    message: Option<MessageId>,
}

impl std::borrow::Borrow<ThreadSummary> for Row {
    fn borrow(&self) -> &ThreadSummary {
        &self.summary
    }
}

/// `threads` as rows, each with its message when `messages` names one for every row.
fn rows_of(threads: Vec<ThreadSummary>, messages: &[MessageId]) -> Vec<Row> {
    let singles = !messages.is_empty() && messages.len() == threads.len();
    threads
        .into_iter()
        .enumerate()
        .map(|(at, summary)| Row {
            summary,
            message: singles.then(|| messages[at]),
        })
        .collect()
}

/// The conversations and drafts for wherever the shell is looking.
#[component]
pub(super) fn ThreadList(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    threads: Memo<Vec<ThreadSummary>>,
    /// Which message each of `threads` is, while the place lists a message to a row.
    messages: Memo<Vec<MessageId>>,
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
    /// Whether the client could not read the rows (`list_query::ListView::failed`).
    failed: Memo<bool>,
) -> Element {
    let every = crate::ui::live::use_live(|| mail_client::query::Accounts);
    let rows = use_memo(move || {
        every
            .read()
            .value()
            .map(|accounts| crate::ui::data::rows_of(accounts))
            .unwrap_or_default()
    });
    // Each account's server asked for a searched line, and what it answered.
    let asked = use_signal(Vec::<Asked>::new);
    // What the servers found for the line shown that the list does not already hold, newest
    // first, as the client has each conversation now: kept read by the client, never looked up
    // here.
    let hits = use_memo(move || {
        let line = marking.read().line.clone();
        if line.is_empty() {
            return Vec::new();
        }
        let listed: Vec<ThreadId> = threads.read().iter().map(|t| t.id).collect();
        found_threads(&asked.read(), &line, &listed)
    });
    let summaries = crate::ui::live::use_live(move || mail_client::query::Summaries(hits()));
    // Only the hits asked for now: while the answer to a new line is read, the last one's is
    // not drawn under it.
    let found = use_memo(move || {
        let wanted = hits.read();
        summaries
            .read()
            .value()
            .map(|summaries| {
                summaries
                    .iter()
                    .filter(|summary| wanted.contains(&summary.id))
                    .cloned()
                    .collect::<Vec<ThreadSummary>>()
            })
            .unwrap_or_default()
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
            .filter(|row| row.plan.incoming.caps().searchable)
            .map(|row| (row.id.clone(), row.shown()))
            .collect()
    };
    let server_rows = found();
    let state = use_hook(super::motion::motion);
    let shown = rows_of(threads(), &messages.read());
    let keys: Vec<ThreadId> = shown.iter().map(|row| row.summary.id).collect();
    let picking = !shell.read().picked.chosen(&keys).is_empty();
    // What the header is: the selection's while anything is picked, else the search's while one
    // narrows the list and the panel is down, else the place's own.
    let head = if picking {
        Head::Picking
    } else {
        match super::command::affordance(&shell.read().search, &shell.read().bar) {
            super::command::Affordance::Searching(_) => {
                let listed = shell.read().listed_scope();
                let looked: Vec<String> = accounts
                    .iter()
                    .filter(|row| listed.shows(row.id.clone()))
                    .map(AccountRow::shown)
                    .collect();
                Head::Searching(searched(
                    &marking.read().scope,
                    &place,
                    &looked,
                    accounts.len(),
                ))
            }
            super::command::Affordance::Idle => Head::Idle,
        }
    };
    let head_class = head.class();
    // Picked, or open with nothing picked: asked of the shell against what is listed. A
    // message's row is the open one only while it is the message the reader shows.
    let selection_of = |id: ThreadId, message: Option<MessageId>| {
        let current = shell.read();
        let selected = match message {
            Some(message) if !current.picked.any(&keys) => {
                current.open == Some(id) && current.focus == Some(message)
            }
            _ => current.is_selected(id, &keys),
        };
        if selected {
            Selection::Selected
        } else {
            Selection::Unselected
        }
    };
    // Which conversation each listed message is of, for the keyboard's cursor to open one.
    let of_message: std::collections::HashMap<MessageId, (ThreadId, usize)> = shown
        .iter()
        .enumerate()
        .filter_map(|(at, row)| row.message.map(|message| (message, (row.summary.id, at))))
        .collect();
    if let Some(state) = state {
        let mut order = state.order;
        order.set(keys.clone());
        let mut messages = state.messages;
        messages.set(shown.iter().filter_map(|row| row.message).collect());
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
            rsx! { ListRow { listed, model, shell, chips, via, selection } },
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
    let mut mail_row = |slot: Slot, row: Row, items: &mut Vec<ListItem<Slot>>| {
        let dress = dress_row(&row.summary);
        let selection = selection_of(row.summary.id, row.message);
        let listed = match row.message {
            Some(message) => Listed::Message(row.summary, message),
            None => Listed::Thread(row.summary),
        };
        push_row(slot, listed, dress, selection, items);
    };
    let conversation = |summary: ThreadSummary| Row {
        summary,
        message: None,
    };
    let strip = top();
    if !strip.is_empty() {
        items.push(ListItem::heading(
            Slot::TopHeading,
            rsx! { div { class: "band", SectionHeader { title: copy::headers::TOP_RESULTS.to_owned() } } },
        ));
        for summary in strip {
            mail_row(Slot::Top(summary.id), conversation(summary), &mut items);
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
        for row in band.threads {
            let slot = match row.message {
                Some(message) => Slot::Message(message),
                None => Slot::Thread(row.summary.id),
            };
            mail_row(slot, row, &mut items);
        }
    }
    if !server_rows.is_empty() {
        items.push(ListItem::heading(
            Slot::ServerHeading,
            rsx! { div { class: "band", SectionHeader { title: copy::headers::FROM_SERVER.to_owned() } } },
        ));
        for summary in server_rows {
            mail_row(Slot::Server(summary.id), conversation(summary), &mut items);
        }
    }
    let cursor = match (shell.read().open, shell.read().focus) {
        (Some(_), Some(message)) if of_message.contains_key(&message) => {
            Some(Slot::Message(message))
        }
        (open, _) => open.map(Slot::Thread),
    };
    let open_slot = move |slot: Slot| match slot {
        Slot::Thread(id) | Slot::Top(id) | Slot::Server(id) => shell.write().open(id),
        Slot::Message(message) => {
            if let Some((id, at)) = of_message.get(&message) {
                shell.write().open_message(*id, message, *at);
            }
        }
        Slot::Draft(_) | Slot::Band(_) | Slot::TopHeading | Slot::ServerHeading => {}
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
    // A reading that failed, of the accounts or of the rows, with nothing earlier to show: said,
    // not drawn as an empty mailbox.
    let unread =
        nothing_here && (failed() || matches!(*every.read(), mail_client::Snapshot::Failed { .. }));
    let phase = match &face {
        ListFace::Empty | ListFace::NoAccount | ListFace::NoMatch(_) | ListFace::Filtered
            if unread =>
        {
            Phase::Failed {
                title: copy::status::CANNOT_LOAD.to_owned(),
                description: Some(copy::status::NOT_READ.to_owned().into()),
            }
        }
        ListFace::FirstSync => Phase::Loading(
            fetching
                .map(|fetching| fetching.op_in_view(&shell.read()))
                .unwrap_or_default(),
        ),
        ListFace::CannotLoad(problem) => Phase::Failed {
            title: copy::status::CANNOT_LOAD.to_owned(),
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
        ListFace::Rows
        | ListFace::Empty
        | ListFace::NoMatch(_)
        | ListFace::Filtered
        | ListFace::NoAccount => Phase::Ready,
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
        Nothing::NoMatch(_) | Nothing::Filtered => EmptyForm::NoResults,
        Nothing::NoAccount | Nothing::EmptyFolder => EmptyForm::Empty,
    };
    // With no account the way out is a button, not a command to type; with the quick filter
    // leaving nothing, it is clearing it.
    let empty_action: Option<Element> = match nothing() {
        Nothing::NoAccount => Some(rsx! {
            Button {
                label: copy::controls::ADD_ACCOUNT,
                onclick: on_primary(super::add_account::open),
            }
        }),
        Nothing::Filtered => Some(rsx! {
            Button {
                label: copy::controls::CLEAR_FILTER,
                onclick: on_primary(move || quick::clear(shell, pages)),
            }
        }),
        Nothing::NoMatch(_) | Nothing::EmptyFolder => None,
    };
    let retry = EventHandler::new(move |()| super::fetching::sync_now(&shell.read()));
    rsx! {
        div { class: "list-col",
            // The list's header is quire's 52 px `Toolbar`: the place's title, New Message, the
            // quick filter's button, the search and a ⋯ for the rest (`page::ListMore`). How fresh the mail is, and any
            // warning, is the sidebar's sync mark's to say. While anything listed is picked, the
            // bar is the selection's: its count and its actions, and nothing else. While a search
            // narrows the list it is the search's: the search across it, Save as View and the ×.
            Toolbar::<()> {
                onpick: move |_| {},
                center: rsx! {
                    div { class: head_class,
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
                        match head {
                            Head::Picking => rsx! { PickBar { shell, threads } },
                            Head::Searching(Said { text, bad }) => rsx! {
                                super::command::SearchHead { shell, pages, scope: text, bad }
                            },
                            Head::Idle => rsx! { PlaceTools { shell, place: place.clone(), pages, syncs: !quiet, side_hidden } },
                        }
                    }
                },
            }
            // Unread, Starred and Has Attachments, while the Filter button shows them.
            QuickRow { shell, pages }
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
                    // Over the empty list, not under it: the list fills the pane, and what the
                    // pane says (and Clear Filter, Add Account) would be drawn past its end.
                    if nothing_here && !unread {
                        div { class: "list-empty",
                            EmptyState {
                                form: empty_form,
                                title: nothing().message(),
                                action: empty_action,
                            }
                        }
                    }
                }
            }
            if !servers.is_empty() {
                ServerSearch { input: line.clone(), accounts: servers, asked }
            }
            Toast { shell }
            Ghost {}
        }
    }
}

/// What the list's header holds.
#[derive(Debug, Clone, PartialEq)]
enum Head {
    /// The place's title and tools, and the magnifier.
    Idle,
    /// The selection's count and actions.
    Picking,
    /// The search that narrows the list, and where it looks.
    Searching(Said),
}

impl Head {
    fn class(&self) -> &'static str {
        match self {
            Head::Idle => "list-head",
            Head::Picking => "list-head picking",
            Head::Searching(_) => "list-head searching",
        }
    }
}

/// Where a search looks, as the end of the search shown says it, or why it cannot be run.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Said {
    text: String,
    /// The search cannot be run: said in the danger ink.
    bad: bool,
}

/// Where a search the list answered as `scope` looked. A search looks through every account in
/// view, whatever place is selected, so it is named by the one account `looked` at when the list
/// shows one of the `all` there are, and is all mail otherwise. A bare pattern matches only the
/// rows `place` had loaded; a pattern that does not compile says why ([`reason`]).
fn searched(scope: &Scope, place: &str, looked: &[String], all: usize) -> Said {
    let text = match (scope, looked) {
        (Scope::Invalid(why), _) => {
            return Said {
                text: reason(why),
                bad: true,
            };
        }
        (Scope::OverPage, _) => copy::status::searched_page_of(place),
        (Scope::Place | Scope::Searched, [one]) if all > 1 => copy::status::searched_in(one),
        (Scope::Place | Scope::Searched, _) => copy::status::SEARCHED_ALL_MAIL.to_owned(),
    };
    Said { text, bad: false }
}

/// Why a pattern does not compile, in the few words the end of the search has room for: the
/// `regex` crate's message draws the pattern and a caret over several lines and says why on its
/// last ("error: unclosed group").
fn reason(why: &str) -> String {
    let last = why
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty());
    let last = last.unwrap_or(why.trim());
    last.strip_prefix("error:")
        .unwrap_or(last)
        .trim()
        .to_owned()
}

/// The header's own contents while nothing is picked and no search narrows the list: the place's
/// title, its tools, the magnifier and the ⋯ that closes the row.
#[component]
fn PlaceTools(
    shell: Signal<Shell>,
    place: String,
    pages: Signal<u32>,
    syncs: bool,
    side_hidden: Signal<bool>,
) -> Element {
    rsx! {
        div { class: "list-title",
            Label { text: place, style: LabelStyle::Title, common: classed("truncate") }
        }
        div { class: "bar-tools",
            // Trash and Spam alone: everything in them, deleted forever, once asked.
            super::destroy::EmptyButton { shell }
            // A search being typed can be kept as a view, and a view shown can be changed.
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
                    let client = crate::ui::live::client();
                    if open_composer(&client, shell, Writes::New).is_ok() {
                    }
                }),
            }
            // The quick filter's row, shown and hidden.
            FilterButton { shell, pages }
        }
        // The search, at the toolbar's trailing end where the Mac keeps it: a magnifier that
        // brings up the search panel. The ⋯ closes the row: Sort By, Group, Row Properties and
        // Sync Now.
        super::command::SearchBox { shell }
        ListMore { shell, syncs, side_hidden }
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
    highlight: &mail_client::search::Highlight,
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

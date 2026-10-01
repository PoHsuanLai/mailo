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
use super::press::{available, on_primary};
use super::row::{DraftRow, MailRow};
use super::server_search::{Asked, ServerSearch, found_threads};
use super::view_groups::group_list;
use crate::fetch::Tone;
use crate::provider::provider;
use crate::view::{Nothing, Shell};
use dioxus::prelude::*;
use ds::components::chrome::toolbar::view::Toolbar;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::overlays::empty_state::EmptyForm;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::collections::BTreeMap;
use std::sync::Arc;

/// What a list item is, by identity: the same key on every render, so quire's `List` keeps a
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
    let address = shell.read().account.and_then(|id| {
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
    let quiet = syncs_nothing(&rows(), shell.read().account, &shell.read().scope);
    // Said by the links of the accounts in view, and said of nothing when they have none.
    let fetching = try_consume_context::<super::fetching::Fetching>();
    let syncing = fetching.is_some_and(|fetching| fetching.busy_in(&shell.read()));
    let line = fetching
        .filter(|_| !quiet)
        .map(|fetching| fetching.status(&shell.read(), super::clock::now()));
    let bad = line.as_ref().is_some_and(|line| line.tone != Tone::Plain);
    let note = line.map(|line| line.text).filter(|text| !text.is_empty());
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
            .filter(|row| match shell.read().account {
                Some(id) => row.id == id,
                None => shell.read().scope.is_empty() || shell.read().scope.contains(&row.id),
            })
            .filter(|row| crate::server_search::searchable(&row.plan))
            .map(|row| (row.id, row.shown()))
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
            rsx! { SectionHeader { title: "Top results".to_owned() } },
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
            rsx! { SectionHeader { title: "Newest first".to_owned() } },
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
                rsx! { SectionHeader { title } },
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
            rsx! { SectionHeader { title: "From the server".to_owned() } },
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
    let nothing_here = threads().is_empty() && drafts().is_empty() && found().is_empty();
    let empty_form = match nothing() {
        Nothing::NoMatch(_) => EmptyForm::NoResults,
        Nothing::NoAccount | Nothing::EmptyFolder => EmptyForm::Empty,
    };
    let empty_description: Option<TextLine> = nothing().command().map(|command| {
        TextLine::Runs(vec![
            TextRun::new("Run ", RunTone::Plain),
            TextRun::new(command, RunTone::Code),
        ])
    });
    rsx! {
        div { class: "list-col",
            // The list's header is quire's 52 px `Toolbar`. While anything listed is picked, the
            // bar is the selection's: its count and its actions, and nothing else.
            Toolbar::<()> {
                onpick: move |()| {},
                center: rsx! {
                    div { class: if picking { "list-head picking" } else { "list-head" },
                        if picking {
                            PickBar { shell, revision, threads }
                        } else {
                            div { class: "list-title",
                                Label { text: place.clone(), style: LabelStyle::Title }
                                if let Some(address) = address {
                                    Label { text: address, role: LabelRole::Tertiary, style: LabelStyle::Caption }
                                }
                            }
                            if let Some(note) = note {
                                // One line, cut short when it must be; the whole of it on hover.
                                Label {
                                    text: note.clone(),
                                    role: LabelRole::Tertiary,
                                    style: LabelStyle::Footnote,
                                    common: classed(if bad { "status bad" } else { "status" }),
                                }
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
                                        availability: available(!syncing),
                                        onclick: on_primary(move || super::fetching::sync_now(&shell.read())),
                                    }
                                }
                                Button {
                                    bezel: Bezel::Toolbar,
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
            // quire's list in mailo's scroller: it keeps each row by its key, so a row that
            // leaves plays its exit and the rows below close the gap.
            div { class: "list",
                List::<Slot> {
                    label: place.clone(),
                    items,
                    cursor,
                    onselect: move |slot: Slot| {
                        if let Slot::Thread(id) | Slot::Top(id) | Slot::Server(id) = slot {
                            shell.write().open(id);
                        }
                    },
                }
                if nothing_here {
                    EmptyState {
                        form: empty_form,
                        title: nothing().message(),
                        description: empty_description,
                    }
                }
            }
            if !servers.is_empty() {
                ServerSearch { input: line.clone(), accounts: servers, asked, revision }
            }
            if more() {
                div { class: "more",
                    Button {
                        size: ControlSize::Small,
                        label: "Show more",
                        onclick: on_primary(move || pages += 1),
                    }
                }
            }
            Toast { shell, revision }
            Ghost {}
        }
    }
}

/// What a row shows beside its summary: the provider mark, the label chips, the search marks.
struct Dress {
    via: Option<crate::provider::Provider>,
    chips: Vec<String>,
    hit: Option<RowHit>,
}

/// The chip on a conversation a search of the server brought here.
pub(super) const FROM_SERVER: &str = "from the server";

fn dress(
    summary: &ThreadSummary,
    accounts: &[AccountRow],
    names: &BTreeMap<LabelId, String>,
    highlight: &crate::search::Highlight,
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

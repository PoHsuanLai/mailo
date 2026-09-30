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
use super::page::{PageMenus, group_page};
use super::press::{available, on_primary};
use super::row::{DraftRow, MailRow};
use crate::provider::provider;
use crate::view::{Nothing, Shell, SyncState, synced};
use dioxus::prelude::*;
use ds::components::chrome::toolbar::view::Toolbar;
use ds::components::content::label::{Label, LabelRole, LabelStyle};
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Bezel, ImagePosition};
use ds::components::overlays::empty_state::EmptyForm;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_domain::*;
use mail_store::SqliteStore;
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
    sync_state: Signal<SyncState>,
    marking: Memo<Marking>,
    /// A search's top results, drawn above the date-ordered rows.
    top: Memo<Vec<ThreadSummary>>,
) -> Element {
    let rows = use_memo(move || {
        let _ = revision();
        let store = consume_context::<Arc<SqliteStore>>();
        account_rows(&store)
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
    let note = if quiet {
        None
    } else {
        sync_state.read().message().map(|text| text.to_owned())
    };
    let search_note = marking.read().note();
    let invalid = matches!(marking.read().scope, Scope::Invalid(_));
    let bad = sync_state.read().is_failure();
    let accounts = rows();
    let highlight = marking.read().highlight.clone();
    let dress_row = |summary: &ThreadSummary| dress(summary, &accounts, &names, &highlight);
    let state = use_hook(super::motion::motion);
    let shown = threads();
    if let Some(state) = state {
        let mut order = state.order;
        order.set(shown.iter().map(|summary| summary.id).collect());
    }
    // The list's items, top to bottom: drafts, a search's top results, then the bands.
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
            items.push(ListItem::row(
                Slot::Top(id),
                label,
                rsx! { MailRow { summary, shell, revision, chips, via, hit } },
            ));
        }
        items.push(ListItem::heading(
            Slot::NewestHeading,
            rsx! { SectionHeader { title: "Newest first".to_owned() } },
        ));
    }
    for band in group_page(
        shown,
        shell.read().group,
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
            items.push(ListItem::row(
                Slot::Thread(id),
                label,
                rsx! { MailRow { summary, shell, revision, chips, via, hit } },
            ));
        }
    }
    let cursor = shell.read().open.map(Slot::Thread);
    let nothing_here = threads().is_empty() && drafts().is_empty();
    let empty_form = match nothing() {
        Nothing::NoMatch(_) => EmptyForm::NoResults,
        Nothing::NoAccount | Nothing::EmptyFolder => EmptyForm::Empty,
    };
    let empty_description: Option<TextLine> = match nothing().command() {
        Some(command) => Some(TextLine::Runs(vec![
            TextRun::new("Run ", RunTone::Plain),
            TextRun::new(command, RunTone::Code),
        ])),
        None => None,
    };
    rsx! {
        div { class: "list-col",
            // The list's header is quire's 52 px `Toolbar`; what it holds (the place, its status
            // and the buttons that open menus from themselves) is the band's centre.
            Toolbar::<()> {
                onpick: move |()| {},
                center: rsx! {
                  div { class: "list-head",
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
                    if !quiet {
                        Button {
                            bezel: Bezel::Toolbar,
                            image: ImagePosition::Only,
                            label: "Sync now",
                            icon: Some(IconSource::Glyph(Icon::Refresh)),
                            availability: available(sync_state.read().may_start()),
                            onclick: on_primary(move || {
                                if !sync_state.read().may_start() {
                                    return;
                                }
                                sync_state.set(SyncState::Running);
                                super::folder_open::forget();
                                let store = consume_context::<Arc<SqliteStore>>();
                                spawn(async move {
                                    // `spawn_blocking`, not this task: sync::run opens sockets and
                                    // builds its own runtime, and `Runtime::block_on` inside an async
                                    // context panics.
                                    let done = tokio::task::spawn_blocking(move || {
                                        crate::sync::run(store, chrono::Utc::now())
                                    })
                                    .await;
                                    sync_state.set(match done {
                                        Ok(result) => synced(result.map(|ran| ran.text)),
                                        Err(e) => synced(Err(format!("the sync pass stopped: {e}"))),
                                    });
                                    revision += 1;
                                });
                            }),
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
                        if let Slot::Thread(id) | Slot::Top(id) = slot {
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

fn dress(
    summary: &ThreadSummary,
    accounts: &[AccountRow],
    names: &BTreeMap<LabelId, String>,
    highlight: &crate::search::Highlight,
) -> Dress {
    Dress {
        via: accounts
            .iter()
            .find(|row| row.id == summary.account)
            .map(|row| provider(&row.plan)),
        chips: summary
            .labels
            .iter()
            .filter_map(|id| names.get(id).cloned())
            .collect(),
        hit: row_hit(summary, highlight),
    }
}

#[cfg(test)]
mod tests;

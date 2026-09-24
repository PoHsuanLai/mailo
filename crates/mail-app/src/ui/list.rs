//! The list pane.
//!
//! The search box, what the pane says when it has nothing to show, the rows, and the way to
//! ask for another page. Split from [`super::app`] (`CONVENTIONS.md` §8). The queries stay in
//! `App`; this reads the memos it is handed rather than cloning their answers in the parent.

use super::data::{AccountRow, account_rows, syncs_nothing};
use super::field::{Field, FieldKind};
use super::hover::{HoverLayer, Site, hover};
use super::list_search::{Marking, RowHit, Scope, row_hit};
use super::motion::{Ghost, Motion, Toast, motion};
use super::ops::start_new;
use super::page::{PageMenus, group_page};
use super::row::{DraftRow, Moving, Row};
use crate::provider::provider;
use crate::view::{Nothing, Shell, SyncState, synced};
use dioxus::prelude::*;
use ds::{Glyph, Icon};
use mail_domain::*;
use mail_store::SqliteStore;
use std::collections::BTreeMap;
use std::sync::Arc;

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
    entering: Signal<bool>,
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
    let inbox = place == "Inbox" && shell.read().search.trim().is_empty();
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
    enum Line {
        Head(String),
        Mail {
            index: usize,
            summary: Box<ThreadSummary>,
            via: Option<crate::provider::Provider>,
            chips: Vec<String>,
            hit: Option<RowHit>,
            moving: Moving,
            landing: Option<String>,
        },
    }
    let accounts = rows();
    let highlight = marking.read().highlight.clone();
    let dress_row = |summary: &ThreadSummary| dress(summary, &accounts, &names, &highlight);
    // The strip is the same row, marked the same way, under its own header. A top result is
    // also in the list below, where its date puts it, as Gmail shows it.
    let strip: Vec<(ThreadSummary, Dress)> = top()
        .into_iter()
        .map(|summary| {
            let dressed = dress_row(&summary);
            (summary, dressed)
        })
        .collect();
    let state = use_hook(motion);
    let shown = with_leaving(threads(), state);
    if let Some(state) = state {
        let mut order = state.order;
        order.set(shown.iter().map(|summary| summary.id).collect());
    }
    let warm = hover().is_some_and(|hover| *hover.warm.read());
    let mut lines = Vec::new();
    let mut row_index = 0usize;
    for band in group_page(
        shown,
        shell.read().group,
        &names,
        chrono::Utc::now(),
        &chrono::Local,
    ) {
        if let Some(title) = band.title {
            lines.push(Line::Head(title));
        }
        for summary in band.threads {
            let Dress { via, chips, hit } = dress_row(&summary);
            let moving = moving_of(state, summary.id);
            let landing = state
                .and_then(|state| *state.landing.read())
                .filter(|(thread, _)| *thread == summary.id)
                .and_then(|(_, label)| names.get(&label).cloned());
            lines.push(Line::Mail {
                index: row_index,
                summary: Box::new(summary),
                via,
                chips,
                hit,
                moving,
                landing,
            });
            row_index += 1;
        }
    }
    rsx! {
        div { class: if warm { "list-col warm" } else { "list-col" },
            div { class: "list-bar",
                h2 {
                    "{place}"
                    if let Some(address) = address {
                        span { class: "mono", "{address}" }
                    }
                }
                if let Some(note) = note {
                    // One line, cut short when it must be; the whole of it on hover.
                    span { class: if bad { "status bad" } else { "status" }, title: "{note}", "{note}" }
                }
                if let Some(said) = search_note {
                    span { class: if invalid { "status bad" } else { "status" }, "{said}" }
                }
                div { class: "bar-tools",
                    PageMenus { shell }
                    if !quiet {
                        button {
                            class: "mini",
                            aria_label: "Sync now",
                            disabled: !sync_state.read().may_start(),
                            onclick: move |_| {
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
                            },
                            Glyph { icon: Icon::Refresh, size: ds::IconSize::Compact }
                        }
                    }
                    button {
                        class: "mini",
                        title: "Write a new message (c)",
                        onclick: move |_| {
                            let store = consume_context::<Arc<SqliteStore>>();
                            let known = shell.peek().accounts.clone();
                            match start_new(&store, &known) {
                                Ok(draft) => {
                                    shell.write().compose(&draft);
                                    revision += 1;
                                }
                                Err(why) => eprintln!("compose: {why}"),
                            }
                        },
                        Glyph { icon: Icon::Pen, size: ds::IconSize::Compact }
                        "Compose"
                    }
                }
            }
            label { class: "search",
                Glyph { icon: Icon::Search, size: ds::IconSize::Nav }
                Field {
                    kind: FieldKind::Boxed,
                    value: shell.read().search.clone(),
                    placeholder: "Search all mail".to_owned(),
                    extra: Some("search".to_owned()),
                    on_input: move |value| {
                        shell.write().search = value;
                        pages.set(1);
                    },
                    on_focus: move |_| in_a_field.set(true),
                    on_blur: move |_| in_a_field.set(false),
                }
            }
            ul {
                class: if entering() { "list entering" } else { "list" },
                onanimationend: move |_| entering.set(false),
                if threads().is_empty() && drafts().is_empty() {
                    li { class: "empty",
                        p { "{nothing().message()}" }
                        if nothing().command().is_none() && matches!(nothing(), Nothing::EmptyFolder) {
                            p { class: "mono", if inbox { "inbox zero" } else { "empty" } }
                        }
                        if let Some(command) = nothing().command() {
                            pre { class: "command", "{command}" }
                        }
                    }
                }
                for (index, draft) in drafts().into_iter().enumerate() {
                    {
                        let id = draft.id;
                        rsx! { DraftRow { key: "{id}", draft, shell, index } }
                    }
                }
                if !strip.is_empty() {
                    li { class: "list-top-h", "Top results" }
                    for (index, (summary, Dress { via, chips, hit })) in strip.into_iter().enumerate() {
                        {
                            let id = summary.id;
                            rsx! {
                                Row {
                                    key: "top-{id}",
                                    summary,
                                    shell,
                                    revision,
                                    index,
                                    chips,
                                    via,
                                    hit,
                                    moving: Moving::Still,
                                    landing: None,
                                }
                            }
                        }
                    }
                    li { class: "list-top-h", "Newest first" }
                }
                for line in lines {
                    match line {
                        Line::Head(title) => rsx! { li { key: "band-{title}", class: "list-g", "{title}" } },
                        Line::Mail {
                            index,
                            summary,
                            via,
                            chips,
                            hit,
                            moving,
                            landing,
                        } => {
                            let summary = *summary;
                            let id = summary.id;
                            rsx! { Row { key: "{id}", summary, shell, revision, index, chips, via, hit, moving, landing } }
                        }
                    }
                }
            }
            if more() {
                button {
                    class: "mini",
                    onclick: move |_| pages += 1,
                    "Show more"
                }
            }
            HoverLayer { site: Site::List, shell, revision, spaces: None }
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

/// The list as the store returned it, with the rows that are still leaving put back where they
/// stood, so each one is drawn until its exit ends.
fn with_leaving(mut threads: Vec<ThreadSummary>, state: Option<Motion>) -> Vec<ThreadSummary> {
    let Some(state) = state else {
        return threads;
    };
    for row in state.leaving.read().iter() {
        if !threads.iter().any(|summary| summary.id == row.summary.id) {
            let at = row.index.min(threads.len());
            threads.insert(at, row.summary.clone());
        }
    }
    threads
}

/// What a row is doing, from the motion state.
fn moving_of(state: Option<Motion>, id: ThreadId) -> Moving {
    let Some(state) = state else {
        return Moving::Still;
    };
    if let Some(row) = state.leaving.read().iter().find(|row| row.summary.id == id) {
        return Moving::Going(row.op);
    }
    if let Some((_, stagger)) = state.healing.read().iter().find(|(row, _)| *row == id) {
        return Moving::Healing(*stagger);
    }
    if *state.returning.read() == Some(id) {
        return Moving::Returning;
    }
    Moving::Still
}

#[cfg(test)]
mod tests;

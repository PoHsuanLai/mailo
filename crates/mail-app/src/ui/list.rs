//! The list pane.
//!
//! The search box, what the pane says when it has nothing to show, the rows, and the way to
//! ask for another page. Split from [`super::app`] (`CONVENTIONS.md` §8). The queries stay in
//! `App`; this reads the memos it is handed rather than cloning their answers in the parent.

use super::data::{AccountRow, account_rows, syncs_nothing};
use super::field::{Field, FieldKind};
use super::hover::{HoverLayer, Site, hover};
use super::list_search::{Marking, RowHit, Scope, row_hit};
use super::motion::{Clock, Ghost, Leaving, Motion, Toast, motion};
use super::ops::start_new;
use super::page::{PageMenus, group_page};
use super::press::{available, on_primary};
use super::row::{DraftRow, Moving, Row, gap};
use crate::provider::provider;
use crate::view::{Nothing, Shell, SyncState, synced};
use dioxus::prelude::*;
use ds::{Anim, Glyph, Icon, Presence, Roster, RowPitch};
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
    let listed = threads();
    let keys: Vec<ThreadId> = listed.iter().map(|summary| summary.id).collect();
    let pitch = RowPitch(ds::Px(gap(&shell.peek()) as f32));
    let roster = use_roster_clock(keys.clone(), pitch, state);
    let leaving = state
        .map(|state| state.leaving.read().clone())
        .unwrap_or_default();
    let drawn = drawn(&roster, &keys, listed, &leaving);
    // The list is being shown while `entering` holds and its rows are still arriving; it rests
    // once they have, and a row that arrives after that is not an entrance.
    use_list_rest(entering, roster);
    let list_presence = if entering() {
        ds::ListPresence::Entering
    } else {
        ds::ListPresence::Present
    };
    let returning = state.and_then(|state| *state.returning.read());
    let moving: BTreeMap<ThreadId, Moving> = drawn
        .iter()
        .map(|(summary, moving)| {
            let moving = match moving {
                Moving::Entering(_) if returning == Some(summary.id) && !entering() => {
                    Moving::Returning
                }
                other => *other,
            };
            (summary.id, moving)
        })
        .collect();
    let shown: Vec<ThreadSummary> = drawn.into_iter().map(|(summary, _)| summary).collect();
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
            let moving = moving.get(&summary.id).copied().unwrap_or_default();
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
                        ds::Button {
                            variant: ds::ButtonVariant::Mini,
                            label: String::new(),
                            icon: Icon::Refresh,
                            aria_label: "Sync now".to_owned(),
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
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: "Compose".to_owned(),
                        icon: Icon::Pen,
                        title: "Write a new message (c)".to_owned(),
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
            // quire's list in mailo's scroller: its presence picks the entrance a row plays.
            div { class: "list",
            ds::AnimatedList {
                label: "{place}",
                presence: list_presence,
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
            }
            if more() {
                div { class: "more",
                    ds::Button {
                        variant: ds::ButtonVariant::Mini,
                        label: "Show more".to_owned(),
                        onclick: on_primary(move || pages += 1),
                    }
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

/// The list's roster over `keys`, and the timers an op starts, handed to the window's motion
/// state so an op anywhere can start them.
fn use_roster_clock(
    keys: Vec<ThreadId>,
    pitch: RowPitch,
    state: Option<Motion>,
) -> Roster<ThreadId> {
    let roster = ds::use_roster(keys, pitch);
    let gulp = ds::use_motion_timer(Anim::Gulp);
    let landing = ds::use_motion_timer(Anim::ChipLand);
    let gulped = use_callback(move |()| {
        if let Some(mut state) = state {
            state.gulp.set(None);
        }
    });
    let landed = use_callback(move |()| {
        if let Some(mut state) = state {
            state.landing.set(None);
        }
    });
    use_hook(move || {
        if let Some(state) = state {
            let mut clock = state.clock;
            clock.set(Some(Clock {
                roster,
                gulp,
                landing,
                gulped,
                landed,
            }));
        }
    });
    roster
}

/// Once the list being shown has no row still arriving, it is at rest. Marked from an effect,
/// after the render that saw it, and only once a row has been seen arriving: a place's rows
/// come from a query that may land a frame after the place was chosen.
fn use_list_rest(mut entering: Signal<bool>, roster: Roster<ThreadId>) {
    let mut seen = use_signal(|| false);
    use_effect(move || {
        let arriving = roster
            .entries()
            .iter()
            .any(|entry| entry.presence == Presence::Entering);
        if !entering() {
            seen.set(false);
        } else if arriving {
            seen.set(true);
        } else if *seen.peek() {
            seen.set(false);
            entering.set(false);
        }
    });
}

/// What the list draws, in order: every row the roster holds, with the summary to draw it by
/// and what it is doing. A listed row is drawn as the store has it; a row that has left is
/// drawn as it was, until its exit settles. A row an undo brought back mid-exit stays under its
/// own key (`Roster::stay`), drawn once.
fn drawn(
    roster: &Roster<ThreadId>,
    keys: &[ThreadId],
    listed: Vec<ThreadSummary>,
    leaving: &[Leaving],
) -> Vec<(ThreadSummary, Moving)> {
    roster
        .entries()
        .into_iter()
        .filter_map(|entry| {
            let summary = if keys.contains(&entry.key) {
                listed
                    .iter()
                    .find(|summary| summary.id == entry.key)?
                    .clone()
            } else {
                leaving
                    .iter()
                    .find(|row| row.key == entry.key)?
                    .summary
                    .clone()
            };
            let moving = match entry.presence {
                Presence::Present => Moving::Still,
                Presence::Entering => Moving::Entering(entry.index.get()),
                Presence::Leaving(exit) => Moving::Going(exit),
                Presence::Healing { d, .. } => Moving::Healing(d.get()),
            };
            Some((summary, moving))
        })
        .collect()
}

#[cfg(test)]
mod tests;

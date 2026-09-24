//! Send later, in the window: what the Sends row's choice means to the outbox, the "Pick a
//! time…" field, and the list of messages waiting for their time.
//!
//! The times are the snooze menu's: a typed phrase goes through [`crate::view::snooze_until`],
//! and every time is written by [`when_words`], so "Tomorrow 08:00" reads the same everywhere.

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_domain::{DraftId, SendState};
use mail_store::{SqliteStore, Store};

use super::super::field::{Field, FieldKind};
use super::super::menu::{MenuKey, menu_key};
use super::super::menus::{when_in_sentence, when_words};
use super::desk::{Desk, refusal, take_back};
use super::page::{Float, Page, When};
use crate::compose::Leaves;
use crate::view::Shell;
use ds::{Glyph, Icon};

/// The Sends menu's key for "Pick a time…".
pub(in crate::ui) const PICK_KEY: &str = "at";
/// And what it says.
pub(in crate::ui) const PICK_LABEL: &str = "Pick a time…";

/// What the example phrases are, where the field has nothing to go on.
const TRY: &str = "Type a time: tomorrow 9, fri 17:00, +2h";

/// What Send does with the Sends row's choice. A time that has already gone is refused, in
/// words, rather than sent at once: the person asked for later, and "now" is not later.
pub(in crate::ui) fn leaves<Tz: TimeZone>(
    when: When,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Leaves, String>
where
    Tz::Offset: std::fmt::Display,
{
    match when.due(now, zone) {
        None => Ok(Leaves::Now),
        Some(at) if at <= now => Err(passed(at, now, zone)),
        Some(at) => Ok(Leaves::At(at)),
    }
}

/// The time a phrase typed into "Pick a time…" names, if it is one still to come.
pub(in crate::ui) fn pick_time<Tz: TimeZone>(
    typed: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    if typed.trim().is_empty() {
        return Err(TRY.to_owned());
    }
    let at = crate::view::snooze_until(typed, now, zone)?;
    if at <= now {
        return Err(passed(at, now, zone));
    }
    Ok(at)
}

fn passed<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!(
        "{} has already passed. Pick a later time, or send it right away.",
        when_words(at, now, zone)
    )
}

/// Enter in the field: the typed time becomes the Sends row's choice. Refused, the field stays
/// open with its reason under it.
pub(in crate::ui) fn choose_time<Tz: TimeZone>(
    page: &mut Page,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    let Float::PickTime(typed) = &page.float else {
        return Err(TRY.to_owned());
    };
    let at = pick_time(typed, now, zone)?;
    page.when = When::At(at);
    page.float = Float::Closed;
    page.touch();
    Ok(at)
}

/// "Pick a time…": the field, and under it the time it reads or why it reads none.
#[component]
pub(in crate::ui) fn PickTime(page: Signal<Page>) -> Element {
    let Float::PickTime(typed) = page.read().float.clone() else {
        return rsx! {};
    };
    let now = Utc::now();
    let reading = pick_time(&typed, now, &chrono::Local);
    let (class, says) = match &reading {
        Ok(at) => (
            "pick-says",
            format!("Leaves {}", when_in_sentence(*at, now, &chrono::Local)),
        ),
        Err(why) if typed.trim().is_empty() => ("pick-says", why.clone()),
        Err(why) => ("pick-says refused", why.clone()),
    };
    rsx! {
        div { class: "fmenu slim pick-time",
            div { class: "g", "Send at" }
            div {
                class: "pick-in",
                onkeydown: move |event: KeyboardEvent| {
                    match menu_key(&event.key().to_string()) {
                        Some(MenuKey::Enter) => {
                            event.prevent_default();
                            let _ = choose_time(&mut page.write(), Utc::now(), &chrono::Local);
                        }
                        Some(MenuKey::Escape) => {
                            event.stop_propagation();
                            page.write().float = Float::Sends;
                        }
                        _ => {}
                    }
                },
                Glyph { icon: Icon::Clock, size: ds::IconSize::Nav }
                Field {
                    kind: FieldKind::Inline,
                    value: typed.clone(),
                    placeholder: "tomorrow 9, fri 17:00, +2h".to_owned(),
                    extra: Some("pick-field".to_owned()),
                    on_input: move |value: String| page.write().float = Float::PickTime(value),
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
            }
            p { class: "{class}", role: "status", "{says}" }
        }
    }
}

/// A draft held for later: its id, what it is called, and when it leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Waiting {
    pub draft: DraftId,
    pub title: String,
    pub at: DateTime<Utc>,
}

/// Every draft held for later, on every account that sends, soonest first.
pub(in crate::ui) fn waiting(store: &SqliteStore) -> Vec<Waiting> {
    let mut out: Vec<Waiting> = crate::compose::sending_accounts(store)
        .into_iter()
        .flat_map(|(_, account)| store.drafts(account).unwrap_or_default())
        .filter_map(|draft| match draft.state {
            SendState::Scheduled { at } => Some(Waiting {
                draft: draft.id,
                title: if draft.subject.trim().is_empty() {
                    "(no subject)".to_owned()
                } else {
                    draft.subject
                },
                at,
            }),
            _ => None,
        })
        .collect();
    out.sort_by_key(|one| one.at);
    out
}

/// Cancel from the list: the draft goes back to being one, and opens as a page. Refused, the
/// reason is said under the row and in the toast, and nothing changes.
pub(in crate::ui) fn cancel_waiting(
    desk: Desk,
    shell: Signal<Shell>,
    draft: DraftId,
) -> Result<(), String> {
    take_back(desk, shell, draft).map_err(|why| {
        let said = refusal(&why);
        super::super::motion::tell(said.clone(), super::super::motion::Follow::Nothing);
        said
    })
}

/// The clock entries in Today: messages waiting for their time, each with Cancel.
#[component]
pub(in crate::ui) fn ScheduledDrafts(shell: Signal<Shell>) -> Element {
    let Some(desk) = try_use_context::<Desk>() else {
        return rsx! {};
    };
    let mut refused = use_signal(|| None::<(DraftId, String)>);
    // Read so a send scheduled or taken back redraws the list.
    let _ = desk.outbox.read();
    let store = consume_context::<Arc<SqliteStore>>();
    let now = Utc::now();
    let rows: Vec<(Waiting, String)> = waiting(&store)
        .into_iter()
        .map(|one| {
            let words = when_words(one.at, now, &chrono::Local);
            (one, words)
        })
        .collect();
    rsx! {
        for (one, words) in rows {
            {
                let draft = one.draft;
                let why = refused().filter(|(which, _)| *which == draft).map(|(_, why)| why);
                rsx! {
                    div {
                        key: "{draft}",
                        class: "item today-item later",
                        title: "Waiting to be sent at {words}",
                        span { class: "fav later", Glyph { icon: Icon::Clock, size: ds::IconSize::Micro } }
                        span { class: "t", "{one.title}" }
                        span { class: "when", "{words}" }
                        button {
                            class: "x",
                            r#type: "button",
                            aria_label: "Cancel sending {one.title}",
                            title: "Cancel: it goes back to being a draft",
                            onclick: move |event| {
                                event.stop_propagation();
                                match cancel_waiting(desk, shell, draft) {
                                    Ok(()) => refused.set(None),
                                    Err(why) => refused.set(Some((draft, why))),
                                }
                            },
                            Glyph { icon: Icon::X, size: ds::IconSize::Tiny }
                        }
                    }
                    if let Some(why) = why {
                        p { key: "{draft}-why", class: "today-hint refused", "{why}" }
                    }
                }
            }
        }
    }
}

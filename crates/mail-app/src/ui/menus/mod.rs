//! The menus a row opens when a click needs a payload.
//!
//! A label and a snooze time are not things a button can carry, so the row opens one of these
//! instead of performing the operation itself. Split from [`super::app`] (`CONVENTIONS.md` §8).

use super::menu::{Floating, MenuItem, Right, Tile, anchor_for, palette_groups};
use super::motion::{act, act_all};
use super::picks::{label_all, with_selection};
use crate::ui::view::Shell;
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::components::content::avatar::AvatarSize;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use mail_domain::*;
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::sync::Arc;

/// The four times the snooze menu offers, as `(what it says, the phrase it means)`.
const SNOOZE: &[(&str, &str)] = &[
    ("Later today", "later"),
    ("Tomorrow", "tomorrow"),
    ("This weekend", "weekend"),
    ("Next week", "monday"),
];

/// How a resolved snooze time is written on the row.
pub(super) fn snooze_help<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.with_timezone(zone).format("%Y-%m-%d %H:%M").to_string()
}

/// The time a snooze lands on, short enough for the end of a menu row: `7:30 PM` today or
/// tomorrow (the name says which), `Sat 9:00 AM` within the week, `Mon 12 Oct` past it.
pub(super) fn snooze_hint<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let there = at.with_timezone(zone);
    let days = (there.date_naive() - now.with_timezone(zone).date_naive()).num_days();
    match days {
        ..=1 => there.format("%-I:%M %p").to_string(),
        2..=6 => there.format("%a %-I:%M %p").to_string(),
        _ => there.format("%a %-d %b").to_string(),
    }
}

/// A time to come as a person says it: `Today 17:00`, `Tomorrow 08:00`, `Tue 08:00` within the
/// week, `Tue 6 Oct 08:00` past it, with the year only when it is not this one. The one wording
/// for a time something waits until: the Sends menu, the outbox pill and the scheduled list.
pub(in crate::ui) fn when_words<Tz: TimeZone>(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike as _;
    let there = at.with_timezone(zone);
    let here = now.with_timezone(zone);
    let clock = there.format("%H:%M");
    match (there.date_naive() - here.date_naive()).num_days() {
        -1 => format!("Yesterday {clock}"),
        0 => format!("Today {clock}"),
        1 => format!("Tomorrow {clock}"),
        2..=6 => format!("{} {clock}", there.format("%a")),
        _ if there.year() == here.year() => format!("{} {clock}", there.format("%a %-d %b")),
        _ => format!("{} {clock}", there.format("%a %-d %b %Y")),
    }
}

/// [`when_words`] inside a sentence: "Scheduled for tomorrow 08:00".
pub(in crate::ui) fn when_in_sentence<Tz: TimeZone>(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let words = when_words(at, now, zone);
    for day in ["Yesterday", "Today", "Tomorrow"] {
        if let Some(rest) = words.strip_prefix(day) {
            return format!("{}{rest}", day.to_lowercase());
        }
    }
    words
}

/// The snooze menu's rows. The hint is [`snooze_hint`] of [`mail_core::snooze::snooze_until`].
pub(super) fn snooze_items<Tz: TimeZone>(now: DateTime<Utc>, zone: &Tz) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    SNOOZE
        .iter()
        .filter_map(|(says, phrase)| {
            let at = mail_core::snooze::snooze_until(phrase, now, zone).ok()?;
            Some(MenuItem {
                key: (*phrase).to_owned(),
                tile: Tile::Icon(Icon::Clock),
                name: (*says).to_owned(),
                help: None,
                right: Right::Hint(snooze_hint(at, now, zone)),
                group: None,
                marks: Vec::new(),
                title: Vec::new(),
                detail: Vec::new(),
            })
        })
        .collect()
}

/// The label menu's rows, including "Create “…”" when `typed` names nothing we have.
pub(super) fn label_items(
    known: &[(String, LabelId)],
    summary: &ThreadSummary,
    typed: &str,
) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = crate::ui::view::label_menu(known, summary)
        .into_iter()
        .map(|choice| MenuItem {
            key: choice.id.to_string(),
            tile: Tile::Icon(Icon::Tag),
            name: choice.name,
            help: None,
            right: Right::Check(choice.membership == Membership::In),
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect();
    let typed = typed.trim();
    let known_name = known
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(typed));
    if !typed.is_empty() && !known_name {
        items.push(MenuItem {
            key: format!("create:{typed}"),
            tile: Tile::Icon(Icon::Tag),
            name: format!("Create “{typed}”"),
            help: Some("New label".to_owned()),
            right: Right::None,
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        });
    }
    items
}

/// When to bring a conversation back, anchored to the button that asked.
#[component]
pub(super) fn SnoozeMenu(
    id: ThreadId,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    anchor: Option<MountedRef>,
    /// The snooze button's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
    /// Told when the menu goes, picked or dismissed: a host that opened it itself (the reader's
    /// ⋯) closes it so.
    #[props(default)]
    on_done: Option<EventHandler<()>>,
) -> Element {
    let mut done = move || {
        shell.write().snoozing = None;
        if let Some(on_done) = on_done {
            on_done.call(());
        }
    };
    let now = Utc::now();
    // The Mac's menu has no second line, so each choice says its time at its trailing end.
    let items = snooze_items(now, &chrono::Local);
    rsx! {
        Floating {
            anchor,
            placed,
            title: "Snooze until".to_owned(),
            items,
            on_pick: move |phrase: String| {
                let store = consume_context::<Arc<SqliteStore>>();
                // The same time `mailo snooze` resolves, applied through the same op; only the
                // undo, the curl and the toast are the window's.
                match mail_core::snooze::snooze_until(&phrase, Utc::now(), &chrono::Local) {
                    Ok(at) => {
                        done();
                        // The whole selection when this row is picked, as one gesture.
                        let ops = with_selection(shell, id)
                            .into_iter()
                            .map(|thread| (thread, Op::SetSnooze(Snooze::Until(at))))
                            .collect();
                        act_all(&store, shell, revision, ops);
                    }
                    // The vocabulary is fixed and the clock is the only other input, so this
                    // is "the year 262143 has no tomorrow".
                    Err(why) => eprintln!("snooze: {why}"),
                }
            },
            on_close: move |_| done(),
        }
    }
}

/// The labels on this conversation, and the ones it could wear: a checklist that stays open as
/// each is put on or taken off, anchored to the button that opened it.
#[component]
pub(super) fn LabelMenu(
    id: ThreadId,
    summary: ThreadSummary,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    anchor: Option<MountedRef>,
    /// The Label button's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
    /// Told when the list goes, as [`SnoozeMenu`]'s is.
    #[props(default)]
    on_done: Option<EventHandler<()>>,
) -> Element {
    let mut done = move || {
        shell.write().labelling = None;
        if let Some(on_done) = on_done {
            on_done.call(());
        }
    };
    let mut typed = use_signal(String::new);
    let (create, base): (Vec<MenuItem>, Vec<MenuItem>) =
        label_items(&shell.read().labels, &summary, &typed())
            .into_iter()
            .partition(|item| item.key.starts_with("create:"));
    let mut items = super::menu::narrowed(&base, &typed());
    items.extend(create);
    let account = summary.account;
    let empty = if shell.read().labels.is_empty() {
        "No labels"
    } else {
        "No label matches."
    };
    rsx! {
        PickList::<String> {
            anchor: anchor_for(anchor, placed),
            label: "Labels",
            placeholder: "Filter labels…",
            query: typed(),
            groups: palette_groups(&items, AvatarSize::Size22, None),
            empty,
            oninput: move |value: String| typed.set(value),
            onpick: move |key: String| {
                let store = consume_context::<Arc<SqliteStore>>();
                // A label is one account's, so a picked row on another account is left alone.
                let picked = with_selection(shell, id);
                let targets: Vec<ThreadId> = picked
                    .into_iter()
                    .filter(|thread| {
                        *thread == id
                            || super::move_to::account_of(&store, *thread) == Some(account.clone())
                    })
                    .collect();
                if let Some(name) = key.strip_prefix("create:") {
                    let Some(created) = create_label(&store, account.clone(), name) else {
                        return;
                    };
                    let ops = targets
                        .into_iter()
                        .map(|thread| (thread, Op::Label(created, Membership::In)))
                        .collect();
                    act_all(&store, shell, revision, ops);
                    done();
                    return;
                }
                let Some(which) = shell
                    .read()
                    .labels
                    .iter()
                    .find(|(_, id)| id.to_string() == key)
                    .map(|(_, id)| *id)
                else {
                    return;
                };
                if targets.len() > 1 {
                    label_all(&store, shell, revision, &targets, which);
                    return;
                }
                let on = summary.labels.contains(&which);
                let wanted = if on { Membership::Out } else { Membership::In };
                act(&store, shell, revision, id, Op::Label(which, wanted));
            },
            onclose: move |()| done(),
        }
    }
}

/// A label the user just named. The store creates provider labels during ingest; this one is
/// the user's, so its origin is `user`.
fn create_label(store: &SqliteStore, account: AccountId, name: &str) -> Option<LabelId> {
    let id = LabelId::generate();
    store
        .connection()
        .execute(
            "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, ?3, '\"user\"')",
            rusqlite::params![id.to_string(), account.to_string(), name],
        )
        .ok()?;
    Some(id)
}

#[cfg(test)]
mod tests;

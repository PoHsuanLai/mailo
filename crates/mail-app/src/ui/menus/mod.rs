//! The menus a row opens when a click needs a payload.
//!
//! A label and a snooze time are not things a button can carry, so the row opens one of these
//! instead of performing the operation itself. Split from [`super::app`] (`CONVENTIONS.md` §8).
//!
//! Both are plain lists of words: no header restating what was picked to open them, and no
//! glyph repeated down every row (design-language §2.2, §2.6).

mod further;
pub(super) use further::FurtherMenus;

use super::menu::{Floating, MenuItem, MenuKey, Right, anchor_for, menu_key, palette_groups};
use super::motion::{act, act_all};
use super::picks::{label_all, with_selection};
use crate::ui::copy;
use crate::ui::view::Shell;
use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::avatar::AvatarSize;
use ds::components::fields::text_field_model::Invalid;
use ds::components::overlays::popover::Arrow;
use ds::host::measure::{Anchor, MountedRef};
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use mail_core::SqliteStore;
use mail_core::when::Clock;
use mail_domain::*;
use porter_core::AccountId;
use std::sync::Arc;

/// The four times the snooze menu offers, as `(what it says, the phrase it means)`.
const SNOOZE: &[(&str, &str)] = &[
    (copy::menus::LATER_TODAY, "later"),
    (copy::menus::TOMORROW, "tomorrow"),
    (copy::menus::THIS_WEEKEND, "weekend"),
    (copy::menus::NEXT_WEEK, "monday"),
];

/// The snooze menu's key for Choose Time…, which no phrase is.
pub(super) const CHOOSE: &str = "choose";

/// How a resolved snooze time is written on the row, its hour by `clock`.
pub(super) fn snooze_help<Tz: TimeZone>(at: DateTime<Utc>, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    mail_core::when::stamp(at, zone, mail_core::when::Stamp::Full, clock)
}

/// The time a snooze lands on, short enough for the end of a menu row: `19:30` (or `7:30 PM`)
/// today or tomorrow (the name says which), `Sat 09:00` within the week, `Mon 12 Oct` past it.
///
/// The one menu that wrote twelve hours while every other time in the window wrote
/// twenty-four; now the hour is `clock`'s, as everywhere else.
pub(super) fn snooze_hint<Tz: TimeZone>(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let there = at.with_timezone(zone);
    let days = (there.date_naive() - now.with_timezone(zone).date_naive()).num_days();
    let hour = mail_core::when::time_of_day(&there, clock);
    match days {
        ..=1 => hour,
        2..=6 => format!("{} {hour}", there.format("%a")),
        _ => there.format("%a %-d %b").to_string(),
    }
}

/// The one wording for a time something waits until, which lives in `mail_core::when`.
pub(in crate::ui) use mail_core::when::upcoming as when_words;

/// [`when_words`] inside a sentence: "Scheduled for tomorrow 08:00".
pub(in crate::ui) fn when_in_sentence<Tz: TimeZone>(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let words = mail_core::when::upcoming(at, now, zone, clock);
    for day in ["Yesterday", "Today", "Tomorrow"] {
        if let Some(rest) = words.strip_prefix(day) {
            return format!("{}{rest}", day.to_lowercase());
        }
    }
    words
}

/// The snooze menu's rows: the four times, each with [`snooze_hint`] of
/// [`mail_core::snooze::snooze_until`] at its end, then Choose Time….
pub(super) fn snooze_items<Tz: TimeZone>(
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let row = |key: &str, name: &str, right: Right| MenuItem {
        right,
        ..MenuItem::plain(key, name)
    };
    SNOOZE
        .iter()
        .filter_map(|(says, phrase)| {
            let at = mail_core::snooze::snooze_until(phrase, now, zone).ok()?;
            Some(row(
                phrase,
                says,
                Right::Hint(snooze_hint(at, now, zone, clock)),
            ))
        })
        .chain([row(CHOOSE, copy::menus::CHOOSE_TIME, Right::None)])
        .collect()
}

/// The time a phrase typed into Choose Time… names, if it is one still to come. Read as
/// `mailo snooze` reads it, so "fri 17:00" means the same in both.
pub(super) fn chosen_time<Tz: TimeZone>(
    typed: &str,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    if typed.trim().is_empty() {
        return Err(copy::fields::TYPE_A_TIME.to_owned());
    }
    let at = mail_core::snooze::snooze_until(typed, now, zone)?;
    if at <= now {
        return Err(copy::errors::time_passed(&when_words(at, now, zone, clock)));
    }
    Ok(at)
}

/// The label menu's rows, including "Create “…”" when `typed` names nothing we have.
pub(super) fn label_items(
    known: &[(String, LabelId)],
    summary: &ThreadSummary,
    typed: &str,
) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = mail_core::view::label_menu(known, summary)
        .into_iter()
        .map(|choice| MenuItem {
            right: Right::Check(choice.membership == Membership::In),
            ..MenuItem::plain(choice.id.to_string(), choice.name)
        })
        .collect();
    let typed = typed.trim();
    let known_name = known
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(typed));
    if !typed.is_empty() && !known_name {
        items.push(MenuItem {
            help: Some("New label".to_owned()),
            ..MenuItem::plain(format!("create:{typed}"), format!("Create “{typed}”"))
        });
    }
    items
}

/// When to bring a conversation back, anchored to the button that asked: four times, and
/// Choose Time… for any other, typed where the menu stood.
#[component]
pub(super) fn SnoozeMenu(
    id: ThreadId,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    anchor: Option<MountedRef>,
    /// The snooze button's rect once measured, which wins over `anchor`.
    #[props(default)]
    placed: Option<Rect>,
) -> Element {
    let mut done = move || shell.write().further = None;
    // Choose Time… picked: the field stands where the menu stood.
    let mut choosing = use_signal(|| false);
    // The whole selection when this row is picked, as one gesture, through the op `mailo snooze`
    // applies; only the undo, the curl and the toast are the window's.
    let snooze = move |at: DateTime<Utc>| {
        let store = consume_context::<Arc<SqliteStore>>();
        let ops = with_selection(shell, id)
            .into_iter()
            .map(|thread| (thread, Op::SetSnooze(Snooze::Until(at))))
            .collect();
        act_all(&store, shell, revision, ops);
    };
    if choosing() {
        return rsx! {
            ChooseTime {
                anchor: anchor_for(anchor, placed),
                on_chosen: move |at: DateTime<Utc>| {
                    done();
                    snooze(at);
                },
                on_close: move |()| done(),
            }
        };
    }
    let now = Utc::now();
    // The Mac's menu has no second line, so each choice says its time at its trailing end.
    let items = snooze_items(now, &chrono::Local, crate::ui::clock::hours());
    rsx! {
        Floating {
            anchor,
            placed,
            title: String::new(),
            items,
            on_pick: move |phrase: String| {
                if phrase == CHOOSE {
                    choosing.set(true);
                    return;
                }
                match mail_core::snooze::snooze_until(&phrase, Utc::now(), &chrono::Local) {
                    Ok(at) => {
                        done();
                        snooze(at);
                    }
                    // The vocabulary is fixed and the clock is the only other input, so this
                    // is "the year 262143 has no tomorrow".
                    Err(why) => log::warn!("snooze: {why}"),
                }
            },
            // A pick closes the menu after it is heard; Choose Time… leaves its field up.
            on_close: move |_| {
                if !choosing() {
                    done();
                }
            },
        }
    }
}

/// Choose Time…: a popover with one field taking a time in words, and under it the time it reads
/// or why it reads none. Return snoozes until it; Esc or a click away closes it, snoozing nothing.
#[component]
fn ChooseTime(
    anchor: Anchor,
    on_chosen: EventHandler<DateTime<Utc>>,
    on_close: EventHandler<()>,
) -> Element {
    let mut typed = use_signal(String::new);
    let now = Utc::now();
    let hours = crate::ui::clock::hours();
    let reading = chosen_time(&typed(), now, &chrono::Local, hours);
    // What it reads is help under the field; a phrase it cannot read is the field's own refusal.
    let (validity, says) = match &reading {
        Ok(at) => (
            Validity::Valid,
            Some(copy::fields::back_at(&when_in_sentence(
                *at,
                now,
                &chrono::Local,
                hours,
            ))),
        ),
        Err(why) if typed().trim().is_empty() => (Validity::Valid, Some(why.clone())),
        Err(why) => (
            Validity::Invalid(Invalid {
                message: why.clone().into(),
                stamp: EventStamp(u32::try_from(typed().len()).unwrap_or(0)),
            }),
            None,
        ),
    };
    rsx! {
        Popover {
            anchor,
            placement: Placement::new(Side::Bottom, Align::Start),
            gap: Px(4.0),
            arrow: Arrow::None,
            onclose: move |()| on_close.call(()),
            div {
                class: "pick-time",
                onkeydown: move |event: KeyboardEvent| {
                    match menu_key(&event.key().to_string()) {
                        Some(MenuKey::Enter) => {
                            event.prevent_default();
                            if let Ok(at) = chosen_time(&typed(), Utc::now(), &chrono::Local, hours) {
                                on_chosen.call(at);
                            }
                        }
                        Some(MenuKey::Escape) => {
                            event.stop_propagation();
                            on_close.call(());
                        }
                        _ => {}
                    }
                },
                TextField {
                    label: copy::fields::SNOOZE_UNTIL.to_owned(),
                    placeholder: copy::fields::A_TIME.to_owned(),
                    value: typed(),
                    validity,
                    help: says.map(TextLine::from),
                    focus: FieldFocus::OnMount,
                    oninput: move |value: String| typed.set(value),
                    common: Common { extra_class: ExtraClass::parse("pick-field").ok(), ..Common::default() },
                }
            }
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
) -> Element {
    let mut done = move || shell.write().further = None;
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
    store.create_label(account, name).ok()
}

#[cfg(test)]
mod tests;

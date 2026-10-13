//! Remind Me If No Reply, chosen as the message is written: under Send ▾ only, and a Remind row
//! that is drawn only while a reminder is set, whose menu turns it Off.
//!
//! The times are the thread menu's ([`mail_core::follow_up::CHOICES`]), counted from when the message
//! leaves rather than from when the row was set, so a message scheduled for Monday and "In 3 days"
//! comes back on Thursday. A typed time goes through [`mail_core::follow_up::due`], which reads it as
//! snooze does. The reminder is held until the message has left (`mail_core::follow_up`).

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;

use super::super::menu::{MenuItem, MenuKey, Right, Tile, menu_key};
use super::super::menus::{snooze_help, when_in_sentence, when_words};
use super::page::{Float, Page};
use super::props::ChoiceRow;
use ds::base::geometry::placement::{Align, Side};
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;

/// The Remind menu's key for "Off".
pub(in crate::ui) const OFF_KEY: &str = "off";
/// And for "Pick a time…".
pub(in crate::ui) const PICK_KEY: &str = "at";

/// Whether, and when, Send asks to be reminded if nobody answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Remind {
    Off,
    /// One of [`mail_core::follow_up::CHOICES`], by its key, counted from when the message leaves.
    After(&'static str),
    /// A time typed into "Pick a time…".
    At(DateTime<Utc>),
}

impl Remind {
    /// The reminder's time for a message leaving at `leaves`, or `None` for no reminder. A typed
    /// time that is not after the message leaves is refused, in words.
    pub(in crate::ui) fn due<Tz: TimeZone>(
        self,
        leaves: DateTime<Utc>,
        zone: &Tz,
    ) -> Result<Option<DateTime<Utc>>, String>
    where
        Tz::Offset: std::fmt::Display,
    {
        match self {
            Remind::Off => Ok(None),
            Remind::After(key) => mail_core::follow_up::due(key, leaves, zone)
                .map(Some)
                .map_err(String::from),
            Remind::At(at) if at <= leaves => Err(format!(
                "The reminder, {}, comes before the message leaves. Pick a later time.",
                when_words(at, leaves, zone)
            )),
            Remind::At(at) => Ok(Some(at)),
        }
    }

    /// What the row's button says.
    pub(in crate::ui) fn shown<Tz: TimeZone>(self, now: DateTime<Utc>, zone: &Tz) -> String
    where
        Tz::Offset: std::fmt::Display,
    {
        match self {
            Remind::Off => "Off".to_owned(),
            Remind::After(key) => mail_core::follow_up::CHOICES
                .iter()
                .find(|(choice, _)| *choice == key)
                .map(|(_, says)| format!("{says}, if no reply"))
                .unwrap_or_default(),
            Remind::At(at) => format!("{}, if no reply", when_words(at, now, zone)),
        }
    }
}

/// The reminder's choices: the named times, each with the date it means if the message left now,
/// then "Pick a Time…", the current one checked. What Send ▾ › Remind Me If No Reply lists.
pub(in crate::ui) fn remind_choices<Tz: TimeZone>(
    remind: Remind,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let row = |key: &str, icon, name: &str, help: Option<String>, on: bool| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help,
        right: Right::Check(on),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    };
    let mut items: Vec<MenuItem> = mail_core::follow_up::CHOICES
        .iter()
        .filter_map(|(key, says)| {
            let at = mail_core::follow_up::due(key, now, zone).ok()?;
            Some(row(
                key,
                Icon::Bell,
                says,
                Some(snooze_help(at, zone)),
                remind == Remind::After(key),
            ))
        })
        .collect();
    let (help, on) = match remind {
        Remind::At(at) => (Some(when_words(at, now, zone)), true),
        _ => (None, false),
    };
    items.push(row(PICK_KEY, Icon::Clock, "Pick a Time…", help, on));
    items
}

/// The Remind row's menu: "Off" first, which is the way back to the default, then the choices.
pub(in crate::ui) fn remind_items<Tz: TimeZone>(
    remind: Remind,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let mut items = vec![MenuItem {
        key: OFF_KEY.to_owned(),
        tile: Tile::Icon(Icon::X),
        name: "Off".to_owned(),
        help: None,
        right: Right::Check(remind == Remind::Off),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }];
    items.extend(remind_choices(remind, now, zone));
    items
}

/// A choice from the Remind menu or from Send ▾ › Remind Me If No Reply.
pub(in crate::ui) fn pick_remind(page: &mut Page, key: &str) {
    if key == PICK_KEY {
        page.float = Float::PickRemind(String::new());
        return;
    }
    if key == OFF_KEY {
        page.remind = Remind::Off;
    } else if let Some((choice, _)) = mail_core::follow_up::CHOICES
        .iter()
        .find(|(choice, _)| *choice == key)
    {
        page.remind = Remind::After(choice);
    }
    page.touch();
    page.float = Float::Closed;
}

/// Enter in the field: the typed time becomes the reminder. Refused, the field stays open with
/// its reason under it.
pub(in crate::ui) fn choose_remind<Tz: TimeZone>(
    page: &mut Page,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    let Float::PickRemind(typed) = &page.float else {
        return Err("Type a time".to_owned());
    };
    let at = mail_core::follow_up::due(typed, now, zone)?;
    page.remind = Remind::At(at);
    page.float = Float::Closed;
    page.touch();
    Ok(at)
}

/// The Remind row, only while a reminder is set: when it comes, and the menu that turns it Off.
#[component]
pub(in crate::ui) fn RemindRow(page: Signal<Page>) -> Element {
    let remind = page.read().remind;
    if remind == Remind::Off {
        return rsx! {};
    }
    let now = super::super::clock::now();
    let items = remind_items(remind, now, &chrono::Local);
    let shown = remind.shown(now, &chrono::Local);
    rsx! {
        ChoiceRow {
            page,
            label: "Remind",
            row: "remind",
            value: shown,
            float: Float::Remind,
            items,
            on_pick: move |key: String| {
                let mut page = page;
                pick_remind(&mut page.write(), &key);
            },
        }
    }
}

/// "Pick a Time…" for the reminder: a popover hung from `anchor` with the field, and under it the
/// time it reads or why not. Esc or a click away closes it and leaves the reminder as it was.
#[component]
pub(in crate::ui) fn PickRemind(page: Signal<Page>, anchor: Option<MountedRef>) -> Element {
    let Float::PickRemind(typed) = page.read().float.clone() else {
        return rsx! {};
    };
    let now = super::super::clock::now();
    let reading = mail_core::follow_up::due(&typed, now, &chrono::Local);
    let (class, says) = match &reading {
        Ok(at) => (
            "pick-says",
            format!(
                "Back {} if nobody answers",
                when_in_sentence(*at, now, &chrono::Local)
            ),
        ),
        Err(why) if typed.trim().is_empty() => ("pick-says", why.to_string()),
        Err(why) => ("pick-says refused", why.to_string()),
    };
    rsx! {
        Popover {
            anchor: super::super::menu::anchor_at(anchor),
            placement: Placement::new(Side::Bottom, Align::Start),
            gap: Px(4.0),
            arrow: Arrow::None,
            onclose: move |()| page.write().float = Float::Closed,
            div {
                class: "pick-time",
                onkeydown: move |event: KeyboardEvent| {
                    match menu_key(&event.key().to_string()) {
                        Some(MenuKey::Enter) => {
                            event.prevent_default();
                            let now = super::super::clock::now();
                            let _ = choose_remind(&mut page.write(), now, &chrono::Local);
                        }
                        Some(MenuKey::Escape) => {
                            event.stop_propagation();
                            page.write().float = Float::Closed;
                        }
                        _ => {}
                    }
                },
                TextField {
                    label: "Remind me at".to_owned(),
                    value: typed.clone(),
                    placeholder: "tomorrow 9, fri 17:00, +3d".to_owned(),
                    focus: FieldFocus::OnMount,
                    common: Common {
                        extra_class: ExtraClass::parse("remind-field").ok(),
                        ..Common::default()
                    },
                    oninput: move |value: String| page.write().float = Float::PickRemind(value),
                }
                p { class: "{class}", role: "status", "{says}" }
            }
        }
    }
}

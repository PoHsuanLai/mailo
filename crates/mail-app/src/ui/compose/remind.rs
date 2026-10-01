//! The Remind row: "remind me if no reply", chosen as the message is written.
//!
//! The times are the thread menu's ([`crate::follow_up::CHOICES`]), counted from when the message
//! leaves rather than from when the row was set, so a message scheduled for Monday and "In 3 days"
//! comes back on Thursday. A typed time goes through [`crate::follow_up::due`], which reads it as
//! snooze does. The reminder is held until the message has left (`crate::follow_up`).

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;

use super::super::menu::{Floating, MenuItem, MenuKey, Right, Tile, menu_key};
use super::super::menus::{snooze_help, when_in_sentence, when_words};
use super::super::press::on_primary;
use super::page::{Float, Page};
use ds::components::controls::button_marks::Trailing;
use ds::components::controls::button_model::Bezel;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::icon::render::Glyph;

/// The Remind menu's key for "Don't remind me".
pub(in crate::ui) const OFF_KEY: &str = "off";
/// And for "Pick a time…".
pub(in crate::ui) const PICK_KEY: &str = "at";

/// Whether, and when, Send asks to be reminded if nobody answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Remind {
    Off,
    /// One of [`crate::follow_up::CHOICES`], by its key, counted from when the message leaves.
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
            Remind::After(key) => crate::follow_up::due(key, leaves, zone).map(Some),
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
            Remind::Off => "Don't remind me".to_owned(),
            Remind::After(key) => crate::follow_up::CHOICES
                .iter()
                .find(|(choice, _)| *choice == key)
                .map(|(_, says)| format!("{says}, if no reply"))
                .unwrap_or_default(),
            Remind::At(at) => format!("{}, if no reply", when_words(at, now, zone)),
        }
    }
}

/// The Remind menu: off, the named times with the date each means if the message left now, and
/// "Pick a time…".
pub(in crate::ui) fn remind_items<Tz: TimeZone>(
    remind: Remind,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Vec<MenuItem>
where
    Tz::Offset: std::fmt::Display,
{
    let row = |key: &str, icon, name: &str, help: String, on: bool| MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(icon),
        name: name.to_owned(),
        help: Some(help),
        right: Right::Check(on),
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    };
    let mut items = vec![row(
        OFF_KEY,
        Icon::X,
        "Don't remind me",
        "whether or not anyone answers".to_owned(),
        remind == Remind::Off,
    )];
    items.extend(crate::follow_up::CHOICES.iter().filter_map(|(key, says)| {
        let at = crate::follow_up::due(key, now, zone).ok()?;
        Some(row(
            key,
            Icon::Bell,
            says,
            snooze_help(at, zone),
            remind == Remind::After(key),
        ))
    }));
    let (help, on) = match remind {
        Remind::At(at) => (when_words(at, now, zone), true),
        _ => ("tomorrow 9, fri 17:00, +3d".to_owned(), false),
    };
    items.push(row(PICK_KEY, Icon::Clock, "Pick a time…", help, on));
    items
}

/// A choice from the Remind menu.
pub(in crate::ui) fn pick_remind(page: &mut Page, key: &str) {
    if key == PICK_KEY {
        page.float = Float::PickRemind(String::new());
        return;
    }
    if key == OFF_KEY {
        page.remind = Remind::Off;
    } else if let Some((choice, _)) = crate::follow_up::CHOICES
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
    let at = crate::follow_up::due(typed, now, zone)?;
    page.remind = Remind::At(at);
    page.float = Float::Closed;
    page.touch();
    Ok(at)
}

#[component]
pub(in crate::ui) fn RemindRow(page: Signal<Page>) -> Element {
    let remind = page.read().remind;
    let float = page.read().float.clone();
    let open = float == Float::Remind;
    let picking = matches!(float, Float::PickRemind(_));
    let now = super::super::clock::now();
    let items = remind_items(remind, now, &chrono::Local);
    let shown = remind.shown(now, &chrono::Local);
    let mut value = use_signal(|| None::<MountedRef>);
    rsx! {
        div { class: "prop-row", "data-row": "remind",
            div { class: "k", Glyph { icon: Icon::Bell, size: IconSize::Compact }, "Remind me" }
            div { class: "v",
                Button {
                    bezel: Bezel::Inline,
                    label: shown,
                    trailing: Some(Trailing::Glyph(Icon::ChevronDown)),
                    shown: Some(if open { Shown::Visible } else { Shown::Hidden }),
                    common: Common {
                        aria_label: Some("Remind me if no reply".to_owned()),
                        mounted: Some(EventHandler::new(move |event: MountedEvent| {
                            value.set(Some(MountedRef(event.data())));
                        })),
                        ..Common::default()
                    },
                    onclick: on_primary(move || {
                        let next = if open || picking { Float::Closed } else { Float::Remind };
                        page.write().float = next;
                    }),
                }
                if open {
                    Floating {
                        anchor: value(),
                        title: "Remind me if no reply".to_owned(),
                        items,
                        on_pick: move |key: String| {
                            pick_remind(&mut page.write(), &key);
                            if matches!(page.peek().float, Float::PickRemind(_)) {
                                crate::ui::host::Host::focus_after_task(".remind-field input");
                            }
                        },
                        on_close: move |_| {
                            if page.peek().float == Float::Remind {
                                page.write().float = Float::Closed;
                            }
                        },
                    }
                }
                if picking {
                    div { class: "p-menu", PickRemind { page } }
                }
            }
        }
    }
}

/// "Pick a time…" under the Remind row: the field, and under it the time it reads or why not.
#[component]
fn PickRemind(page: Signal<Page>) -> Element {
    let Float::PickRemind(typed) = page.read().float.clone() else {
        return rsx! {};
    };
    let now = super::super::clock::now();
    let reading = crate::follow_up::due(&typed, now, &chrono::Local);
    let (class, says) = match &reading {
        Ok(at) => (
            "pick-says",
            format!(
                "Back {} if nobody answers",
                when_in_sentence(*at, now, &chrono::Local)
            ),
        ),
        Err(why) if typed.trim().is_empty() => ("pick-says", why.clone()),
        Err(why) => ("pick-says refused", why.clone()),
    };
    rsx! {
        div { class: "pick-time",
            div { class: "g", "Remind me at" }
            div {
                class: "pick-in",
                onkeydown: move |event: KeyboardEvent| {
                    match menu_key(&event.key().to_string()) {
                        Some(MenuKey::Enter) => {
                            event.prevent_default();
                            let now = super::super::clock::now();
                            let _ = choose_remind(&mut page.write(), now, &chrono::Local);
                        }
                        Some(MenuKey::Escape) => {
                            event.stop_propagation();
                            page.write().float = Float::Remind;
                        }
                        _ => {}
                    }
                },
                Glyph { icon: Icon::Bell, size: IconSize::Nav }
                TextField {
                    label: "Remind me at".to_owned(),
                    value: typed.clone(),
                    placeholder: "tomorrow 9, fri 17:00, +3d".to_owned(),
                    bezel: FieldBezel::Plain,
                    common: Common {
                        extra_class: ExtraClass::parse("remind-field").ok(),
                        ..Common::default()
                    },
                    oninput: move |value: String| page.write().float = Float::PickRemind(value),
                }
            }
            p { class: "{class}", role: "status", "{says}" }
        }
    }
}

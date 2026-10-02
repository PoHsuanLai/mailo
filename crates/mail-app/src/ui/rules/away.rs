//! The vacation reply, in the Rules sheet: what `mailo vacation on|off` keeps, from fields.
//!
//! Kept here, run by the server: "Put on server" is what installs it, as `mailo sieve push`
//! does. A server without ManageSieve cannot run one, and this client does not send one itself
//! (`rules::server` says why), so on those accounts there is the reason and no form.

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::{FieldGroup, FieldRow, RowLayout};
use ds::components::fields::text_field_model::{FieldRows, Invalid};
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use mail_domain::{DateRange, Vacation};
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

use super::super::data::AccountRow;
use super::super::menus::when_words;
use super::super::press::on_primary;
use super::server::{configured, reach};

/// Whether the reply is wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Reply {
    Off,
    On,
}

/// The form, as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Away {
    pub reply: Reply,
    pub subject: String,
    pub body: String,
    /// When it starts and stops, as typed: a date, a date and a time, or a snooze word.
    pub from: String,
    pub until: String,
    /// The addresses it answers mail for, separated by commas or spaces.
    pub addresses: String,
    /// `:days`, kept from the reply already there.
    pub days: u16,
}

/// How a kept instant is written back into its field: what [`when`] reads.
fn typed<Tz: TimeZone>(at: Option<DateTime<Utc>>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    at.map(|t| t.with_timezone(zone).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

/// The form for `row`: its reply as kept, or an empty one that answers for its addresses.
pub(in crate::ui) fn load<Tz: TimeZone>(
    store: &SqliteStore,
    row: &AccountRow,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Away
where
    Tz::Offset: std::fmt::Display,
{
    match store.vacation(row.id).ok().flatten() {
        Some(kept) => Away {
            reply: Reply::On,
            subject: kept.subject,
            body: kept.body,
            from: typed(kept.during.from, zone),
            until: typed(kept.during.to, zone),
            addresses: kept.addresses.join(", "),
            days: kept.days,
        },
        None => {
            let fresh = mail_core::rules::server::vacation_for(
                &configured(store, row, now),
                "",
                "",
                Vacation::DEFAULT_DAYS,
                DateRange::default(),
            );
            Away {
                reply: Reply::Off,
                subject: String::new(),
                body: String::new(),
                from: String::new(),
                until: String::new(),
                addresses: fresh.addresses.join(", "),
                days: fresh.days,
            }
        }
    }
}

/// An instant as typed in `zone`: `2026-10-08`, `2026-10-08 09:00`, or what a snooze takes
/// (`tomorrow`, `monday`, `+3d`). Empty is no bound.
pub(in crate::ui) fn when<Tz: TimeZone>(
    text: &str,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<Option<DateTime<Utc>>, String>
where
    Tz::Offset: std::fmt::Display,
{
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    mail_core::rules::server::instant(text, zone)
        .or_else(|_| mail_core::snooze::snooze_until(text, now, zone))
        .map(Some)
        .map_err(|_| {
            format!("“{text}” is not a time: write 2026-10-08, 2026-10-08 09:00, or tomorrow")
        })
}

/// Keep what the form says on `row`, returning what the sheet says of it.
pub(in crate::ui) fn save<Tz: TimeZone>(
    store: &SqliteStore,
    row: &AccountRow,
    away: &Away,
    now: DateTime<Utc>,
    zone: &Tz,
) -> Result<String, String>
where
    Tz::Offset: std::fmt::Display,
{
    // Refused before anything is kept, as `vacation on` refuses: a reply nobody will send is
    // worse than none, because the user believes it is going out.
    reach(&row.plan)?;
    if away.reply == Reply::Off {
        store
            .put_vacation(row.id, None, now)
            .map_err(|e| e.to_string())?;
        return Ok("Vacation reply off.".to_owned());
    }
    if away.subject.trim().is_empty() {
        return Err("The reply needs a subject.".to_owned());
    }
    if away.body.trim().is_empty() {
        return Err("The reply needs something to say.".to_owned());
    }
    let during = DateRange {
        from: when(&away.from, now, zone)?,
        to: when(&away.until, now, zone)?,
    };
    if let Some(end) = during.to {
        if end <= now {
            return Err(format!(
                "“Until” is already past: {}. A reply that has ended answers nobody.",
                when_words(end, now, zone)
            ));
        }
        if during.from.is_some_and(|start| end <= start) {
            return Err("“Until” has to be after “From”.".to_owned());
        }
    }
    let addresses = addresses(&away.addresses)?;
    let reply = Vacation {
        addresses,
        ..mail_core::rules::server::vacation_for(
            &configured(store, row, now),
            away.subject.trim(),
            &away.body,
            away.days,
            during,
        )
    };
    store
        .put_vacation(row.id, Some(&reply), now)
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Saved: “{}”, {}.",
        reply.subject,
        span(&reply.during, now, zone)
    ))
}

/// The addresses typed, lowercased, each one an address.
fn addresses(typed: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for word in typed.split(|c: char| c == ',' || c.is_whitespace()) {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        if !word.contains('@') || word.starts_with('@') || word.ends_with('@') {
            return Err(format!("“{word}” is not an address."));
        }
        let address = word.to_lowercase();
        if !out.contains(&address) {
            out.push(address);
        }
    }
    if out.is_empty() {
        return Err("Add an address to reply for.".to_owned());
    }
    Ok(out)
}

/// When a reply runs, in words.
fn span<Tz: TimeZone>(during: &DateRange, now: DateTime<Utc>, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let at = |t: DateTime<Utc>| when_words(t, now, zone);
    match (during.from, during.to) {
        (None, None) => "from now until it is turned off".to_owned(),
        (Some(a), None) => format!("from {} until it is turned off", at(a)),
        (None, Some(b)) => format!("from now until {}", at(b)),
        (Some(a), Some(b)) => format!("from {} until {}", at(a), at(b)),
    }
}

/// The vacation reply's part of the sheet.
#[component]
pub(super) fn AwayPart(row: AccountRow) -> Element {
    let mut away = use_signal({
        let row = row.clone();
        move || {
            let store = consume_context::<Arc<SqliteStore>>();
            load(&store, &row, Utc::now(), &chrono::Local)
        }
    });
    let mut said = use_signal(|| None::<Result<String, String>>);
    if let Err(why) = reach(&row.plan) {
        return rsx! {
            section { class: "rules-part",
                SectionHeader { title: "Vacation reply".to_owned() }
                Label { text: format!("No vacation reply here: {why}."), role: LabelRole::Tertiary }
            }
        };
    }
    let now = Utc::now();
    let form = away();
    let on = form.reply == Reply::On;
    let hint = |text: &str| match when(text, now, &chrono::Local) {
        Ok(Some(at)) => (Validity::Valid, when_words(at, now, &chrono::Local)),
        Ok(None) => (Validity::Valid, String::new()),
        Err(why) => (
            Validity::Invalid(Invalid {
                message: why.clone().into(),
                stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
            }),
            String::new(),
        ),
    };
    let (from_validity, from_hint) = hint(&form.from);
    let (until_validity, until_hint) = hint(&form.until);
    let keep = row.clone();
    rsx! {
        section { class: "rules-part",
            SectionHeader { title: "Vacation reply".to_owned() }
            SegmentedControl::<Reply> {
                label: "Vacation reply".to_owned(),
                choices: vec![Choice::new(Reply::Off, "Off"), Choice::new(Reply::On, "On")],
                tracking: Tracking::SelectOne(form.reply),
                onchange: move |reply: Reply| away.write().reply = reply,
            }
            if on {
                FieldGroup {
                    FieldRow {
                        label: "Subject",
                        layout: RowLayout::Form,
                        TextField {
                            label: "Subject".to_owned(),
                            value: form.subject.clone(),
                            placeholder: "Away until the 12th".to_owned(),
                            oninput: move |value: String| away.write().subject = value,
                        }
                    }
                    FieldRow {
                        label: "Reply",
                        layout: RowLayout::Form,
                        TextField {
                            label: "The reply's text".to_owned(),
                            kind: FieldKind::Multiline,
                            rows: FieldRows::Four,
                            value: form.body.clone(),
                            placeholder: "Reply text".to_owned(),
                            oninput: move |value: String| away.write().body = value,
                        }
                    }
                    FieldRow {
                        label: "From",
                        help: Some(from_hint.into()),
                        layout: RowLayout::Form,
                        TextField {
                            label: "From".to_owned(),
                            value: form.from.clone(),
                            placeholder: "now, or 2026-10-08".to_owned(),
                            validity: from_validity,
                            oninput: move |value: String| away.write().from = value,
                        }
                    }
                    FieldRow {
                        label: "Until",
                        help: Some(until_hint.into()),
                        layout: RowLayout::Form,
                        TextField {
                            label: "Until".to_owned(),
                            value: form.until.clone(),
                            placeholder: "turned off, or monday".to_owned(),
                            validity: until_validity,
                            oninput: move |value: String| away.write().until = value,
                        }
                    }
                    FieldRow {
                        label: "Answers mail to",
                        layout: RowLayout::Form,
                        TextField {
                            label: "Answers mail to".to_owned(),
                            value: form.addresses.clone(),
                            placeholder: "you@example.com".to_owned(),
                            oninput: move |value: String| away.write().addresses = value,
                        }
                    }
                }
            }
            div { class: "rules-acts",
                match said() {
                    Some(Ok(text)) => rsx! {
                        div { role: "status", Label { text, role: LabelRole::Secondary } }
                    },
                    Some(Err(why)) => rsx! {
                        div { role: "alert", Label { text: why, role: LabelRole::Primary, style: LabelStyle::Headline } }
                    },
                    None => rsx! {},
                }
                Button {
                    label: "Keep reply".to_owned(),
                    onclick: on_primary(move || {
                        let store = consume_context::<Arc<SqliteStore>>();
                        let form = away.peek().clone();
                        said.set(Some(save(&store, &keep, &form, Utc::now(), &chrono::Local)));
                    }),
                }
            }
        }
    }
}

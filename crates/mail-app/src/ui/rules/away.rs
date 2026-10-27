//! The vacation reply, Settings' Vacation Reply page: what `mailo vacation on|off` keeps, from
//! fields.
//!
//! Kept here, run by the server: keeping it puts the account's script on its server, as `mailo
//! sieve push` does, or sets the reply a JMAP or Graph server keeps itself
//! (`rules::server::away_server`). A server that keeps none cannot send one, and this client does
//! not send one itself (`rules::server` says why), so on those accounts there is the reason and
//! no form.

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::components::fields::text_field_model::{FieldRows, Invalid};
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use mail_core::when::Clock;
use mail_core::{SqliteStore, Store};
use mail_domain::{DateRange, Vacation};
use std::sync::Arc;

use super::super::common::classed;
use super::super::data::AccountRow;
use super::super::menus::when_words;
use super::super::press::{available, on_primary};
use super::server::{configured, reach_away};
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::remedy::plainly;

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
///
/// Twenty-four hours whatever the person's clock: the field is typed into and read back
/// (`rules::server::instant`), and this is the form its refusal asks for, so what it holds must be
/// what it takes. Every time the page *says* follows the clock ([`span`]).
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
    match store.vacation(row.id.clone()).ok().flatten() {
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
        .map_err(|_| copy::errors::not_a_time(text))
}

/// Keep what the form says on `row`, returning what the page says of it.
pub(in crate::ui) fn save<Tz: TimeZone>(
    store: &SqliteStore,
    row: &AccountRow,
    away: &Away,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> Result<String, String>
where
    Tz::Offset: std::fmt::Display,
{
    // Refused before anything is kept, as `vacation on` refuses: a reply nobody will send is
    // worse than none, because the user believes it is going out.
    reach_away(&row.plan)?;
    if away.reply == Reply::Off {
        store
            .put_vacation(row.id.clone(), None, now)
            .map_err(|why| plainly(copy::errors::NOT_KEPT_HERE, why))?;
        return Ok(copy::toasts::VACATION_OFF.to_owned());
    }
    if away.subject.trim().is_empty() {
        return Err(copy::errors::REPLY_NEEDS_SUBJECT.to_owned());
    }
    if away.body.trim().is_empty() {
        return Err(copy::errors::REPLY_NEEDS_BODY.to_owned());
    }
    let during = DateRange {
        from: when(&away.from, now, zone)?,
        to: when(&away.until, now, zone)?,
    };
    if let Some(end) = during.to {
        if end <= now {
            return Err(copy::errors::until_passed(&when_words(
                end, now, zone, clock,
            )));
        }
        if during.from.is_some_and(|start| end <= start) {
            return Err(copy::errors::UNTIL_BEFORE_FROM.to_owned());
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
        .put_vacation(row.id.clone(), Some(&reply), now)
        .map_err(|why| plainly(copy::errors::NOT_KEPT_HERE, why))?;
    Ok(copy::toasts::vacation_saved(
        &reply.subject,
        &span(&reply.during, now, zone, clock),
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
            return Err(copy::errors::not_an_address(word));
        }
        let address = word.to_lowercase();
        if !out.contains(&address) {
            out.push(address);
        }
    }
    if out.is_empty() {
        return Err(copy::errors::ADD_AN_ADDRESS.to_owned());
    }
    Ok(out)
}

/// When a reply runs, in words.
fn span<Tz: TimeZone>(during: &DateRange, now: DateTime<Utc>, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let at = |t: DateTime<Utc>| when_words(t, now, zone, clock);
    copy::status::vacation::runs(during.from.map(at).as_deref(), during.to.map(at).as_deref())
}

/// The vacation reply's page: a switch, and while it is on, the reply's fields and Keep Reply.
/// Turned off, it is kept off at once; turned on, it is kept when Keep Reply is pressed, since a
/// reply needs its words first. Either is put on the server.
/// A date that cannot be read is its field's error and Keep Reply waits; a reply that could not
/// be kept is said in a dialog.
#[component]
pub(super) fn AwayPage(row: AccountRow, pushed: Callback<AccountRow>) -> Element {
    let mut refused = use_signal(|| None::<String>);
    let mut away = use_signal({
        let row = row.clone();
        move || {
            let store = consume_context::<Arc<SqliteStore>>();
            load(&store, &row, Utc::now(), &chrono::Local)
        }
    });
    // An account whose server takes no vacation reply has its switch, greyed: there is nothing
    // to turn on.
    if reach_away(&row.plan).is_err() {
        return rsx! {
            Form {
                FormSection {
                    FieldRow { label: copy::fields::VACATION_REPLY,
                        Toggle {
                            label: copy::fields::VACATION_REPLY.to_owned(),
                            value: Check::Off,
                            availability: Availability::Disabled,
                            onchange: |_| {},
                        }
                    }
                }
            }
        };
    }
    let now = Utc::now();
    let hours = crate::ui::clock::hours();
    let form = away();
    let on = form.reply == Reply::On;
    // A date that cannot be read is its field's error, and Keep Reply waits for both to read.
    let validity = |text: &str| match when(text, now, &chrono::Local) {
        Ok(_) => Validity::Valid,
        Err(why) => Validity::Invalid(Invalid {
            message: why.clone().into(),
            stamp: EventStamp(u32::try_from(why.len()).unwrap_or(0)),
        }),
    };
    let from_validity = validity(&form.from);
    let until_validity = validity(&form.until);
    let readable =
        matches!(from_validity, Validity::Valid) && matches!(until_validity, Validity::Valid);
    let keep = row.clone();
    let off = row.clone();
    rsx! {
        Form {
        FormSection {
            FieldRow {
                label: copy::fields::VACATION_REPLY,
                Toggle {
                    label: copy::fields::VACATION_REPLY.to_owned(),
                    value: if on { Check::On } else { Check::Off },
                    onchange: move |_| {
                        if on {
                            away.write().reply = Reply::Off;
                            let store = consume_context::<Arc<SqliteStore>>();
                            let form = away.peek().clone();
                            match save(&store, &off, &form, Utc::now(), &chrono::Local, hours) {
                                Ok(_) => {
                                    refused.set(None);
                                    pushed.call(off.clone());
                                }
                                Err(why) => refused.set(Some(why)),
                            }
                        } else {
                            away.write().reply = Reply::On;
                            refused.set(None);
                        }
                    },
                }
            }
        }
        if on {
            FormSection {
                FieldRow { label: copy::fields::SUBJECT,
                    TextField {
                        label: copy::fields::SUBJECT.to_owned(),
                        value: form.subject.clone(),
                        placeholder: copy::fields::AWAY_SUBJECT_EXAMPLE.to_owned(),
                        common: classed("rules-field"),
                        oninput: move |value: String| away.write().subject = value,
                    }
                }
                FieldRow { label: copy::fields::REPLY,
                    TextField {
                        label: copy::fields::REPLY_BODY.to_owned(),
                        kind: FieldKind::Multiline,
                        rows: FieldRows::Four,
                        value: form.body.clone(),
                        placeholder: copy::fields::REPLY_TEXT.to_owned(),
                        common: classed("rules-field"),
                        oninput: move |value: String| away.write().body = value,
                    }
                }
                FieldRow { label: copy::fields::ANSWERS_MAIL_TO,
                    TextField {
                        label: copy::fields::ANSWERS_MAIL_TO.to_owned(),
                        value: form.addresses.clone(),
                        placeholder: "you@example.com".to_owned(),
                        common: classed("rules-field"),
                        oninput: move |value: String| away.write().addresses = value,
                    }
                }
            }
            FormSection {
                FieldRow { label: copy::fields::FROM,
                    TextField {
                        label: copy::fields::FROM.to_owned(),
                        value: form.from.clone(),
                        placeholder: copy::fields::FROM_EXAMPLE.to_owned(),
                        validity: from_validity,
                        common: classed("rules-field"),
                        oninput: move |value: String| away.write().from = value,
                    }
                }
                FieldRow { label: copy::fields::UNTIL,
                    TextField {
                        label: copy::fields::UNTIL.to_owned(),
                        value: form.until.clone(),
                        placeholder: copy::fields::UNTIL_EXAMPLE.to_owned(),
                        validity: until_validity,
                        common: classed("rules-field"),
                        oninput: move |value: String| away.write().until = value,
                    }
                }
            }
            // Keep Reply stands alone under the groups, at their end, as a Space's Create Space
            // does.
            div { class: "acct-actions",
                    Button {
                        label: copy::controls::KEEP_REPLY.to_owned(),
                        availability: available(readable),
                        onclick: on_primary(move || {
                            let store = consume_context::<Arc<SqliteStore>>();
                            let form = away.peek().clone();
                            match save(&store, &keep, &form, Utc::now(), &chrono::Local, hours) {
                                Ok(_) => {
                                    refused.set(None);
                                    pushed.call(keep.clone());
                                }
                                Err(why) => refused.set(Some(why)),
                            }
                        }),
                    }
                }
            }
        }
        if let Some(why) = refused() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| refused.set(None) }
        }
    }
}

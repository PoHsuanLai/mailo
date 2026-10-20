//! Send later, in the window: what Send ▾ › Send Later and the Sends row choose means to the
//! outbox, the "Pick a Time…" field, and the list of messages waiting for their time.
//!
//! The times are the snooze menu's: a typed phrase goes through [`mail_core::snooze::snooze_until`],
//! and every time is written by [`when_words`], so "Tomorrow 08:00" reads the same everywhere.

use ds::base::geometry::placement::{Align, Side};
use ds::components::fields::text_field_model::Invalid;
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;

use chrono::{DateTime, TimeZone, Utc};
use dioxus::prelude::*;
use mail_core::undo::{Resend, Undo};
use mail_core::when::Clock;
use mail_core::{SqliteStore, Store};
use mail_domain::{ChangeId, Draft, DraftId, Patch, SendState};
use std::sync::Arc;

use super::super::menu::{MenuKey, menu_key};
use super::super::menus::{when_in_sentence, when_words};
use super::super::motion::{Follow, record, tell};
use super::desk::{Desk, refusal};
use super::page::{Float, Page, When};
use crate::ui::copy;
use crate::ui::space::Scope;
use crate::ui::view::Shell;
use mail_core::compose::Leaves;

/// The Sends menu's key for "Pick a Time…".
pub(in crate::ui) const PICK_KEY: &str = "at";
/// And what it says.
pub(in crate::ui) const PICK_LABEL: &str = "Pick a Time…";

/// What the example phrases are, where the field has nothing to go on.
const TRY: &str = "Type a time, like tomorrow 9";

/// What Send does with the Sends row's choice. A time that has already gone is refused, in
/// words, rather than sent at once: the person asked for later, and "now" is not later.
pub(in crate::ui) fn leaves<Tz: TimeZone>(
    when: When,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> Result<Leaves, String>
where
    Tz::Offset: std::fmt::Display,
{
    match when.due(now, zone) {
        None => Ok(Leaves::Now),
        Some(at) if at <= now => Err(passed(at, now, zone, clock)),
        Some(at) => Ok(Leaves::At(at)),
    }
}

/// The time a phrase typed into "Pick a time…" names, if it is one still to come.
pub(in crate::ui) fn pick_time<Tz: TimeZone>(
    typed: &str,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    if typed.trim().is_empty() {
        return Err(TRY.to_owned());
    }
    let at = mail_core::snooze::snooze_until(typed, now, zone)?;
    if at <= now {
        return Err(passed(at, now, zone, clock));
    }
    Ok(at)
}

fn passed<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, zone: &Tz, clock: Clock) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!(
        "{} has passed. Pick a later time, or send it now.",
        when_words(at, now, zone, clock)
    )
}

/// Enter in the field: the typed time becomes the Sends row's choice. Refused, the field stays
/// open with its reason under it.
pub(in crate::ui) fn choose_time<Tz: TimeZone>(
    page: &mut Page,
    now: DateTime<Utc>,
    zone: &Tz,
    clock: Clock,
) -> Result<DateTime<Utc>, String>
where
    Tz::Offset: std::fmt::Display,
{
    let Float::PickTime(typed) = &page.float else {
        return Err(TRY.to_owned());
    };
    let at = pick_time(typed, now, zone, clock)?;
    page.when = When::At(at);
    page.float = Float::Closed;
    page.touch();
    Ok(at)
}

/// "Pick a Time…": a popover hung from `anchor` (Send ▾, or the Sends row's value) with the field,
/// and under it the time it reads or why it reads none. Esc or a click away closes it and leaves
/// the choice as it was.
#[component]
pub(in crate::ui) fn PickTime(page: Signal<Page>, anchor: Option<MountedRef>) -> Element {
    let Float::PickTime(typed) = page.read().float.clone() else {
        return rsx! {};
    };
    let now = Utc::now();
    let hours = crate::ui::clock::hours();
    let reading = pick_time(&typed, now, &chrono::Local, hours);
    // What it reads is help under the field; a phrase it cannot read is the field's own refusal.
    let (validity, says) = match &reading {
        Ok(at) => (
            Validity::Valid,
            Some(format!(
                "Leaves {}",
                when_in_sentence(*at, now, &chrono::Local, hours)
            )),
        ),
        Err(why) if typed.trim().is_empty() => (Validity::Valid, Some(why.clone())),
        Err(why) => (
            Validity::Invalid(Invalid {
                message: why.clone().into(),
                stamp: EventStamp(u32::try_from(typed.len()).unwrap_or(0)),
            }),
            None,
        ),
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
                            let _ = choose_time(&mut page.write(), Utc::now(), &chrono::Local, hours);
                        }
                        Some(MenuKey::Escape) => {
                            event.stop_propagation();
                            page.write().float = Float::Closed;
                        }
                        _ => {}
                    }
                },
                TextField {
                    label: "Send at".to_owned(),
                    placeholder: "tomorrow 9".to_owned(),
                    value: typed.clone(),
                    validity,
                    help: says.map(TextLine::from),
                    focus: FieldFocus::OnMount,
                    oninput: move |value: String| page.write().float = Float::PickTime(value),
                    common: Common { extra_class: ExtraClass::parse("pick-field").ok(), ..Common::default() },
                }
            }
        }
    }
}

/// Every draft held for later on the accounts `scope` shows, soonest first: what the Scheduled
/// place lists, and whether the sidebar draws it.
pub(in crate::ui) fn scheduled(store: &SqliteStore, scope: &Scope) -> Vec<Draft> {
    let mut out: Vec<(DateTime<Utc>, Draft)> = mail_core::compose::sending_accounts(store)
        .into_iter()
        .filter(|(_, account)| scope.shows(account.clone()))
        .flat_map(|(_, account)| store.drafts(account).unwrap_or_default())
        .filter_map(|draft| match draft.state {
            SendState::Scheduled { at } => Some((at, draft)),
            _ => None,
        })
        .collect();
    out.sort_by_key(|(at, _)| *at);
    out.into_iter().map(|(_, draft)| draft).collect()
}

/// Cancel Sending, from a scheduled message's menu: it goes back to being a draft, in Drafts,
/// and the toast offers it back. Refused (it has started to leave), the reason is said in the
/// toast and nothing changes.
pub(in crate::ui) fn cancel_scheduled(shell: Signal<Shell>, draft: DraftId) -> Result<(), String> {
    let store = consume_context::<Arc<SqliteStore>>();
    let held = store.draft(draft).map_err(crate::ui::remedy::told)?;
    let SendState::Scheduled { at } = held.state else {
        return Ok(());
    };
    if let Err(why) = super::life::unsend(&store, draft, Utc::now()) {
        let said = refusal(&why);
        tell(said.clone(), Follow::Nothing);
        return Err(said);
    }
    // The pill was saying this send was scheduled; it is not any more.
    if let Some(mut desk) = try_consume_context::<Desk>()
        && desk
            .outbox
            .peek()
            .as_ref()
            .is_some_and(|out| out.draft == draft)
    {
        desk.outbox.set(None);
    }
    let nothing = || Patch {
        id: ChangeId::generate(),
        changes: Vec::new(),
    };
    record(
        shell,
        vec![Undo {
            said: copy::toasts::SENDING_CANCELLED.to_owned(),
            thread: None,
            account: held.account,
            forward: nothing(),
            inverse: nothing(),
            remote: None,
            resend: Some(Resend { draft, at }),
        }],
        copy::toasts::SENDING_CANCELLED.to_owned(),
    );
    Ok(())
}

/// Undo of Cancel Sending: the draft is queued again for its time, sealed again as it asks.
/// `false`, and nothing changes, when it cannot be: its time has passed, or its key is locked.
pub(in crate::ui) fn send_again(store: &SqliteStore, resend: Resend) -> bool {
    let secrets = crate::ui::pgp::seams().secrets;
    super::life::queue_reminding(
        store,
        secrets.as_ref(),
        &mail_core::pgp::no_passphrase,
        resend.draft,
        Leaves::At(resend.at),
        None,
        Utc::now(),
    )
    .is_ok()
}

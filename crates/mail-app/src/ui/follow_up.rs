//! Remind me if no reply, in the window: the sweep that brings a conversation back. A reminder is
//! set under the composer's Send ▾ alone. And the timer that shows a snoozed one come back.
//!
//! The deciding is [`mail_core::follow_up`]'s; this is when it runs and how it is shown.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_core::SqliteStore;
use mail_core::notify::Notifier;
use std::sync::Arc;
use std::time::Duration;

/// Where the window says a reminder came back: the desktop's notifications in the launched
/// window, while notifications are on; a recorder in a test. A window with none says it only in
/// its list.
#[derive(Clone)]
pub struct Notices(pub Arc<dyn Notifier>);

impl std::fmt::Debug for Notices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Notices(..)")
    }
}

/// The longest the window sleeps between sweeps. A sleep is measured on a clock that stops while
/// the machine is suspended, so a reminder due during a suspend is caught up at most this long
/// after the machine wakes.
const LONGEST: Duration = Duration::from_secs(300);

/// How long to sleep from `now` until the sweep has something to do at `next`: never less than a
/// second, never more than [`LONGEST`].
///
/// A `next` already gone means the sweep just before this failed (a successful one brings every
/// due reminder back), so it is tried again after the longest wait rather than in a tight loop.
pub(in crate::ui) fn wait(next: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Duration {
    match next.map(|next| (next - now).to_std()) {
        Some(Ok(until)) => until.clamp(Duration::from_secs(1), LONGEST),
        Some(Err(_)) | None => LONGEST,
    }
}

/// Sweep the reminders now and again whenever the next one comes due, and on every revision, so
/// a reply that just arrived or a reminder just set is seen at once. A sweep that changed
/// anything moves the revision, which redraws the list.
///
/// The first sweep is the launch's: a reminder that came due while mailo was closed comes back
/// then. The sleep is quire's, so it runs on the window's clock (a harness's virtual one in a
/// test).
pub(in crate::ui) fn use_reminders(mut revision: Signal<u64>) {
    let _sweeping = use_resource(move || {
        let _ = revision();
        async move {
            loop {
                let now = super::clock::now();
                let store = consume_context::<Arc<SqliteStore>>();
                let notices = try_consume_context::<Notices>();
                let notifier = notices.as_ref().map(|notices| notices.0.as_ref());
                match mail_core::follow_up::sweep_and_announce(&store, notifier, now) {
                    // Moving the revision starts this again, with the next due time.
                    Ok(swept) if swept.changed() => {
                        revision += 1;
                        return;
                    }
                    Ok(_) => {}
                    Err(why) => log::warn!("reminders: {why}"),
                }
                ds::base::time::clock::sleep(wait(mail_core::follow_up::next_due(&store), now))
                    .await;
            }
        }
    });
}

/// Ask the list again when the next snoozed conversation comes back, so it shows in the inbox on
/// its time rather than at the next change. A revision starts this again, with a snooze just set
/// or woken counted; the sleep is quire's, on the window's clock, as [`use_reminders`]'s is.
pub(in crate::ui) fn use_snooze_wake(mut revision: Signal<u64>) {
    let client = super::live::use_client();
    let _waking = use_resource(move || {
        let client = client.clone();
        let _ = revision();
        async move {
            let store = consume_context::<Arc<SqliteStore>>();
            let Some(at) = mail_core::snooze::next_wake(&store, super::clock::now()) else {
                return;
            };
            loop {
                let now = super::clock::now();
                if at <= now {
                    // What the client keeps read by the clock is read again, and what the
                    // window still reads itself is too.
                    super::live::clock_moved(&client);
                    revision += 1;
                    return;
                }
                ds::base::time::clock::sleep(wait(Some(at), now)).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 28, hour, 0, 0).unwrap()
    }

    #[test]
    fn the_sweep_sleeps_until_the_next_reminder_within_bounds() {
        let now = at(12);
        let cases = [
            ("nothing waiting", None, LONGEST),
            ("an hour away", Some(at(13)), LONGEST),
            (
                "a minute away",
                Some(now + chrono::TimeDelta::minutes(1)),
                Duration::from_secs(60),
            ),
            (
                "half a second away",
                Some(now + chrono::TimeDelta::milliseconds(500)),
                Duration::from_secs(1),
            ),
            (
                "already due, after a sweep that failed",
                Some(at(11)),
                LONGEST,
            ),
        ];
        for (name, next, want) in cases {
            assert_eq!(wait(next, now), want, "{name}");
        }
    }
}

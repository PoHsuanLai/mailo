//! Remind me if no reply, in the window: the sweep that brings a conversation back. A reminder is
//! set under the composer's Send ▾ alone. And the timer that shows a snoozed one come back.
//!
//! The deciding is [`mail_client::follow_up`]'s; this is when it runs and how it is shown.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_client::notify::Notifier;
use std::sync::Arc;
use std::time::Duration;

/// Where the window says new mail arrived and a reminder came back: the desktop's notifications
/// in the launched window, a recorder in a test. A window with none says it only in its list.
///
/// Provided whether or not notifications are on; [`speaking`] asks the setting each time, so
/// turning them off or on in Settings takes effect at once.
#[derive(Clone)]
pub struct Notices(pub Arc<dyn Notifier>);

/// The window's notices while `notifications.new_mail` is on, read now. A window with no settings
/// (a test's lone component) has them on, as the setting's default is.
pub(in crate::ui) fn speaking(
    notices: Option<&Notices>,
    settings: Option<Signal<crate::settings::MailSettings>>,
) -> Option<Arc<dyn Notifier>> {
    let on = settings.is_none_or(|settings| {
        settings.peek().notifications.new_mail == crate::settings::NewMail::On
    });
    notices.filter(|_| on).map(|notices| notices.0.clone())
}

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

/// The least time between one sweep's start and the next's, whatever moves the store in between.
///
/// A sync commits summaries again and again, and each sweep reads every reminded conversation
/// and its messages. A reply that ends a reminder is seen this much later at worst, which is
/// no later than a person would look.
const SWEEP_EVERY: Duration = Duration::from_secs(5);

/// Sweep the reminders now and again whenever the next one comes due, and whenever the store
/// moves, so a reply that just arrived or a reminder just set is seen soon. A sweep that
/// changed anything moves the store, which redraws the list.
///
/// A reply arriving is a change to the summaries, and it is what ends a reminder, so the sweep
/// follows them as well as the reminders and the clock; [`SWEEP_EVERY`] keeps a sync's stream of
/// commits from sweeping with each. The sweep runs off the thread that draws, as it reads the
/// store, and the window waits for its answer.
///
/// The first sweep is the launch's: a reminder that came due while mailo was closed comes back
/// then. The sleep is quire's, so it runs on the window's clock (a harness's virtual one in a
/// test).
pub(in crate::ui) fn use_reminders() {
    // When the last sweep began, kept across the runs a commit starts, so a run that is started
    // again before its turn waits only for what is left of the time, not for all of it.
    let mut began = use_hook(|| CopyValue::new(None::<DateTime<Utc>>));
    let _sweeping = use_resource(move || {
        let _ = crate::ui::live::moved_for(
            mail_client::Topics::SUMMARIES
                | mail_client::Topics::FOLLOW_UPS
                | mail_client::Topics::CLOCK,
        );
        async move {
            loop {
                let last = *began.peek();
                if let Some(turn) = last.map(|last| last + SWEEP_EVERY)
                    && let Ok(left) = (turn - super::clock::now()).to_std()
                {
                    ds::base::time::clock::sleep(left).await;
                }
                let now = super::clock::now();
                began.set(Some(now));
                let client = crate::ui::live::client();
                let notices = speaking(
                    try_consume_context::<Notices>().as_ref(),
                    try_consume_context::<Signal<crate::settings::MailSettings>>(),
                );
                let swept = tokio::task::spawn_blocking(move || {
                    let swept = client
                        .follow_up()
                        .sweep_and_announce(notices.as_deref(), now);
                    (swept, client.follow_up().next_due())
                })
                .await;
                let next = match swept {
                    // The store moving starts this again, with the next due time.
                    Ok((Ok(swept), _)) if swept.changed() => {
                        return;
                    }
                    Ok((Ok(_), next)) => next,
                    Ok((Err(why), next)) => {
                        log::warn!("reminders: {why}");
                        next
                    }
                    Err(why) => {
                        log::warn!("reminders: the sweep did not finish: {why}");
                        None
                    }
                };
                ds::base::time::clock::sleep(wait(next, now)).await;
            }
        }
    });
}

/// Tell the client when the next snoozed conversation comes back, where it cannot tell by
/// itself: the program's client keeps the system's time on a thread of its own, but a window
/// whose clock is its own (a test's, on quire's virtual time) sleeps on that clock, and a client
/// that reads on the thread that draws has no thread to keep it. The sleep is quire's, as
/// [`use_reminders`]'s is; what the client marks stale moves the store, which starts this
/// again with the next one.
pub(in crate::ui) fn use_snooze_wake() {
    let client = super::live::use_client();
    let bridging =
        !client.keeps_time() || try_consume_context::<super::clock::WallClock>().is_some();
    let _waking = use_resource(move || {
        let client = client.clone();
        let _ =
            crate::ui::live::moved_for(mail_client::Topics::SUMMARIES | mail_client::Topics::CLOCK);
        async move {
            if !bridging {
                return;
            }
            let Some(at) = client.next_clock() else {
                return;
            };
            loop {
                let now = super::clock::now();
                if at <= now {
                    super::live::clock_moved(&client);
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

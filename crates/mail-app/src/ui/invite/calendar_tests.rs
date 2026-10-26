//! The invitation card and the calendar, against a calendar that lives in memory: adding the
//! event, answering it, taking a cancelled one out, and proposing another time. No test reaches a
//! real calendar; the window gets one only by being handed a [`Calendars`].

use super::tests::{
    ME, Open, OpenProps, Saves, far, ics, markup, outbox, put, request, submissions,
};
use crate::calendar::Calendars;
use crate::calendar::fake::Fake;
use crate::calendar::flow::{self, Added, Kept};
use crate::ui::copy::controls;
use crate::ui::fixtures::{Seen, click, dispatching, rebuild_into, seeded, settle_for};
use dioxus::dioxus_core::VirtualDom;
use mail_core::message::Looks;
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use porter_calendar::When;
use std::sync::Arc;
use std::time::Duration;

fn cancelled() -> String {
    ics(
        "CANCEL",
        1,
        "CANCELLED",
        "ada@example.test",
        &[("Me", ME, "NEEDS-ACTION")],
        "",
    )
}

fn desk() -> (Arc<Fake>, Calendars) {
    let fake = Arc::new(Fake::new());
    (fake.clone(), Calendars(fake))
}

#[tokio::test]
async fn add_puts_the_event_in_the_chosen_calendar_as_the_organiser_sent_it() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, &request(0), "REQUEST");
    let (fake, calendars) = desk();

    let added = flow::add(&calendars, &store, message, "home")
        .await
        .unwrap();

    assert_eq!(added, Added::In("Home".to_owned()));
    assert_eq!(
        fake.held(),
        [("home".to_owned(), "review-7@example.test".to_owned())]
    );
    let event = fake.event("review-7@example.test").unwrap();
    assert_eq!(event.summary, "Design review");
    assert_eq!(event.location.as_deref(), Some("Room 2"));
    // 14:00 in Berlin on 5 October 2026 is 12:00 UTC: 1 790 000 000 s is 2026-09-21.
    assert!(matches!(event.when, When::Timed { start, end } if end.0 - start.0 == 3600));
    // The object a calendar keeps is the event, not the message that carried it.
    assert!(!event.ics.contains("METHOD:"), "{}", event.ics);
    assert!(
        event.ics.contains("ORGANIZER;CN=Ada Lovelace"),
        "{}",
        event.ics
    );
    // Adding does not answer: the reader's own line is as it was sent.
    assert!(event.ics.contains("PARTSTAT=NEEDS-ACTION"), "{}", event.ics);
}

#[tokio::test]
async fn add_falls_back_to_the_first_calendar_that_takes_events_and_says_when_there_is_none() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, &request(0), "REQUEST");
    let (fake, calendars) = desk();
    // An id the store no longer has, and an empty one: the first writable calendar.
    for wanted in ["gone", "", "holidays"] {
        flow::add(&calendars, &store, message, wanted)
            .await
            .unwrap();
    }
    assert_eq!(
        fake.held(),
        [("work".to_owned(), "review-7@example.test".to_owned())],
        "the same event, put again, is one event"
    );

    let mut none = Fake::new();
    none.calendars.iter_mut().for_each(|c| c.writable = false);
    let calendars = Calendars(Arc::new(none));
    assert!(flow::add(&calendars, &store, message, "").await.is_err());
}

#[tokio::test]
async fn an_answer_updates_an_event_that_is_in_the_calendar_and_never_adds_one() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, &request(0), "REQUEST");
    let (fake, calendars) = desk();

    let kept = flow::follow(&calendars, &store, message, Attendance::Accepted)
        .await
        .unwrap();
    assert_eq!(kept, Kept::NotThere);
    assert!(fake.held().is_empty(), "an answer added an event");

    flow::add(&calendars, &store, message, "work")
        .await
        .unwrap();
    let kept = flow::follow(&calendars, &store, message, Attendance::Accepted)
        .await
        .unwrap();
    assert_eq!(kept, Kept::Updated("Work".to_owned()));
    let ics = fake.event("review-7@example.test").unwrap().ics;
    assert!(ics.contains("PARTSTAT=ACCEPTED"), "{ics}");
    assert!(!ics.contains("PARTSTAT=NEEDS-ACTION"), "{ics}");

    // Declining takes it out, and declining again finds nothing to do.
    let kept = flow::follow(&calendars, &store, message, Attendance::Declined)
        .await
        .unwrap();
    assert_eq!(kept, Kept::Removed("Work".to_owned()));
    assert!(fake.held().is_empty());
    let kept = flow::follow(&calendars, &store, message, Attendance::Declined)
        .await
        .unwrap();
    assert_eq!(kept, Kept::NotThere);
}

#[tokio::test]
async fn a_cancellation_takes_the_event_out_of_the_calendar_that_has_it() {
    let (store, _dir) = seeded();
    let (_, invitation) = put(&store, &request(0), "REQUEST");
    let (_, cancellation) = put(&store, &cancelled(), "CANCEL");
    let (fake, calendars) = desk();

    assert_eq!(
        flow::cancel(&calendars, &store, cancellation).await,
        Ok(None)
    );
    flow::add(&calendars, &store, invitation, "home")
        .await
        .unwrap();
    assert_eq!(
        flow::cancel(&calendars, &store, cancellation).await,
        Ok(Some("Home".to_owned()))
    );
    assert!(fake.held().is_empty());
    // Only a cancellation is taken out this way.
    assert!(flow::cancel(&calendars, &store, invitation).await.is_err());
}

#[test]
fn a_proposed_time_queues_a_counter_and_records_no_answer() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, &request(0), "REQUEST");
    let before = submissions(&store).len();
    let start = flow::local_time("2026-10-06 09:30").unwrap();

    let sent = mail_core::invite::propose(
        &store,
        message,
        start,
        Some("Tuesday suits me better"),
        chrono::Utc::now(),
    )
    .unwrap();

    assert_eq!(sent.to, ["ada@example.test"]);
    let queued = submissions(&store);
    assert_eq!(queued.len(), before + 1);
    let counter = queued.last().unwrap();
    assert!(counter.contains("method=\"COUNTER\""), "{counter}");
    assert!(counter.contains("METHOD:COUNTER"), "{counter}");
    assert!(counter.contains("UID:review-7@example.test"), "{counter}");
    assert!(
        counter.contains(&start.format("DTSTART:%Y%m%dT%H%M%SZ").to_string()),
        "{counter}"
    );
    // The event keeps its length: an hour.
    assert!(
        counter.contains(
            &(start + chrono::TimeDelta::hours(1))
                .format("DTEND:%Y%m%dT%H%M%SZ")
                .to_string()
        ),
        "{counter}"
    );
    assert!(counter.contains("Tuesday suits me better"), "{counter}");
    assert_eq!(store.invite_answer(message).unwrap(), None);
}

#[test]
fn only_an_invitation_to_the_reader_can_be_countered() {
    let (store, _dir) = seeded();
    let (_, gone) = put(&store, &cancelled(), "CANCEL");
    let (_, own) = put(
        &store,
        &ics(
            "REQUEST",
            0,
            "CONFIRMED",
            ME,
            &[("Ada Lovelace", "ada@example.test", "NEEDS-ACTION")],
            "",
        ),
        "REQUEST",
    );
    let queued = outbox(&store);
    for (case, message) in [("cancelled", gone), ("organised", own)] {
        let refused = mail_core::invite::propose(
            &store,
            message,
            chrono::Utc::now() + chrono::TimeDelta::days(1),
            None,
            chrono::Utc::now(),
        );
        assert!(refused.is_err(), "{case}");
    }
    assert_eq!(outbox(&store), queued);
}

#[test]
fn a_typed_time_is_read_in_the_readers_zone_or_refused() {
    assert!(flow::local_time(" 2026-11-02 14:30 ").is_some());
    for bad in [
        "",
        "tomorrow",
        "2026-11-02",
        "2026-13-02 14:30",
        "2026-11-02 25:00",
    ] {
        assert!(flow::local_time(bad).is_none(), "{bad:?}");
    }
}

/// The reader on `thread` with `calendars` in its root context, once its lookups have landed.
async fn reader_with(
    store: Arc<SqliteStore>,
    thread: ThreadId,
    saves: &std::path::Path,
    calendars: Option<Calendars>,
) -> (VirtualDom, Seen) {
    dispatching();
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread })
        .with_root_context(store)
        .with_root_context(Arc::new(Looks::new()))
        .with_root_context(Saves(saves.to_owned()));
    if let Some(calendars) = calendars {
        dom = dom.with_root_context(calendars);
    }
    let mut seen = rebuild_into(&mut dom);
    settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
    (dom, seen)
}

#[tokio::test]
async fn a_window_with_no_calendar_offers_none_and_one_with_a_calendar_adds_on_a_press() {
    let (store, dir) = seeded();
    let (thread, _) = put(&store, &request(0), "REQUEST");

    // Without a calendar in the window nothing about calendars is drawn.
    let (dom, _) = reader_with(store.clone(), thread, dir.path(), None).await;
    let page = markup(&dom);
    assert!(!page.contains(controls::ADD_TO_CALENDAR), "{page}");
    assert!(!page.contains(controls::PROPOSE_NEW_TIME), "{page}");

    // With one, opening the message reaches nothing, and the press adds the event.
    let (fake, calendars) = desk();
    let (mut dom, seen) = reader_with(store.clone(), thread, dir.path(), Some(calendars)).await;
    assert!(fake.held().is_empty(), "opening reached the calendar");
    assert!(markup(&dom).contains(controls::PROPOSE_NEW_TIME));
    let mut seen = click(&mut dom, seen.one("aria-label", controls::ADD_TO_CALENDAR));
    settle_for(&mut dom, &mut seen, Duration::from_millis(400)).await;
    assert_eq!(
        fake.held(),
        [("work".to_owned(), "review-7@example.test".to_owned())]
    );
    // Nothing was answered or sent by it.
    assert_eq!(outbox(&store), 0);
    let _ = far;
}

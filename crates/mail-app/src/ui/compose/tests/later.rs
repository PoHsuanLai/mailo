//! Send later in the window: the Sends choice as the outbox reads it, a scheduled send on the
//! pill and in Today, and Cancel, against a real store.

use mail_store::Store;

use super::super::later::{choose_time, leaves, pick_time, waiting};
use super::super::page::{Float, Phase, When};
use super::super::props::pick_sends;
use super::*;
use crate::compose::Leaves;
use crate::editor::{to_flowed, to_html};
use crate::ui::fixtures::{ACCOUNT, click, seeded};

/// 2026-09-23 10:00 UTC, a Wednesday, read in UTC.
fn wednesday(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 23, hour, minute, 0)
        .single()
        .unwrap_or_else(|| panic!("a real instant"))
}

fn day(date: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, date, hour, minute, 0)
        .single()
        .unwrap_or_else(|| panic!("a real instant"))
}

#[test]
fn the_sends_choice_is_when_the_outbox_lets_it_leave() {
    let now = wednesday(10, 0);
    let cases: &[(&str, When, Result<Leaves, &str>)] = &[
        ("right away", When::Now, Ok(Leaves::Now)),
        ("tomorrow", When::Tomorrow, Ok(Leaves::At(day(24, 8, 0)))),
        ("monday", When::Monday, Ok(Leaves::At(day(28, 9, 0)))),
        (
            "a typed time",
            When::At(day(25, 17, 0)),
            Ok(Leaves::At(day(25, 17, 0))),
        ),
        (
            "a typed time now gone",
            When::At(day(23, 9, 0)),
            Err("Today 09:00 has already passed"),
        ),
        (
            "this very minute",
            When::At(now),
            Err("Today 10:00 has already passed"),
        ),
    ];
    for (name, when, want) in cases {
        let got = leaves(*when, now, &Utc);
        match (want, &got) {
            (Ok(want), Ok(got)) => assert_eq!(got, want, "{name}"),
            (Err(words), Err(said)) => assert!(said.starts_with(words), "{name}: {said}"),
            _ => panic!("{name}: wanted {want:?}, got {got:?}"),
        }
    }
}

#[test]
fn a_typed_phrase_is_read_the_way_snooze_reads_it_and_a_past_one_is_refused() {
    let now = wednesday(10, 0);
    let cases: &[(&str, Result<DateTime<Utc>, &str>)] = &[
        ("tomorrow 9", Ok(day(24, 9, 0))),
        ("fri 17:00", Ok(day(25, 17, 0))),
        ("today 17:30", Ok(day(23, 17, 30))),
        ("+2h", Ok(wednesday(12, 0))),
        ("tomorrow", Ok(day(24, 9, 0))),
        (
            "2026-09-01 10:00",
            Err("Tue 1 Sep 10:00 has already passed"),
        ),
        ("today 8", Err("Today 08:00 has already passed")),
        ("", Err("Type a time")),
        ("soonish", Err("\"soonish\" is not a time I know")),
        (
            "tomorrow 25:00",
            Err("\"tomorrow 25:00\" is not a time I know"),
        ),
    ];
    for (typed, want) in cases {
        let got = pick_time(typed, now, &Utc);
        match (want, &got) {
            (Ok(want), Ok(got)) => assert_eq!(got, want, "{typed:?}"),
            (Err(words), Err(said)) => assert!(said.starts_with(words), "{typed:?}: {said}"),
            _ => panic!("{typed:?}: wanted {want:?}, got {got:?}"),
        }
    }
}

#[test]
fn pick_a_time_opens_a_field_and_enter_makes_the_typed_time_the_choice() {
    let now = wednesday(10, 0);
    let mut page = page_of("hello");
    page.float = Float::Sends;
    pick_sends(&mut page, super::super::later::PICK_KEY);
    assert_eq!(
        page.float,
        Float::PickTime(String::new()),
        "the field did not open"
    );
    assert_eq!(page.when, When::Now, "picking the row chose a time");

    // A time that has gone leaves the field open and the choice alone.
    page.float = Float::PickTime("today 8".to_owned());
    assert!(choose_time(&mut page, now, &Utc).is_err());
    assert_eq!(page.when, When::Now);
    assert_eq!(page.float, Float::PickTime("today 8".to_owned()));

    page.float = Float::PickTime("fri 17:00".to_owned());
    assert_eq!(choose_time(&mut page, now, &Utc), Ok(day(25, 17, 0)));
    assert_eq!(page.when, When::At(day(25, 17, 0)));
    assert_eq!(page.float, Float::Closed);
    assert_eq!(leaves(page.when, now, &Utc), Ok(Leaves::At(day(25, 17, 0))));
}

/// A whole second a day from now: the store keeps what it is given, and the test compares it.
fn a_day_from_now() -> DateTime<Utc> {
    let later = Utc::now() + chrono::TimeDelta::days(1);
    Utc.timestamp_opt(later.timestamp(), 0)
        .single()
        .unwrap_or_else(|| panic!("a real instant"))
}

fn outbox(store: &SqliteStore, by: DateTime<Utc>) -> usize {
    store
        .outbox_due(ACCOUNT, by)
        .unwrap_or_else(|why| panic!("the outbox: {why}"))
        .len()
}

/// A page addressed and written, with the Sends row set to `at`, and its body as HTML.
fn scheduled_page(window: &mut Window, at: DateTime<Utc>) -> String {
    let mut page = window.page();
    window.dom.in_runtime(|| {
        let mut write = page.write();
        write.to = vec![dana()];
        write.subject = "Friday".to_owned();
        type_text(&mut write, "See you then.");
        write.when = When::At(at);
    });
    window.render();
    window.dom.in_runtime(|| to_html(&page.peek().session.doc))
}

fn fresh(store: &SqliteStore) -> Draft {
    crate::compose::draft_new(store, ACCOUNT, &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

#[tokio::test]
async fn scheduling_holds_the_draft_until_its_time_and_cancel_brings_back_the_page() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), None);
    let before_body = scheduled_page(&mut window, at);
    let far = at + chrono::TimeDelta::days(1);
    let (before_due, before_all) = (
        outbox(&store, at - chrono::TimeDelta::seconds(1)),
        outbox(&store, far),
    );

    // The button says Schedule now; it is the same element it was.
    let painted = click(&mut window.dom, seen.one("aria-label", "Send"));
    let markup = window.render();
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Scheduled { at }),
        "the draft is not held for its time"
    );
    assert_eq!(outbox(&store, far), before_all + 1, "nothing was queued");
    assert_eq!(
        outbox(&store, at - chrono::TimeDelta::seconds(1)),
        before_due,
        "the send may leave before its time"
    );
    assert!(
        markup.contains("Scheduled for "),
        "no scheduled pill:\n{markup}"
    );
    assert!(!markup.contains("Waiting in the outbox"), "{markup}");
    assert!(
        !markup.contains("Sending in"),
        "a scheduled send counted down:\n{markup}"
    );
    assert!(
        markup.contains(r#"class="cpage sending""#),
        "the page did not fold:\n{markup}"
    );
    // And it is listed in Today, with its time.
    assert_eq!(
        waiting(&store)
            .into_iter()
            .map(|one| (one.draft, one.at))
            .collect::<Vec<_>>(),
        [(draft.id, at)]
    );
    assert!(
        markup.contains(r#"class="today-at later""#),
        "not in Today:\n{markup}"
    );

    click(
        &mut window.dom,
        painted.fixed("class", "ds-send-pill-undo")[0],
    );
    let markup = window.render();
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Editing),
        "Cancel left it held"
    );
    assert_eq!(
        outbox(&store, far),
        before_all,
        "Cancel left the submission queued"
    );
    let page = window.page();
    let (html, phase, when) = window.dom.in_runtime(|| {
        let read = page.peek();
        (to_html(&read.session.doc), read.phase, read.when)
    });
    assert_eq!(
        html, before_body,
        "the page that came back is not the one scheduled"
    );
    assert_eq!(phase, Phase::Writing);
    assert_eq!(when, When::At(at), "the page forgot its time");
    assert!(
        !markup.contains("Scheduled for"),
        "the pill stayed:\n{markup}"
    );
    assert!(
        !markup.contains("today-at later"),
        "Today still lists it:\n{markup}"
    );
}

#[tokio::test]
async fn cancel_from_today_opens_the_draft_as_the_store_has_it() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), None);
    scheduled_page(&mut window, at);
    let painted = click(&mut window.dom, seen.one("aria-label", "Send"));
    let stored = store.draft(draft.id).unwrap_or_else(|why| panic!("{why}"));
    // The pill has gone: Today is the only way back.
    let desk = window.desk;
    window.dom.in_runtime(|| {
        let mut outbox = desk.outbox;
        outbox.set(None);
    });
    window.render();

    click(
        &mut window.dom,
        painted.one("aria-label", "Cancel sending Friday"),
    );
    window.render();
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Editing)
    );
    let page = window.page();
    let (text, phase) = window
        .dom
        .in_runtime(|| (to_flowed(&page.peek().session.doc), page.peek().phase));
    assert_eq!(text, stored.text, "the reopened page has another body");
    assert_eq!(phase, Phase::Writing);
    assert!(waiting(&store).is_empty());
}

#[tokio::test]
async fn cancel_of_a_send_already_on_the_wire_says_so_and_changes_nothing() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(store.clone(), draft.clone(), None);
    scheduled_page(&mut window, at);
    let painted = click(&mut window.dom, seen.one("aria-label", "Send"));
    let far = at + chrono::TimeDelta::days(1);
    let queued = outbox(&store, far);
    // The outbox took it between the pill being drawn and the press.
    store
        .set_send_state(draft.id, &SendState::Sending, Utc::now())
        .unwrap_or_else(|why| panic!("{why}"));

    click(
        &mut window.dom,
        painted.fixed("class", "ds-send-pill-undo")[0],
    );
    let markup = window.render();
    assert!(
        markup.contains("Too late to take it back: that message is already being sent"),
        "the refusal was not said:\n{markup}"
    );
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Sending),
        "the refused Cancel moved the draft"
    );
    assert_eq!(
        outbox(&store, far),
        queued,
        "the refused Cancel touched the outbox"
    );
    let still = window
        .dom
        .in_runtime(|| window.desk.outbox.peek().as_ref().map(|out| out.draft));
    assert_eq!(still, Some(draft.id), "the pill let go of the send");
}

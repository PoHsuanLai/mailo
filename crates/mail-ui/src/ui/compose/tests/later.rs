//! Send later in the window: the Sends choice as the outbox reads it, a scheduled send on the
//! pill and in the Scheduled place, and Cancel, against a real store.

use mail_core::Store;

use super::super::later::{choose_time, leaves, pick_time, scheduled};
use super::super::page::{Float, Phase, When};
use super::super::props::pick_sends;
use super::*;
use crate::ui::fixtures::{acct_account, click, pick_named, right_click, seeded, settle};
use crate::ui::space::Scope;
use mail_core::SqliteStore;
use mail_core::compose::Leaves;
use mail_editor::to_html;

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
            Err("Today 09:00 has passed"),
        ),
        (
            "this very minute",
            When::At(now),
            Err("Today 10:00 has passed"),
        ),
    ];
    for (name, when, want) in cases {
        let got = leaves(*when, now, &Utc, mail_logic::when::Clock::H24);
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
    // The phrases themselves are core's `snooze` tests; here is what the field adds to them.
    let cases: &[(&str, Result<DateTime<Utc>, &str>)] = &[
        ("tomorrow 9", Ok(day(24, 9, 0))),
        ("2026-09-01 10:00", Err("Tue 1 Sep 10:00 has passed")),
        ("today 8", Err("Today 08:00 has passed")),
        ("", Err("Type a time")),
    ];
    for (typed, want) in cases {
        let got = pick_time(typed, now, &Utc, mail_logic::when::Clock::H24);
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
    page.float = Float::SendMenu;
    super::super::foot::pick_send(&mut page, "later:at");
    assert_eq!(
        page.float,
        Float::PickTime(String::new()),
        "Send ▾ › Send Later › Pick a Time… did not open the field"
    );
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
    assert!(choose_time(&mut page, now, &Utc, mail_logic::when::Clock::H24).is_err());
    assert_eq!(page.when, When::Now);
    assert_eq!(page.float, Float::PickTime("today 8".to_owned()));

    page.float = Float::PickTime("fri 17:00".to_owned());
    assert_eq!(
        choose_time(&mut page, now, &Utc, mail_logic::when::Clock::H24),
        Ok(day(25, 17, 0))
    );
    assert_eq!(page.when, When::At(day(25, 17, 0)));
    assert_eq!(page.float, Float::Closed);
    assert_eq!(
        leaves(page.when, now, &Utc, mail_logic::when::Clock::H24),
        Ok(Leaves::At(day(25, 17, 0)))
    );
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
        .outbox_due(acct_account(), by)
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
    mail_core::compose::draft_new(store, acct_account(), &[], "", "", Utc::now())
        .unwrap_or_else(|why| panic!("a new draft: {why}"))
}

#[tokio::test]
async fn scheduling_holds_the_draft_until_its_time_and_cancel_brings_back_the_page() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    let before_body = scheduled_page(&mut window, at);
    let far = at + chrono::TimeDelta::days(1);
    let (before_due, before_all) = (
        outbox(&store, at - chrono::TimeDelta::seconds(1)),
        outbox(&store, far),
    );

    // The button still says Send: the Sends row says when. It is the same element it was.
    let painted = click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
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
    // And it is waiting in the store, and listed in the Scheduled place.
    assert_eq!(
        scheduled(&crate::ui::fixtures::client(&store), &Scope::All)
            .into_iter()
            .map(|one| (one.id, one.state))
            .collect::<Vec<_>>(),
        [(draft.id, SendState::Scheduled { at })]
    );
    let listed = window.render();
    assert!(
        listed.contains("data-scheduled=\"Friday\""),
        "not in Scheduled:\n{listed}"
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
    let listed = window.render();
    assert!(
        !listed.contains("data-scheduled=\"Friday\""),
        "Scheduled still lists it:\n{listed}"
    );
}

/// Cancel Sending, in a scheduled message's right-click menu, makes it a draft again, and the
/// undo queues it again for its time.
#[tokio::test]
async fn cancel_sending_from_scheduled_is_undone_to_its_time() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    scheduled_page(&mut window, at);
    // Pictures sent smaller are the page's choice, and the draft does not keep it.
    let mut page = window.page();
    window.dom.in_runtime(|| {
        page.write().pictures = mail_logic::picture::PictureSize::Small;
    });
    let sent = click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
    // The send is queued through the client, a moment after the click.
    let sent = sent.merge(crate::ui::fixtures::settle_quiet_seen(&mut window.dom).await);
    let far = at + chrono::TimeDelta::days(1);
    let queued = outbox(&store, far);
    let early = outbox(&store, at - chrono::TimeDelta::seconds(1));
    let desk = window.desk;
    window.dom.in_runtime(|| {
        let mut outbox = desk.outbox;
        outbox.set(None);
    });
    let drawn = sent.merge(crate::ui::fixtures::drain_seen(&mut window.dom));

    // A right click asks; it takes nothing back by itself.
    let asked = right_click(&mut window.dom, drawn.one("data-scheduled", "Friday"));
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Scheduled { at }),
        "a right click took the message back"
    );
    let opened = settle(&mut window.dom, asked);
    pick_named(
        &mut window.dom,
        &opened,
        crate::ui::copy::menus::CANCEL_SENDING,
    )
    .await;
    let markup = window.render();
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Editing)
    );
    assert_eq!(outbox(&store, far), queued - 1, "the send is still queued");
    assert!(scheduled(&crate::ui::fixtures::client(&store), &Scope::All).is_empty());
    assert!(
        !markup.contains("data-scheduled=\"Friday\""),
        "Scheduled still lists it:\n{markup}"
    );

    // The undo, as ⌘Z or the toast's Undo takes it: queued again for the same time.
    let shell = window.shell;
    let entry = window
        .dom
        .in_runtime(|| {
            let mut shell = shell;
            shell.write().undo.pop()
        })
        .unwrap_or_else(|| panic!("Cancel Sending left no undo"));
    let resent = window.dom.in_scope(dioxus_core::ScopeId::APP, || {
        entry
            .iter()
            .filter_map(|part| part.resend)
            .all(|resend| super::super::send_again(&crate::ui::fixtures::client(&store), resend))
    });
    assert!(resent, "the undo was refused");
    assert!(
        entry
            .iter()
            .filter_map(|part| part.resend)
            .all(|resend| resend.pictures == mail_logic::picture::PictureSize::Small),
        "the undo forgot the picture size the send was queued with"
    );
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Scheduled { at }),
        "the undo did not hold it for its time"
    );
    assert_eq!(outbox(&store, far), queued, "the undo queued nothing");
    assert_eq!(
        outbox(&store, at - chrono::TimeDelta::seconds(1)),
        early,
        "the undone send may leave before its time"
    );
}

/// A click on a scheduled message opens it, held: what it says and when it leaves, and nothing to
/// write in. The bar's Cancel Sending makes it a draft again and opens it in the composer.
#[tokio::test]
async fn a_scheduled_message_opens_held_and_cancel_writes_it() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    scheduled_page(&mut window, at);
    let sent = click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
    // The fold, settled: the page is gone, as its timer takes it away.
    let shell = window.shell;
    window.dom.in_runtime(|| {
        let mut shell = shell;
        shell.write().close_composer();
    });
    let drawn = sent.merge(crate::ui::fixtures::drain_seen(&mut window.dom));

    let opened = click(&mut window.dom, drawn.one("data-scheduled", "Friday"));
    let opened = settle(&mut window.dom, opened);
    let markup = window.render();
    assert!(
        markup.contains("data-held=\"Friday\""),
        "not opened held:\n{markup}"
    );
    let leaves = crate::ui::copy::status::leaves(&crate::ui::menus::when_words(
        at,
        Utc::now(),
        &chrono::Local,
        mail_logic::when::Clock::H24,
    ));
    assert!(
        markup.contains(&leaves),
        "the bar does not say {leaves:?}:\n{markup}"
    );
    assert!(
        !markup.contains("aria-label=\"Send\""),
        "a held message offers Send:\n{markup}"
    );
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Scheduled { at }),
        "opening it took it back"
    );

    click(
        &mut window.dom,
        opened.one("aria-label", crate::ui::copy::menus::CANCEL_SENDING),
    );
    let markup = window.render();
    assert_eq!(
        store.draft(draft.id).map(|d| d.state).ok(),
        Some(SendState::Editing)
    );
    assert!(scheduled(&crate::ui::fixtures::client(&store), &Scope::All).is_empty());
    assert!(!markup.contains("data-held"), "still held:\n{markup}");
    assert!(
        markup.contains("aria-label=\"Send\""),
        "the composer did not open on it:\n{markup}"
    );
}

/// The Scheduled place is the Space's: a message held on an account outside it is not listed.
#[test]
fn scheduled_lists_only_the_spaces_accounts() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    store
        .set_send_state(draft.id, &SendState::Scheduled { at }, Utc::now())
        .unwrap_or_else(|why| panic!("{why}"));
    let ids = |scope: &Scope| -> Vec<DraftId> {
        scheduled(&crate::ui::fixtures::client(&store), scope)
            .into_iter()
            .map(|one| one.id)
            .collect()
    };
    assert_eq!(ids(&Scope::All), [draft.id]);
    assert_eq!(
        ids(&Scope::Accounts(vec![draft.account.clone()])),
        [draft.id]
    );
    assert!(
        ids(&Scope::Accounts(vec![mail_domain::id::new_account_id()])).is_empty(),
        "another Space's message is listed"
    );
    assert!(ids(&Scope::Accounts(Vec::new())).is_empty());
}

#[tokio::test]
async fn cancel_of_a_send_already_on_the_wire_says_so_and_changes_nothing() {
    let (store, _dir) = seeded();
    let draft = fresh(&store);
    let at = a_day_from_now();
    let (mut window, seen) = Window::open(
        crate::ui::fixtures::client(&store.clone()),
        draft.clone(),
        None,
    );
    scheduled_page(&mut window, at);
    let painted = click(
        &mut window.dom,
        seen.one("aria-label", crate::ui::copy::controls::SEND),
    );
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
        markup.contains("Too late: that message is already being sent"),
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

//! Lane 8: Send later and Remind me. A message scheduled for tomorrow at 08:00 waits in the
//! outbox until then; once it has gone and nobody has answered for three days, the conversation
//! comes back saying so.

use ds_harness::Query;
use mail_core::Store;
use mail_domain::{FollowUp, SendState};

use super::drive::{Drive, Key};
use super::look::conversation;
use super::seed::sent_copy;
use super::window::{Window, account, queued};

const SUBJECT: &str = "The draft budget";

#[test]
fn a_message_sent_tomorrow_at_eight_waits_then_its_reminder_comes_back_with_no_reply() {
    let mut window = Window::open(|_| {});
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer with To ready", |h| {
        h.is_focused(".cpage .c-pin input")
    });
    window.type_text("ada@example.test\n");
    window.click(".c-title input");
    window.type_text(SUBJECT);
    window.click(".c-body");
    window.type_text("Numbers inside.");

    // Send: Tomorrow 08:00.
    window.click(".c-props [*|data-row=sends] .ds-button");
    window.menu_item("Tomorrow 08:00");
    window.until("the row says when it leaves", |h| {
        h.text_of(".c-props [*|data-row=sends]")
            .is_some_and(|row| row.contains("Tomorrow 08:00"))
    });

    // Remind me if no reply: in 3 days.
    window.click(".c-props [*|aria-label=\"Remind me if no reply\"]");
    window.menu_item("In 3 days");
    window.until("the row says the reminder", |h| {
        h.text_of(".c-props [*|data-row=remind]")
            .is_some_and(|row| row.contains("if no reply"))
    });

    let asked = window.now();
    window.click(".c-foot [*|aria-label=\"Schedule\"]");
    window.until("the pill says it is scheduled", |h| {
        h.text_of(".ds-send-pill")
            .is_some_and(|pill| pill.contains("Scheduled"))
    });

    // It waits in the outbox, due tomorrow at 08:00 on the window's clock.
    let drafts = window.store.drafts(account()).expect("the drafts");
    assert_eq!(drafts.len(), 1);
    let SendState::Scheduled { at } = drafts[0].state else {
        panic!("not scheduled: {:?}", drafts[0].state);
    };
    let local = at.with_timezone(&chrono::Local);
    assert_eq!(
        (local.date_naive(), local.format("%H:%M").to_string()),
        (
            asked.with_timezone(&chrono::Local).date_naive() + chrono::TimeDelta::days(1),
            "08:00".to_owned()
        )
    );
    let due = |when| {
        window
            .store
            .outbox_due(account(), when)
            .expect("the outbox")
    };
    assert!(
        due(at - chrono::TimeDelta::seconds(1)).is_empty(),
        "due before its time"
    );
    assert_eq!(due(at).len(), 1, "not due at its time");

    // It leaves at 08:00 (the send itself is the sync's; no server here), and the next sync
    // brings its copy back from the Sent folder.
    let sent = queued(&window.store);
    sent_copy(&window.store, "sent1", sent[0].raw.as_bytes());

    // The window's next sweep puts the reminder on it: three days from when it left.
    window.harness.wait(6 * 60_000);
    window.until_stored("the reminder joins its conversation", |store| {
        matches!(
            conversation(store, SUBJECT).follow_up,
            FollowUp::Until { .. }
        )
    });
    let FollowUp::Until { at: due_at, .. } = conversation(&window.store, SUBJECT).follow_up else {
        panic!("no reminder");
    };
    let days = (due_at - at).num_hours();
    assert!(
        (72..=96).contains(&days),
        "not three days after it left: {days} h"
    );

    // Its time, and nobody has answered: the conversation is back, said once.
    let wait = (due_at - window.now()).to_std().unwrap_or_default();
    window.harness.wait(wait.as_millis() as u64 + 2 * 60_000);
    window.until("the conversation comes back with no reply", |h| {
        h.text_of(".list .no-reply")
            .is_some_and(|said| said.contains("No reply yet"))
    });
    let said = window.notices();
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].summary, "No reply yet");
    assert_eq!(said[0].body, SUBJECT);
}

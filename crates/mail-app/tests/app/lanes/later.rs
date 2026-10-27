//! Lane 8: Send later and Remind me. A message scheduled for tomorrow at 08:00 waits in the
//! outbox until then; once it has gone its reminder joins the conversation, which the Waiting
//! place lists; when three days pass with nobody answering, the conversation comes back on top
//! of the inbox saying so, and a notification says so once.
//!
//! The window's wall clock starts at a fixed instant, so the dates asserted here never straddle
//! a daylight-saving change, and the days between are a [`Window::jump`] of that clock rather
//! than days of the harness's, which would stop at every timer on the way.

use chrono::{TimeDelta, TimeZone};
use mail_app::ui::copy;
use mail_core::Store;
use mail_core::follow_up::{CHOICES, returned_words};
use mail_core::notify::Opens;
use mail_domain::{FollowUp, SendState};

use super::drive::{Drive, Key};
use super::look::{SUBMENU_LIST, conversation};
use super::seed::sent_copy;
use super::window::{INBOX, Window, account, queued, row_of};

const SUBJECT: &str = "The draft budget";

/// Long enough for the reminders' sweep, which sleeps at most five minutes, to run again.
const A_SWEEP: u64 = 6 * 60_000;

/// What a returned conversation's row says.
fn no_reply() -> &'static str {
    let returned = FollowUp::Returned {
        at: chrono::Utc::now(),
        set: chrono::Utc::now(),
    };
    returned_words(&returned).expect("a returned conversation says so")
}

#[test]
fn a_message_sent_tomorrow_at_eight_waits_then_its_reminder_comes_back_with_no_reply() {
    let base = chrono::Utc
        .with_ymd_and_hms(2026, 7, 14, 10, 0, 0)
        .single()
        .expect("a fixed instant");
    let mut window = Window::open_at(base, |_| {});
    window.press(&[], Key::Char('c'), 1);
    window.until("c opens a composer with To ready", |h| {
        h.is_focused(".cpage .c-pin input")
    });
    window.type_text("ada@example.test\n");
    window.click(".c-title input");
    window.type_text(SUBJECT);
    window.click(".c-body");
    window.type_text("Numbers inside.");

    // Send ▾ › Send Later › Tomorrow 08:00, its first choice. No Sends row until then.
    assert_eq!(window.harness.count(".c-props [*|data-row=sends]"), 0);
    window.click(&format!(
        ".c-send [*|aria-label=\"{}\"]",
        copy::aria::SEND_OPTIONS
    ));
    window.menu_item(copy::menus::SEND_LATER);
    window.click(&format!("{SUBMENU_LIST} > :nth-child(1)"));
    window.until("the Sends row says when it leaves", |h| {
        h.count(".c-props [*|data-row=sends]") == 1
    });

    // Send ▾ › Remind Me If No Reply › In 3 days.
    assert_eq!(window.harness.count(".c-props [*|data-row=remind]"), 0);
    window.click(&format!(
        ".c-send [*|aria-label=\"{}\"]",
        copy::aria::SEND_OPTIONS
    ));
    window.submenu_item(copy::menus::REMIND_IF_NO_REPLY, CHOICES[1].1);
    window.until("the Remind row says the reminder", |h| {
        h.text_of(".c-props [*|data-row=remind]")
            .is_some_and(|row| row.contains(CHOICES[1].1))
    });

    let asked = window.now();
    window.click(&super::look::send());
    window.until_stored("the draft is scheduled", |store| {
        store.drafts(account()).is_ok_and(|drafts| {
            drafts
                .iter()
                .any(|draft| matches!(draft.state, SendState::Scheduled { .. }))
        })
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
            asked.with_timezone(&chrono::Local).date_naive() + TimeDelta::days(1),
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
        due(at - TimeDelta::seconds(1)).is_empty(),
        "due before its time"
    );
    assert_eq!(due(at).len(), 1, "not due at its time");

    // It leaves at 08:00 (the send itself is the sync's; no server here), and the next sync
    // brings its copy back from the Sent folder.
    let sent = queued(&window.store);
    sent_copy(&window.store, "sent1", sent[0].raw.as_bytes());

    // The window's next sweep puts the reminder on it: three days from when it left.
    window.harness.wait(A_SWEEP);
    window.until_stored("the reminder joins its conversation", |store| {
        matches!(
            conversation(store, SUBJECT).follow_up,
            FollowUp::Until { .. }
        )
    });
    let thread = conversation(&window.store, SUBJECT);
    let FollowUp::Until { at: due_at, .. } = thread.follow_up else {
        panic!("no reminder");
    };
    let hours = (due_at - at).num_hours();
    assert!(
        (72..=96).contains(&hours),
        "not three days after it left: {hours} h"
    );

    // The Waiting place lists it; the inbox does not, since only the user has written.
    window.click("[*|data-place=\"Waiting\"]");
    window.until("the Waiting place lists it", |h| {
        row_of(h, SUBJECT).is_some() && h.count(".list .ds-thread") == 1
    });
    window.click("[*|data-place=\"Inbox\"]");
    window.until("the inbox is as it was", |h| {
        h.count(".list .ds-thread") == INBOX.len()
    });

    // Ten minutes before its time, a sweep later: still waiting, and nothing said.
    window.jump(due_at - window.now() - TimeDelta::minutes(10));
    window.harness.wait(A_SWEEP);
    assert!(
        matches!(
            conversation(&window.store, SUBJECT).follow_up,
            FollowUp::Until { .. }
        ),
        "it came back early"
    );
    assert_eq!(window.harness.count(".list .no-reply"), 0);
    assert!(window.notices().is_empty());

    // Its time, and nobody has answered: back on top of the inbox, saying so, and said once.
    window.harness.wait(A_SWEEP);
    window.until("the conversation comes back on top with no reply", |h| {
        row_of(h, SUBJECT) == Some(1)
            && h.text_of(".list .no-reply")
                .is_some_and(|said| said.contains(no_reply()))
    });
    assert_eq!(
        window.harness.count(".list .no-reply"),
        1,
        "only its row says it"
    );
    let said = window.notices();
    assert_eq!(said.len(), 1, "{said:?}");
    assert_eq!(said[0].summary, no_reply());
    assert_eq!(said[0].body, SUBJECT);
    assert_eq!(said[0].opens, Opens::Thread(thread.id));

    // A day on, nothing is said again.
    window.jump(TimeDelta::days(1));
    window.harness.wait(A_SWEEP);
    assert_eq!(window.notices().len(), 1, "announced once");
}

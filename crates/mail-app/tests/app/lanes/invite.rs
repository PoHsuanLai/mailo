//! Lane 4: an invitation. The card shows on the message; Accept with a note sends an iMIP REPLY,
//! Change answer then Decline sends another, and Save .ics writes the event where the window
//! saves files.

use ds_harness::Query;

use super::drive::Key;
use super::window::{Window, deliver, hours_ago, queued};

const SUBJECT: &str = "Invitation: Design review";
const CARD: &str = "section.invite";

/// The invitation's calendar, the person the window is for among its attendees.
const CALENDAR: &str = "BEGIN:VCALENDAR\r\nPRODID:-//Example//Calendar//EN\r\nVERSION:2.0\r\n\
METHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:review-7@example.test\r\nSEQUENCE:0\r\n\
DTSTAMP:20260920T101500Z\r\nDTSTART:20261105T130000Z\r\nDTEND:20261105T140000Z\r\n\
SUMMARY:Design review\r\nLOCATION:Room 2\r\nSTATUS:CONFIRMED\r\n\
ORGANIZER;CN=Ada Lovelace:mailto:ada@example.test\r\n\
ATTENDEE;CN=Ada Lovelace;PARTSTAT=ACCEPTED:mailto:ada@example.test\r\n\
ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.test\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";

fn invited(store: &mail_core::SqliteStore) {
    let raw = format!(
        "From: Ada Lovelace <ada@example.test>\r\nTo: Me <me@example.test>\r\n\
         Subject: {SUBJECT}\r\nDate: {}\r\nMessage-ID: <invite7@example.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n\
         --alt\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n\
         You are invited to Design review.\r\n\
         --alt\r\nContent-Type: text/calendar; charset=UTF-8; method=REQUEST\r\n\r\n\
         {CALENDAR}--alt--\r\n",
        hours_ago(0)
    );
    deliver(store, "invite7", &raw);
}

/// The Answer control's segment `n`: Accept, Maybe, Decline.
fn answer(n: usize) -> String {
    format!("{CARD} .inv-acts .ds-segmented-segment:nth-child({n})")
}

#[test]
fn an_invitation_is_accepted_with_a_note_then_declined_and_saved_as_ics() {
    let mut window = Window::open(invited);
    window.open_subject(SUBJECT);
    window.until("the invitation's card shows", |h| {
        h.text_of(&format!("{CARD} .inv-title"))
            .is_some_and(|title| title.contains("Design review"))
    });
    assert!(queued(&window.store).is_empty(), "opening it answered");

    // Accept: a note field opens; the note, then Enter, sends the answer.
    window.click(&answer(1));
    window.until("Accept asks for a note", |h| {
        h.count(&format!("{CARD} .inv-noting")) == 1
    });
    window.click(&format!("{CARD} .inv-note-field input"));
    window.type_text("Glad to join");
    window.press(&[], Key::Enter, 1);
    window.until("the card says it was accepted", |h| {
        h.text_of(CARD)
            .is_some_and(|card| card.contains("You accepted"))
    });
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 1, "not one answer");
    assert_eq!(sent[0].rcpt_to, ["ada@example.test"]);
    let raw = sent[0].raw.replace("\r\n ", "");
    for said in [
        "METHOD:REPLY",
        "PARTSTAT=ACCEPTED",
        "UID:review-7@example.test",
    ] {
        assert!(raw.contains(said), "no {said} in the answer:\n{raw}");
    }
    assert!(raw.contains("Glad to join"), "the note did not go:\n{raw}");

    // Change answer, Decline, Send answer: a second reply, declining.
    window.click(&format!("{CARD} [*|aria-label=\"Change answer\"]"));
    window.until("the answers are offered again", |h| {
        h.count(&answer(3)) == 1
    });
    window.click(&answer(3));
    window.click(&format!("{CARD} [*|aria-label=\"Send answer\"]"));
    window.until("the card says it was declined", |h| {
        h.text_of(CARD)
            .is_some_and(|card| card.contains("You declined"))
    });
    let sent = queued(&window.store);
    assert_eq!(sent.len(), 2, "the second answer was not queued");
    assert!(
        sent.iter()
            .any(|one| one.raw.replace("\r\n ", "").contains("PARTSTAT=DECLINED")),
        "no answer declines"
    );

    // Save .ics: the event lands where the window saves files, named for it.
    window.click(&format!("{CARD} [*|aria-label=\"Save .ics\"]"));
    let saved = window.saves().join("Design review.ics");
    window.until("the toast says where it was saved", |h| {
        h.text_of(".ds-toast-body")
            .is_some_and(|toast| toast.starts_with("Saved to"))
    });
    let bytes = std::fs::read_to_string(&saved).expect("the .ics was saved");
    for said in [
        "BEGIN:VCALENDAR",
        "UID:review-7@example.test",
        "SUMMARY:Design review",
    ] {
        assert!(
            bytes.contains(said),
            "no {said} in the saved file:\n{bytes}"
        );
    }
}

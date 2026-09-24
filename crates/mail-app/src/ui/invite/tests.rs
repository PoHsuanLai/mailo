//! Invitations in the window. Opening shows the card and answers nothing; only Send does.

use super::looked_at;
use crate::ui::files::SaveDir;
use crate::ui::fixtures::{
    ACCOUNT, Seen, chord, click, dispatching, rebuild_into, seeded, type_into,
};
use crate::ui::reading::Reader;
use crate::view::Shell;
use dioxus::html::input_data::keyboard_types::Modifiers;
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

/// The address the fixture's account sends from.
pub(super) const ME: &str = "me@example.test";

/// A calendar object: `method`, `sequence`, `status`, organised by `organiser`, asking
/// `attendees` as `(name, address, PARTSTAT)`, with `extra` lines inside the event.
pub(super) fn ics(
    method: &str,
    sequence: u32,
    status: &str,
    organiser: &str,
    attendees: &[(&str, &str, &str)],
    extra: &str,
) -> String {
    let mut people = String::new();
    for (name, address, answer) in attendees {
        people.push_str(&format!(
            "ATTENDEE;CN={name};PARTSTAT={answer};RSVP=TRUE:mailto:{address}\r\n"
        ));
    }
    format!(
        "BEGIN:VCALENDAR\r\n\
         PRODID:-//Example//Calendar//EN\r\n\
         VERSION:2.0\r\n\
         METHOD:{method}\r\n\
         BEGIN:VEVENT\r\n\
         UID:review-7@example.test\r\n\
         SEQUENCE:{sequence}\r\n\
         DTSTAMP:20260920T101500Z\r\n\
         DTSTART;TZID=Europe/Berlin:20261005T140000\r\n\
         DTEND;TZID=Europe/Berlin:20261005T150000\r\n\
         SUMMARY:Design review\r\n\
         LOCATION:Room 2\r\n\
         STATUS:{status}\r\n\
         ORGANIZER;CN=Ada Lovelace:mailto:{organiser}\r\n\
         {people}{extra}\
         END:VEVENT\r\n\
         END:VCALENDAR\r\n"
    )
}

/// The usual invitation to [`ME`], from Ada, with Charles declined.
pub(super) fn request(sequence: u32) -> String {
    ics(
        "REQUEST",
        sequence,
        "CONFIRMED",
        "ada@example.test",
        &[
            ("Ada Lovelace", "ada@example.test", "ACCEPTED"),
            ("Me", ME, "NEEDS-ACTION"),
            ("Charles", "charles@example.test", "DECLINED"),
        ],
        "",
    )
}

/// A message from Ada carrying `calendar` as the alternative calendar servers send, in its own
/// thread. Returns the thread and the message.
fn put(store: &SqliteStore, calendar: &str, method: &str) -> (ThreadId, MessageId) {
    let rfc = format!("{}@example.test", uuid::Uuid::new_v4());
    let bytes = format!(
        "From: Ada Lovelace <ada@example.test>\r\nTo: {ME}\r\n\
         Subject: Invitation: Design review\r\nMessage-ID: <{rfc}>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n\
         --alt\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n\
         You are invited to Design review.\r\n\
         --alt\r\nContent-Type: text/calendar; charset=UTF-8; method={method}\r\n\r\n\
         {calendar}--alt--\r\n"
    );
    let raw = store
        .blobs()
        .put(&store.connection(), bytes.as_bytes())
        .unwrap();
    let id = MessageId::generate();
    let message = Message {
        id,
        thread: ThreadId::generate(),
        account: ACCOUNT,
        key: MessageKey::Rfc(rfc.clone()),
        date: chrono::Utc::now(),
        from: Address {
            name: Some("Ada Lovelace".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: ME.to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "Invitation: Design review".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some(rfc.clone()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: Body::Present {
            text: Some("You are invited to Design review.".to_owned()),
            raw,
        },
        attachments: vec![],
    };
    store
        .ingest(
            ACCOUNT,
            Ingest {
                mailbox: MailboxRef {
                    account: ACCOUNT,
                    path: "INBOX".to_owned(),
                },
                validity: UidValidity::Same,
                cursor: Some(SyncCursor::Pop),
                messages: vec![Fetched {
                    remote: RemoteRef::Pop { uidl: rfc },
                    key: message.key.clone(),
                    raw,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    (store.message(id).unwrap().thread, id)
}

fn far() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now() + chrono::TimeDelta::days(365)
}

/// Every submission waiting in the outbox, as its frozen bytes.
fn submissions(store: &SqliteStore) -> Vec<String> {
    store
        .outbox_due(ACCOUNT, far())
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::Submit { raw, .. } => Some(
                String::from_utf8(store.blobs().get(&store.connection(), raw).unwrap()).unwrap(),
            ),
            _ => None,
        })
        .collect()
}

fn outbox(store: &SqliteStore) -> usize {
    store.outbox_due(ACCOUNT, far()).unwrap().len()
}

#[derive(Clone)]
struct Saves(std::path::PathBuf);

#[component]
fn Open(thread: ThreadId) -> Element {
    let shell = use_signal(Shell::default);
    let saves = use_context::<Saves>();
    use_context_provider(|| SaveDir(saves.0.clone()));
    rsx! { Reader { thread, shell } }
}

/// Let the dom's tasks run for `for_ms`, keeping every attribute the renders set.
async fn settle(dom: &mut VirtualDom, seen: &mut Seen, for_ms: u64) {
    let until = tokio::time::Instant::now() + std::time::Duration::from_millis(for_ms);
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = tokio::time::timeout(left, dom.wait_for_work()).await;
        dom.render_immediate(seen);
    }
}

fn markup(dom: &VirtualDom) -> String {
    dioxus_ssr::render(dom).replace("&#39;", "'")
}

/// The reader on `thread`, saving into `saves`, once its lookups have landed.
async fn reader_on(
    store: Arc<SqliteStore>,
    thread: ThreadId,
    saves: &std::path::Path,
) -> (VirtualDom, Seen, String) {
    dispatching();
    let mut dom = VirtualDom::new_with_props(Open, OpenProps { thread })
        .with_root_context(store)
        .with_root_context(Saves(saves.to_owned()));
    let mut seen = rebuild_into(&mut dom);
    settle(&mut dom, &mut seen, 400).await;
    let page = markup(&dom);
    (dom, seen, page)
}

#[tokio::test]
async fn opening_an_invitation_shows_the_card_and_answers_nothing() {
    let (store, dir) = seeded();
    let (thread, message) = put(&store, &request(0), "REQUEST");
    let queued = outbox(&store);
    let (_, _, page) = reader_on(store.clone(), thread, dir.path()).await;
    // It was looked at and shown: the assertions below are about a reader that did its work.
    assert!(looked_at(message), "the card never looked");
    assert!(page.contains("class=\"invite\""), "{page}");
    assert!(page.contains("Design review") && page.contains("aria-label=\"Accept\""));
    // At the top of the message, above its body, and not inside the sender's HTML.
    let card = page.find("class=\"invite\"").unwrap();
    assert!(card < page.find("You are invited to Design review.").unwrap());
    assert!(card > page.find("class=\"reader-body\"").unwrap());
    assert_eq!(store.invite_answer(message).unwrap(), None);
    assert_eq!(outbox(&store), queued, "opening queued something");
}

#[tokio::test]
async fn accept_with_a_note_queues_one_reply_and_change_answer_queues_another() {
    let (store, dir) = seeded();
    let (thread, message) = put(&store, &request(0), "REQUEST");
    let before = submissions(&store).len();
    let (mut dom, seen, _) = reader_on(store.clone(), thread, dir.path()).await;

    // Choosing an answer opens its note, and sends nothing yet.
    let mut seen = click(&mut dom, seen.one("aria-label", "Accept"));
    settle(&mut dom, &mut seen, 50).await;
    assert_eq!(submissions(&store).len(), before, "choosing sent");
    let field = seen.one("aria-placeholder", "Add a note (optional)");
    type_into(&mut dom, field, "See you there");
    let mut seen = chord(&mut dom, "Enter", Modifiers::empty(), field);
    settle(&mut dom, &mut seen, 400).await;

    let sent = submissions(&store);
    assert_eq!(
        sent.len(),
        before + 1,
        "one answer, queued: {}",
        markup(&dom)
    );
    let reply = sent.last().unwrap();
    assert!(reply.contains("METHOD:REPLY"), "{reply}");
    assert!(reply.contains("PARTSTAT=ACCEPTED"), "{reply}");
    let kept = store.invite_answer(message).unwrap().unwrap();
    assert_eq!(kept.attendance, Attendance::Accepted);
    assert_eq!(kept.comment.as_deref(), Some("See you there"));
    let page = markup(&dom);
    assert!(
        page.contains("<span class=\"said\">You accepted</span>"),
        "{page}"
    );
    assert!(!page.contains("aria-label=\"Accept\""), "{page}");

    // Change answer offers the three again; Decline and Send queue one more.
    let mut seen = click(&mut dom, seen.one("aria-label", "Change answer"));
    settle(&mut dom, &mut seen, 50).await;
    let mut seen = click(&mut dom, seen.one("aria-label", "Decline"));
    settle(&mut dom, &mut seen, 50).await;
    let mut seen = click(&mut dom, seen.one("aria-label", "Send answer"));
    settle(&mut dom, &mut seen, 400).await;
    let _ = seen;

    let sent = submissions(&store);
    assert_eq!(sent.len(), before + 2, "a second answer, queued");
    assert!(sent.iter().any(|raw| raw.contains("PARTSTAT=DECLINED")));
    assert_eq!(
        store.invite_answer(message).unwrap().unwrap().attendance,
        Attendance::Declined
    );
    assert!(markup(&dom).contains("<span class=\"said\">You declined</span>"));
}

#[tokio::test]
async fn escape_takes_the_note_back_and_sends_nothing() {
    let (store, dir) = seeded();
    let (thread, message) = put(&store, &request(0), "REQUEST");
    let before = submissions(&store).len();
    let (mut dom, seen, _) = reader_on(store.clone(), thread, dir.path()).await;
    let mut seen = click(&mut dom, seen.one("aria-label", "Maybe"));
    settle(&mut dom, &mut seen, 50).await;
    let field = seen.one("aria-placeholder", "Add a note (optional)");
    let mut seen = chord(&mut dom, "Escape", Modifiers::empty(), field);
    settle(&mut dom, &mut seen, 100).await;
    assert!(!markup(&dom).contains("Add a note"), "the note stayed open");
    assert_eq!(submissions(&store).len(), before);
    assert_eq!(store.invite_answer(message).unwrap(), None);
}

#[tokio::test]
async fn a_cancellation_and_an_organisers_own_invitation_offer_nothing_to_press() {
    let (store, dir) = seeded();
    let cancelled = ics(
        "CANCEL",
        1,
        "CANCELLED",
        "ada@example.test",
        &[("Me", ME, "NEEDS-ACTION")],
        "",
    );
    let mine = ics(
        "REQUEST",
        0,
        "CONFIRMED",
        ME,
        &[("Ada Lovelace", "ada@example.test", "NEEDS-ACTION")],
        "",
    );
    for (case, calendar, method, says) in [
        (
            "cancelled",
            cancelled,
            "CANCEL",
            "This event will not take place.",
        ),
        (
            "organiser",
            mine,
            "REQUEST",
            "You organised this event, so there is nothing to answer.",
        ),
    ] {
        let (thread, _) = put(&store, &calendar, method);
        let queued = outbox(&store);
        let (_, seen, page) = reader_on(store.clone(), thread, dir.path()).await;
        assert!(page.contains(says), "{case}: {page}");
        for label in ["Accept", "Maybe", "Decline", "Send answer", "Change answer"] {
            assert!(seen.get("aria-label", label).is_none(), "{case}: {label}");
            assert!(
                !page.contains(&format!("aria-label=\"{label}\"")),
                "{case}: {label}"
            );
        }
        assert_eq!(outbox(&store), queued, "{case}");
    }
}

#[tokio::test]
async fn save_ics_writes_the_event_beside_what_is_there_and_never_over_it() {
    let (store, _dir) = seeded();
    let saves = tempfile::tempdir().unwrap();
    let (thread, _) = put(&store, &request(0), "REQUEST");
    let (mut dom, seen, _) = reader_on(store.clone(), thread, saves.path()).await;
    let save = seen.one("aria-label", "Save .ics");
    std::fs::write(saves.path().join("Design review.ics"), b"already here").unwrap();

    let mut seen = click(&mut dom, save);
    settle(&mut dom, &mut seen, 300).await;

    let mut names: Vec<String> = std::fs::read_dir(saves.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 2, "{names:?}");
    assert_eq!(
        std::fs::read(saves.path().join("Design review.ics")).unwrap(),
        b"already here",
        "the file that was there was overwritten"
    );
    let written = names
        .iter()
        .find(|name| *name != "Design review.ics")
        .unwrap();
    assert!(written.ends_with(".ics"), "{written}");
    let body = std::fs::read_to_string(saves.path().join(written)).unwrap();
    assert!(body.starts_with("BEGIN:VCALENDAR"), "{body}");
    assert!(body.contains("METHOD:REQUEST"), "{body}");
}

#[test]
fn saving_names_the_file_for_the_event_and_says_where() {
    let (store, _dir) = seeded();
    let saves = tempfile::tempdir().unwrap();
    let (_, message) = put(&store, &request(0), "REQUEST");
    let said = super::save_ics(&store, message, "Design review", saves.path()).unwrap();
    let path = saves.path().join("Design review.ics");
    assert_eq!(said, format!("Saved to {}", path.display()));
    let unnamed = super::save_ics(&store, message, "(no title)", saves.path()).unwrap();
    assert!(unnamed.ends_with("invitation.ics"), "{unnamed}");
}

#[test]
fn the_answer_says_who_it_goes_to() {
    let (store, _dir) = seeded();
    let (_, message) = put(&store, &request(0), "REQUEST");
    let (said, card) = super::answer(
        &store,
        message,
        Attendance::Tentative,
        None,
        chrono::Utc::now(),
    )
    .unwrap();
    assert_eq!(said, "Queued your answer (tentative) to ada@example.test");
    assert!(matches!(
        card.map(|card| card.stand),
        Some(super::Stand::Answered { said, note: None }) if said == "You said maybe"
    ));
}

#[tokio::test]
#[ignore = "writes target/invite.html for a person or a headless browser to look at"]
async fn render_the_invitation_card_to_a_file() {
    let (store, dir) = seeded();
    let mut body = String::new();
    let update = ics(
        "REQUEST",
        2,
        "CONFIRMED",
        "ada@example.test",
        &[
            ("Ada Lovelace", "ada@example.test", "ACCEPTED"),
            ("Me", ME, "NEEDS-ACTION"),
            ("Charles", "charles@example.test", "DECLINED"),
            ("Grace", "grace@example.test", "TENTATIVE"),
            ("Alan", "alan@example.test", "ACCEPTED"),
            ("Edsger", "edsger@example.test", "NEEDS-ACTION"),
            ("Barbara", "barbara@example.test", "ACCEPTED"),
            ("Donald", "donald@example.test", "NEEDS-ACTION"),
            ("Frances", "frances@example.test", "ACCEPTED"),
        ],
        "RRULE:FREQ=WEEKLY;BYDAY=MO\r\n\
         DESCRIPTION:We will walk through the new reader and the invitation card.\\n\
         Bring questions.\\nThe notes from last week are in the shared folder.\\n\
         Agenda: layout\\, motion\\, and the dark theme.\\nThen lunch.\r\n",
    );
    let cancelled = ics(
        "CANCEL",
        3,
        "CANCELLED",
        "ada@example.test",
        &[("Me", ME, "NEEDS-ACTION")],
        "",
    );
    let (first, _) = put(&store, &request(0), "REQUEST");
    let (updated, _) = put(&store, &update, "REQUEST");
    let (gone, _) = put(&store, &cancelled, "CANCEL");
    let (answered, answered_id) = put(&store, &request(0), "REQUEST");
    crate::invite::answer(
        &store,
        answered_id,
        Attendance::Accepted,
        Some("See you there"),
        chrono::Utc::now(),
    )
    .unwrap();
    for thread in [first, updated, gone, answered] {
        let (_, _, reader) = reader_on(store.clone(), thread, dir.path()).await;
        body.push_str(&format!(
            "<section class=\"reader\" style=\"width:640px;margin:16px;display:inline-block;\
             vertical-align:top\">{reader}</section>"
        ));
    }
    crate::ui::fixtures::dump("invite", &body);
}

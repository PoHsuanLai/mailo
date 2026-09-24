//! Calendar invitations from the command line: `show` says a message invites, `invite` shows,
//! answers and exports it.
//!
//! Every answer is one the user typed. Nothing here answers an invitation because it was
//! opened. The invitations are synthetic, in the shape calendar servers send them.

use chrono::{DateTime, TimeZone, Utc};
use mail_app::cli;
use mail_app::invite::{InviteAction, InviteCommand};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

const ACCOUNT: AccountId =
    AccountId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"));
const IDENTITY: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b1"));
const ALIAS: IdentityId =
    IdentityId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000b2"));
const INVITE: MessageId = MessageId::from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000c1"));

fn at(n: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_790_000_000 + n, 0).unwrap()
}

fn exercise(store: &SqliteStore, command: &cli::Command) -> Result<String, String> {
    cli::run_with_clients(
        store,
        command,
        at(100),
        &mail_runtime::OAuthRegistry::default(),
    )
}

/// A calendar object inviting `attendee`, with `method`, `sequence` and `status`.
fn calendar(method: &str, attendee: &str, sequence: u32, status: &str) -> String {
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
         ORGANIZER;CN=Ada Lovelace:mailto:ada@example.test\r\n\
         ATTENDEE;CN=Ada Lovelace;PARTSTAT=ACCEPTED;ROLE=CHAIR:mailto:ada@example.test\r\n\
         ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{attendee}\r\n\
         ATTENDEE;CN=Charles;PARTSTAT=DECLINED:mailto:charles@example.test\r\n\
         END:VEVENT\r\n\
         END:VCALENDAR\r\n"
    )
}

/// The message calendar servers send: text and calendar as alternatives.
fn message_with(calendar: &str, method: &str) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.test>\r\n\
         To: me@example.test\r\n\
         Subject: Invitation: Design review\r\n\
         Message-ID: <review-7-mail@example.test>\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/alternative; boundary=\"alt\"\r\n\
         \r\n\
         --alt\r\n\
         Content-Type: text/plain; charset=UTF-8\r\n\
         \r\n\
         You are invited to Design review.\r\n\
         --alt\r\n\
         Content-Type: text/calendar; charset=UTF-8; method={method}\r\n\
         \r\n\
         {calendar}\
         --alt--\r\n"
    )
    .into_bytes()
}

fn request(attendee: &str) -> Vec<u8> {
    message_with(&calendar("REQUEST", attendee, 0, "CONFIRMED"), "REQUEST")
}

/// A store with one account, its default identity `me@example.test` (reply-to
/// `me.reply@example.test`) and an alternate identity `alias@example.test`, holding one message
/// built from `raw` — or its headers only when `raw` is `None`.
fn seeded(raw: Option<Vec<u8>>) -> (SqliteStore, tempfile::TempDir, ThreadId) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    {
        let db = store.connection();
        db.execute(
            "INSERT INTO accounts (id, address, plan, created_at)
             VALUES (?1, 'me@example.test', '{}', datetime('now'))",
            [ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, reply_to, is_default)
             VALUES (?1, ?2, 'Me', 'me@example.test',
                     '{\"name\":null,\"email\":\"me.reply@example.test\"}', '\"default\"')",
            rusqlite::params![IDENTITY.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
        db.execute(
            "INSERT INTO identities (id, account, from_name, from_email, is_default)
             VALUES (?1, ?2, 'Me (alias)', 'alias@example.test', '\"alternate\"')",
            rusqlite::params![ALIAS.to_string(), ACCOUNT.to_string()],
        )
        .unwrap();
    }
    let blob = store
        .blobs()
        .put(&store.connection(), raw.as_deref().unwrap_or(b"headers"))
        .unwrap();
    let thread = ThreadId::generate();
    let message = Message {
        id: INVITE,
        thread,
        account: ACCOUNT,
        key: MessageKey::Rfc("review-7-mail@example.test".to_owned()),
        date: at(0),
        from: Address {
            name: Some("Ada Lovelace".to_owned()),
            email: "ada@example.test".to_owned(),
        },
        reply_to: vec![],
        to: vec![Address {
            name: None,
            email: "me@example.test".to_owned(),
        }],
        cc: vec![],
        bcc: vec![],
        subject: "Invitation: Design review".to_owned(),
        in_reply_to: None,
        references: vec![],
        rfc_message_id: Some("review-7-mail@example.test".to_owned()),
        read: ReadState::Unread,
        star: Star::Unstarred,
        mailbox: MailboxRole::Inbox,
        labels: vec![],
        body: match raw {
            Some(_) => Body::Present {
                text: Some("You are invited to Design review.".to_owned()),
                raw: blob,
            },
            None => Body::Absent,
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
                cursor: None,
                messages: vec![Fetched {
                    remote: RemoteRef::Imap {
                        mailbox: "INBOX".to_owned(),
                        uidvalidity: 7,
                        uid: 42,
                    },
                    key: message.key.clone(),
                    raw: blob,
                    message,
                }],
                flags: vec![],
                labels: vec![],
                label_names: Vec::new(),
                gone: vec![],
            },
        )
        .unwrap();
    (store, dir, thread)
}

/// Every submission queued, as `(mail_from, rcpt_to, bytes)`.
fn submissions(store: &SqliteStore) -> Vec<(String, Vec<String>, String)> {
    store
        .outbox_due(ACCOUNT, at(1_000_000))
        .unwrap()
        .into_iter()
        .filter_map(|entry| match entry.op {
            ProtoOp::Submit {
                raw,
                mail_from,
                rcpt_to,
                ..
            } => {
                let bytes = store.blobs().get(&store.connection(), raw).unwrap();
                Some((mail_from, rcpt_to, String::from_utf8(bytes).unwrap()))
            }
            _ => None,
        })
        .collect()
}

fn invite(action: InviteAction) -> cli::Command {
    cli::Command::Invite(InviteCommand {
        message: INVITE,
        action,
    })
}

fn answer(attendance: Attendance, comment: Option<&str>) -> cli::Command {
    invite(InviteAction::Answer {
        attendance,
        comment: comment.map(str::to_owned),
    })
}

/// The `text/calendar` part of a queued answer, decoded.
fn calendar_of(bytes: &str) -> mail_pim::Calendar {
    let part = mail_mime::calendar_part(bytes.as_bytes()).expect("a calendar part");
    assert_eq!(part.method.as_deref(), Some("REPLY"));
    mail_pim::ical::parse(&part.text).unwrap()
}

#[test]
fn show_says_a_message_invites_and_how_to_see_it_without_answering() {
    let (store, _dir, thread) = seeded(Some(request("me@example.test")));
    let out = exercise(&store, &cli::Command::Show { thread }).unwrap();
    assert!(out.contains("invitation: Design review"), "{out}");
    assert!(out.contains(&format!("mailo invite {INVITE}")), "{out}");
    assert!(submissions(&store).is_empty());
    assert_eq!(store.invite_answer(INVITE).unwrap(), None);
}

#[test]
fn invite_shows_the_event_its_people_and_how_to_answer() {
    let (store, _dir, _) = seeded(Some(request("me@example.test")));
    let out = exercise(&store, &invite(InviteAction::Show)).unwrap();
    for expected in [
        "Design review",
        "an invitation",
        "where:     Room 2",
        "organiser: Ada Lovelace <ada@example.test>",
        "Ada Lovelace <ada@example.test>  accepted",
        "Me <me@example.test>  not answered",
        "Charles <charles@example.test>  declined",
        "your answer: not answered",
        "accept|tentative|decline",
    ] {
        assert!(out.contains(expected), "{expected:?} in\n{out}");
    }
}

#[test]
fn accepting_queues_a_reply_to_the_organiser_naming_only_the_user_and_records_it() {
    let (store, _dir, _) = seeded(Some(request("me@example.test")));
    let out = exercise(
        &store,
        &answer(Attendance::Accepted, Some("Looking forward to it")),
    )
    .unwrap();
    assert!(out.contains("accepted"), "{out}");
    assert!(out.contains("ada@example.test"), "{out}");

    let sent = submissions(&store);
    assert_eq!(sent.len(), 1);
    let (mail_from, rcpt_to, bytes) = &sent[0];
    assert_eq!(mail_from, "me@example.test");
    assert_eq!(rcpt_to, &vec!["ada@example.test".to_owned()]);
    assert!(
        bytes.contains("Subject: Accepted: Design review"),
        "{bytes}"
    );
    assert!(
        bytes.contains("In-Reply-To: <review-7-mail@example.test>"),
        "{bytes}"
    );
    assert!(bytes.contains("multipart/alternative"), "{bytes}");

    let reply = calendar_of(bytes);
    assert_eq!(reply.method, mail_pim::ical::Method::Reply);
    let event = &reply.events[0];
    assert_eq!(event.uid, "review-7@example.test");
    assert_eq!(event.sequence, 0);
    assert_eq!(event.stamp, Some(at(100)));
    assert_eq!(
        event.attendees.len(),
        1,
        "only the user's own attendee line"
    );
    assert_eq!(event.attendees[0].party.email, "me@example.test");
    assert_eq!(
        event.attendees[0].answer,
        mail_pim::ical::PartStat::Accepted
    );
    assert_eq!(event.comment.as_deref(), Some("Looking forward to it"));
    assert_eq!(
        event.organizer.as_ref().map(|o| o.email.as_str()),
        Some("ada@example.test")
    );
    // An IANA `TZID` with no `VTIMEZONE` in the invitation: nothing to copy, nothing invented.
    assert_eq!(reply.zones.len(), 0);

    assert_eq!(
        store.invite_answer(INVITE).unwrap(),
        Some(InviteAnswer {
            message: INVITE,
            attendance: Attendance::Accepted,
            sequence: 0,
            comment: Some("Looking forward to it".to_owned()),
            answered_at: at(100),
        })
    );
    let out = exercise(&store, &invite(InviteAction::Show)).unwrap();
    assert!(out.contains("you answered: accepted"), "{out}");
}

#[test]
fn answering_again_sends_a_new_reply_and_shows_the_current_answer() {
    let (store, _dir, thread) = seeded(Some(request("me@example.test")));
    exercise(&store, &answer(Attendance::Accepted, None)).unwrap();
    exercise(&store, &answer(Attendance::Declined, None)).unwrap();

    let sent = submissions(&store);
    assert_eq!(sent.len(), 2);
    assert!(sent[1].2.contains("Subject: Declined: Design review"));
    let reply = calendar_of(&sent[1].2);
    assert_eq!(
        reply.events[0].attendees[0].answer,
        mail_pim::ical::PartStat::Declined
    );
    assert_eq!(
        store.invite_answer(INVITE).unwrap().map(|a| a.attendance),
        Some(Attendance::Declined)
    );
    let out = exercise(&store, &cli::Command::Show { thread }).unwrap();
    assert!(out.contains("you answered: declined"), "{out}");
}

#[test]
fn an_invitation_to_an_alias_is_answered_as_that_identity_whatever_its_case() {
    let (store, _dir, _) = seeded(Some(request("Alias@Example.TEST")));
    exercise(&store, &answer(Attendance::Tentative, None)).unwrap();
    let sent = submissions(&store);
    assert_eq!(
        sent[0].0, "alias@example.test",
        "sent as the invited identity"
    );
    assert!(sent[0].2.contains("Subject: Tentative: Design review"));
    let reply = calendar_of(&sent[0].2);
    // Named as the invitation spelled it, which is what the organiser matches.
    assert_eq!(
        reply.events[0].attendees[0].party.email,
        "Alias@Example.TEST"
    );
    assert_eq!(
        reply.events[0].attendees[0].answer,
        mail_pim::ical::PartStat::Tentative
    );
}

#[test]
fn an_invitation_to_an_identitys_reply_to_address_is_the_users_too() {
    let (store, _dir, _) = seeded(Some(request("me.reply@example.test")));
    exercise(&store, &answer(Attendance::Accepted, None)).unwrap();
    let sent = submissions(&store);
    assert_eq!(sent[0].0, "me@example.test");
    let reply = calendar_of(&sent[0].2);
    assert_eq!(
        reply.events[0].attendees[0].party.email,
        "me.reply@example.test"
    );
}

#[test]
fn an_invitation_that_does_not_name_the_user_is_shown_but_not_answered() {
    let (store, _dir, _) = seeded(Some(request("someone.else@example.test")));
    let out = exercise(&store, &invite(InviteAction::Show)).unwrap();
    assert!(!out.contains("accept|tentative|decline"), "{out}");
    let err = exercise(&store, &answer(Attendance::Accepted, None)).unwrap_err();
    assert!(err.contains("attendees"), "{err}");
    assert!(submissions(&store).is_empty());
    assert_eq!(store.invite_answer(INVITE).unwrap(), None);
}

#[test]
fn a_cancellation_is_shown_as_cancelled_and_has_nothing_to_answer() {
    let raw = message_with(
        &calendar("CANCEL", "me@example.test", 1, "CANCELLED"),
        "CANCEL",
    );
    let (store, _dir, thread) = seeded(Some(raw));
    let out = exercise(&store, &invite(InviteAction::Show)).unwrap();
    assert!(out.contains("CANCELLED"), "{out}");
    assert!(!out.contains("accept|tentative|decline"), "{out}");
    let out = exercise(&store, &cli::Command::Show { thread }).unwrap();
    assert!(out.contains("cancelled: Design review"), "{out}");
    let err = exercise(&store, &answer(Attendance::Declined, None)).unwrap_err();
    assert!(err.contains("cancelled"), "{err}");
    assert!(submissions(&store).is_empty());
}

#[test]
fn an_update_says_it_is_one() {
    let raw = message_with(
        &calendar("REQUEST", "me@example.test", 3, "CONFIRMED"),
        "REQUEST",
    );
    let (store, _dir, _) = seeded(Some(raw));
    let out = exercise(&store, &invite(InviteAction::Show)).unwrap();
    assert!(out.contains("an updated invitation (revision 3)"), "{out}");
    exercise(&store, &answer(Attendance::Accepted, None)).unwrap();
    let reply = calendar_of(&submissions(&store)[0].2);
    assert_eq!(reply.events[0].sequence, 3, "the revision answered");
    assert_eq!(store.invite_answer(INVITE).unwrap().unwrap().sequence, 3);
}

#[test]
fn someone_answering_the_users_invitation_is_summarised_as_who_answered_what() {
    let reply = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\n\
                 UID:review-7@example.test\r\nDTSTAMP:20260921T090000Z\r\n\
                 DTSTART:20261005T120000Z\r\nSUMMARY:Design review\r\n\
                 ORGANIZER:mailto:me@example.test\r\n\
                 ATTENDEE;CN=Charles;PARTSTAT=TENTATIVE:mailto:charles@example.test\r\n\
                 COMMENT:Might be late\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let (store, _dir, _) = seeded(Some(message_with(reply, "REPLY")));
    let out = exercise(&store, &invite(InviteAction::Show)).unwrap();
    assert!(out.contains("an answer to an invitation you sent"), "{out}");
    assert!(
        out.contains("Charles <charles@example.test>  tentative"),
        "{out}"
    );
    assert!(out.contains("comment:   Might be late"), "{out}");
    let err = exercise(&store, &answer(Attendance::Accepted, None)).unwrap_err();
    assert!(err.contains("answer to an invitation"), "{err}");
}

#[test]
fn the_ics_export_is_the_invitation_and_reads_back_as_the_same_one() {
    let (store, dir, _) = seeded(Some(request("me@example.test")));
    let path = dir.path().join("review.ics");
    let out = exercise(&store, &invite(InviteAction::Ics(path.clone()))).unwrap();
    assert!(out.contains("review.ics"), "{out}");
    let exported = std::fs::read(&path).unwrap();
    assert_eq!(
        String::from_utf8(exported.clone()).unwrap(),
        calendar("REQUEST", "me@example.test", 0, "CONFIRMED")
    );
    let me = ["me@example.test"];
    let from_file = mail_pim::summarise(
        &mail_pim::ical::parse(std::str::from_utf8(&exported).unwrap()).unwrap(),
        &me,
    );
    let from_message = mail_app::invite::invite_of(&request("me@example.test"), &me);
    assert!(from_file.is_some());
    assert_eq!(from_file, from_message);
}

#[test]
fn headers_alone_cannot_say_whether_a_message_invites() {
    let (store, _dir, _) = seeded(None);
    let err = exercise(&store, &answer(Attendance::Accepted, None)).unwrap_err();
    assert!(err.contains("mailo sync"), "{err}");
    let err = exercise(&store, &invite(InviteAction::Show)).unwrap_err();
    assert!(err.contains("mailo sync"), "{err}");
}

#[test]
fn a_message_with_no_invitation_says_so() {
    let (store, _dir, thread) = seeded(Some(
        b"From: ada@example.test\r\nSubject: hi\r\n\r\nhello\r\n".to_vec(),
    ));
    let err = exercise(&store, &invite(InviteAction::Show)).unwrap_err();
    assert!(err.contains("no calendar invitation"), "{err}");
    let out = exercise(&store, &cli::Command::Show { thread }).unwrap();
    assert!(!out.contains("mailo invite"), "{out}");
}

#[test]
fn the_invite_command_parses() {
    let args = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    let id = INVITE.to_string();
    type Case = (&'static [&'static str], Option<fn() -> InviteAction>);
    const CASES: &[Case] = &[
        (&[], Some(|| InviteAction::Show)),
        (
            &["accept"],
            Some(|| InviteAction::Answer {
                attendance: Attendance::Accepted,
                comment: None,
            }),
        ),
        (
            &["tentative", "--comment", "might be late"],
            Some(|| InviteAction::Answer {
                attendance: Attendance::Tentative,
                comment: Some("might be late".to_owned()),
            }),
        ),
        (
            &["decline"],
            Some(|| InviteAction::Answer {
                attendance: Attendance::Declined,
                comment: None,
            }),
        ),
        (
            &["--ics", "out.ics"],
            Some(|| InviteAction::Ics("out.ics".into())),
        ),
        (&["maybe"], None),
        (&["accept", "--comment"], None),
        (&["accept", "--note", "x"], None),
        (&["accept", "--comment", "x", "y"], None),
        (&["--ics"], None),
        (&["--ics", "a.ics", "b.ics"], None),
    ];
    for (rest, expected) in CASES {
        let mut words = vec!["invite", id.as_str()];
        words.extend_from_slice(rest);
        let parsed = cli::parse(&args(&words));
        match expected {
            Some(action) => assert_eq!(parsed, Ok(invite(action())), "{rest:?}"),
            None => assert!(parsed.is_err(), "{rest:?}: {parsed:?}"),
        }
    }
    assert!(cli::parse(&args(&["invite"])).is_err());
    assert!(cli::parse(&args(&["invite", "not-a-uuid"])).is_err());
}

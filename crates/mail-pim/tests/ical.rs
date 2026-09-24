//! Invitations as calendar programs write them, read, summarised for the reader, and answered.
//!
//! The samples are written for these tests in the shapes RFC 5545 and RFC 5546 describe and in
//! the styles the common senders use — a web calendar's UTC times, a desktop suite's Windows
//! zone names and `VTIMEZONE` blocks, a phone calendar's IANA zones with `EMAIL` parameters. No
//! real invitation was copied; every address is under `example.test`.

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use mail_domain::Attendance;
use mail_pim::ical::{
    self, Answering, End, EventZone, Method, Moment, PartStat, Role, Rsvp, describe_rule,
};
use mail_pim::{Invite, Kind, Me, PimError, Revision, Unplaced, When, show_when, summarise};

const ME: &[&str] = &["me@example.test"];

/// A web calendar's invitation: UTC times, a long folded description, guests with `CUTYPE`
/// and vendor parameters.
const WEB: &str = "BEGIN:VCALENDAR\r\n\
PRODID:-//Example Web Calendar//Calendar 1.0//EN\r\n\
VERSION:2.0\r\n\
CALSCALE:GREGORIAN\r\n\
METHOD:REQUEST\r\n\
BEGIN:VEVENT\r\n\
DTSTART:20261002T130000Z\r\n\
DTEND:20261002T140000Z\r\n\
DTSTAMP:20260920T101500Z\r\n\
ORGANIZER;CN=Ada Lovelace:mailto:ada@example.test\r\n\
UID:0a1b2c3d4e5f@example.test\r\n\
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;RSVP=TRUE\r\n\
\x20;CN=Ada Lovelace;X-NUM-GUESTS=0:mailto:ada@example.test\r\n\
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=\r\n\
\x20TRUE;CN=me@example.test;X-NUM-GUESTS=0:mailto:me@example.test\r\n\
ATTENDEE;CUTYPE=INDIVIDUAL;ROLE=OPT-PARTICIPANT;PARTSTAT=TENTATIVE;RSVP=TRUE\r\n\
\x20;CN=Charles Babbage;X-NUM-GUESTS=0:mailto:charles@example.test\r\n\
CREATED:20260920T101400Z\r\n\
DESCRIPTION:Agenda:\\n1. Figures\\n2. The engine\\, again\\n\\nJoin by phone: +\r\n\
\x201 555 0100\r\n\
LAST-MODIFIED:20260920T101500Z\r\n\
LOCATION:Room 4\\, second floor\r\n\
SEQUENCE:0\r\n\
STATUS:CONFIRMED\r\n\
SUMMARY:Engine review\r\n\
TRANSP:OPAQUE\r\n\
BEGIN:VALARM\r\n\
ACTION:DISPLAY\r\n\
DESCRIPTION:This is an event reminder\r\n\
TRIGGER:-P0DT0H10M0S\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

/// A desktop suite's invitation: a Windows zone name in `TZID`, its own `VTIMEZONE` since 1601,
/// `MAILTO:` in capitals, and a pile of vendor properties.
const DESKTOP: &str = "BEGIN:VCALENDAR\r\n\
METHOD:REQUEST\r\n\
PRODID:Example Desktop Suite 16.0\r\n\
VERSION:2.0\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Taipei Standard Time\r\n\
BEGIN:STANDARD\r\n\
DTSTART:16010101T000000\r\n\
TZOFFSETFROM:+0800\r\n\
TZOFFSETTO:+0800\r\n\
END:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
ORGANIZER;CN=\"Hopper, Grace\":MAILTO:grace@example.test\r\n\
ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Me Myself:MAIL\r\n\
\x20TO:Me@Example.TEST\r\n\
ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Ada Lovelace\r\n\
\x20:MAILTO:ada@example.test\r\n\
DESCRIPTION;LANGUAGE=en-US:Quarterly planning.\\n\r\n\
UID:040000008200E00074C5B7101A82E00800000000A0B1C2D3E4F5A6B7C8D9E0F1A2B3C4D\r\n\
\x205E6F\r\n\
SUMMARY;LANGUAGE=en-US:Planning\r\n\
DTSTART;TZID=Taipei Standard Time:20261001T150000\r\n\
DTEND;TZID=Taipei Standard Time:20261001T160000\r\n\
CLASS:PUBLIC\r\n\
PRIORITY:5\r\n\
DTSTAMP:20260921T020000Z\r\n\
TRANSP:OPAQUE\r\n\
STATUS:CONFIRMED\r\n\
SEQUENCE:0\r\n\
LOCATION;LANGUAGE=en-US:Board room\r\n\
X-MICROSOFT-CDO-APPT-SEQUENCE:0\r\n\
X-MICROSOFT-CDO-BUSYSTATUS:TENTATIVE\r\n\
X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY\r\n\
X-MICROSOFT-CDO-ALLDAYEVENT:FALSE\r\n\
BEGIN:VALARM\r\n\
DESCRIPTION:REMINDER\r\n\
TRIGGER;RELATED=START:-PT15M\r\n\
ACTION:DISPLAY\r\n\
END:VALARM\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

/// A desktop suite's invitation in a zone it invented a name for, readable only by its rules:
/// US Pacific time, as a `VTIMEZONE` with yearly onsets.
const CUSTOM_ZONE: &str = "BEGIN:VCALENDAR\r\n\
METHOD:REQUEST\r\n\
VERSION:2.0\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Customized Time Zone\r\n\
BEGIN:STANDARD\r\n\
DTSTART:16010101T020000\r\n\
TZOFFSETFROM:-0700\r\n\
TZOFFSETTO:-0800\r\n\
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=1SU;BYMONTH=11\r\n\
END:STANDARD\r\n\
BEGIN:DAYLIGHT\r\n\
DTSTART:16010101T020000\r\n\
TZOFFSETFROM:-0800\r\n\
TZOFFSETTO:-0700\r\n\
RRULE:FREQ=YEARLY;INTERVAL=1;BYDAY=2SU;BYMONTH=3\r\n\
END:DAYLIGHT\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
ORGANIZER:mailto:grace@example.test\r\n\
ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.test\r\n\
UID:custom-zone-1\r\n\
SUMMARY:Launch\r\n\
DTSTART;TZID=\"Customized Time Zone\":20260710T090000\r\n\
DURATION:PT1H30M\r\n\
DTSTAMP:20260601T000000Z\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

/// A phone calendar's invitation: an IANA zone with its `VTIMEZONE`, the organiser known by a
/// `urn:uuid:` with an `EMAIL` parameter, and a weekly rule.
const PHONE: &str = "BEGIN:VCALENDAR\r\n\
VERSION:2.0\r\n\
PRODID:-//Example Phone//Calendar 18.0//EN\r\n\
METHOD:REQUEST\r\n\
CALSCALE:GREGORIAN\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Europe/Berlin\r\n\
BEGIN:DAYLIGHT\r\n\
TZOFFSETFROM:+0100\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\n\
DTSTART:19810329T020000\r\n\
TZNAME:CEST\r\n\
TZOFFSETTO:+0200\r\n\
END:DAYLIGHT\r\n\
BEGIN:STANDARD\r\n\
TZOFFSETFROM:+0200\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\n\
DTSTART:19961027T030000\r\n\
TZNAME:CET\r\n\
TZOFFSETTO:+0100\r\n\
END:STANDARD\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
CREATED:20260901T080000Z\r\n\
UID:5F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0\r\n\
DTEND;TZID=Europe/Berlin:20261105T110000\r\n\
RRULE:FREQ=WEEKLY;BYDAY=MO,WE;COUNT=10\r\n\
TRANSP:OPAQUE\r\n\
SUMMARY:Stand-up\r\n\
DTSTART;TZID=Europe/Berlin:20261105T100000\r\n\
DTSTAMP:20260901T080100Z\r\n\
SEQUENCE:0\r\n\
ORGANIZER;CN=Charles Babbage;EMAIL=charles@example.test:urn:uuid:11111111-2\r\n\
\x20222-3333-4444-555555555555\r\n\
ATTENDEE;CN=Charles Babbage;CUTYPE=INDIVIDUAL;EMAIL=charles@example.test;PAR\r\n\
\x20TSTAT=ACCEPTED;ROLE=CHAIR:urn:uuid:11111111-2222-3333-4444-555555555555\r\n\
ATTENDEE;CN=Me;CUTYPE=INDIVIDUAL;EMAIL=me@example.test;PARTSTAT=NEEDS-ACTIO\r\n\
\x20N;ROLE=REQ-PARTICIPANT;RSVP=TRUE:mailto:me@example.test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn read_invite(text: &str) -> Invite {
    summarise(&ical::parse(text).unwrap(), ME).unwrap()
}

fn tz(name: &str) -> Tz {
    name.parse().unwrap()
}

#[test]
fn a_web_calendar_invitation_reads_in_utc_with_its_guests_and_their_answers() {
    let calendar = ical::parse(WEB).unwrap();
    assert_eq!(calendar.method, Method::Request);
    let event = &calendar.events[0];
    assert_eq!(event.uid, "0a1b2c3d4e5f@example.test");
    assert_eq!(event.stamp, Some(at("2026-09-20T10:15:00Z")));
    assert_eq!(
        event.description.as_deref(),
        Some("Agenda:\n1. Figures\n2. The engine, again\n\nJoin by phone: +1 555 0100")
    );
    // The alarm's own `DESCRIPTION` is the alarm's, not the event's.
    assert!(!event.description.as_deref().unwrap().contains("reminder"));

    let invite = read_invite(WEB);
    assert_eq!(invite.kind, Kind::Request(Revision::First));
    assert_eq!(invite.title.as_deref(), Some("Engine review"));
    assert_eq!(invite.location.as_deref(), Some("Room 4, second floor"));
    assert_eq!(
        invite.when,
        When::Timed {
            start: at("2026-10-02T13:00:00Z"),
            end: Some(at("2026-10-02T14:00:00Z")),
            zone: EventZone::Utc,
        }
    );
    let organiser = invite.organiser.as_ref().unwrap();
    assert_eq!(organiser.email, "ada@example.test");
    assert_eq!(organiser.name.as_deref(), Some("Ada Lovelace"));
    let answers: Vec<(&str, PartStat, Role, Rsvp)> = invite
        .attendees
        .iter()
        .map(|a| (a.party.email.as_str(), a.answer, a.role, a.rsvp))
        .collect();
    assert_eq!(
        answers,
        [
            (
                "ada@example.test",
                PartStat::Accepted,
                Role::Required,
                Rsvp::Requested
            ),
            (
                "me@example.test",
                PartStat::NeedsAction,
                Role::Required,
                Rsvp::Requested
            ),
            (
                "charles@example.test",
                PartStat::Tentative,
                Role::Optional,
                Rsvp::Requested
            ),
        ]
    );
    assert_eq!(
        invite.me,
        Me::Invited {
            address: "me@example.test".into(),
            answer: PartStat::NeedsAction
        }
    );
    // Written in UTC: nothing says where the organiser is, so there is no second reading.
    let shown = show_when(&invite.when, &tz("Asia/Taipei"));
    assert_eq!(shown.yours, "Fri 2 Oct 2026, 21:00–22:00");
    assert_eq!(shown.theirs, None);
}

#[test]
fn a_windows_zone_name_is_read_as_its_iana_zone_and_shown_in_both() {
    let invite = read_invite(DESKTOP);
    assert_eq!(
        invite.uid,
        "040000008200E00074C5B7101A82E00800000000A0B1C2D3E4F5A6B7C8D9E0F1A2B3C4D5E6F"
    );
    assert_eq!(
        invite.when,
        When::Timed {
            start: at("2026-10-01T07:00:00Z"),
            end: Some(at("2026-10-01T08:00:00Z")),
            zone: EventZone::Iana(tz("Asia/Taipei")),
        }
    );
    // `CN` quoted because it holds a comma.
    assert_eq!(
        invite.organiser.as_ref().unwrap().name.as_deref(),
        Some("Hopper, Grace")
    );
    // Matched whatever the case the invitation wrote the address in, and kept as it wrote it.
    assert_eq!(
        invite.me,
        Me::Invited {
            address: "Me@Example.TEST".into(),
            answer: PartStat::NeedsAction
        }
    );

    let shown = show_when(&invite.when, &tz("Europe/London"));
    assert_eq!(shown.yours, "Thu 1 Oct 2026, 08:00–09:00");
    assert_eq!(
        shown.theirs.as_deref(),
        Some("Thu 1 Oct 2026, 15:00–16:00 (Asia/Taipei)")
    );
    // A reader in the organiser's zone is told once.
    let shown = show_when(&invite.when, &tz("Asia/Taipei"));
    assert_eq!(shown.yours, "Thu 1 Oct 2026, 15:00–16:00");
    assert_eq!(shown.theirs, None);
}

#[test]
fn a_zone_known_only_by_its_rules_is_evaluated_from_them() {
    let invite = read_invite(CUSTOM_ZONE);
    let When::Timed { start, end, zone } = &invite.when else {
        panic!("{:?}", invite.when);
    };
    // 10 July is in daylight time: UTC-7.
    assert_eq!(*start, at("2026-07-10T16:00:00Z"));
    assert_eq!(*end, Some(at("2026-07-10T17:30:00Z")));
    assert!(
        matches!(zone, EventZone::Rules { tzid, offset }
            if tzid == "Customized Time Zone" && offset.local_minus_utc() == -7 * 3600),
        "{zone:?}"
    );
    let shown = show_when(&invite.when, &Utc);
    assert_eq!(shown.yours, "Fri 10 Jul 2026, 16:00–17:30");
    assert_eq!(
        shown.theirs.as_deref(),
        Some("Fri 10 Jul 2026, 09:00–10:30 (Customized Time Zone)")
    );
}

#[test]
fn a_phone_invitation_finds_addresses_in_email_parameters_and_says_its_rule() {
    let invite = read_invite(PHONE);
    assert_eq!(
        invite.when,
        When::Timed {
            start: at("2026-11-05T09:00:00Z"),
            end: Some(at("2026-11-05T10:00:00Z")),
            zone: EventZone::Iana(tz("Europe/Berlin")),
        }
    );
    let organiser = invite.organiser.as_ref().unwrap();
    assert_eq!(organiser.email, "charles@example.test");
    assert_eq!(invite.attendees[0].role, Role::Chair);
    assert_eq!(
        invite.repeats.as_deref(),
        Some("every week on Monday and Wednesday, 10 times")
    );
}

#[test]
fn an_all_day_event_covers_its_days_and_is_in_no_zone() {
    let text = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:offsite\r\n\
                SUMMARY:Offsite\r\nDTSTART;VALUE=DATE:20261001\r\nDTEND;VALUE=DATE:20261003\r\n\
                ORGANIZER:mailto:ada@example.test\r\n\
                ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.test\r\n\
                END:VEVENT\r\nEND:VCALENDAR\r\n";
    let invite = read_invite(text);
    assert_eq!(
        invite.when,
        When::AllDay {
            first: day(2026, 10, 1),
            last: day(2026, 10, 2)
        }
    );
    let shown = show_when(&invite.when, &tz("Pacific/Auckland"));
    assert_eq!(shown.yours, "Thu 1 Oct 2026 – Fri 2 Oct 2026, all day");
    assert_eq!(shown.theirs, None);

    // One day, by duration, and with no end at all.
    for end in ["DURATION:P1D\r\n", ""] {
        let text = format!(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART;VALUE=DATE:20261001\r\n{end}\
             END:VEVENT\r\nEND:VCALENDAR\r\n"
        );
        let invite = read_invite(&text);
        assert_eq!(
            invite.when,
            When::AllDay {
                first: day(2026, 10, 1),
                last: day(2026, 10, 1)
            },
            "{end:?}"
        );
        assert_eq!(invite.kind, Kind::Published, "no METHOD: a file to add");
    }
}

#[test]
fn a_higher_sequence_is_an_update_and_a_cancel_is_a_cancellation() {
    let update = WEB.replace("SEQUENCE:0", "SEQUENCE:2");
    assert_eq!(
        read_invite(&update).kind,
        Kind::Request(Revision::Update { sequence: 2 })
    );
    assert_eq!(read_invite(&update).sequence, 2);

    let cancel = WEB
        .replace("METHOD:REQUEST", "METHOD:CANCEL")
        .replace("STATUS:CONFIRMED", "STATUS:CANCELLED")
        .replace("SEQUENCE:0", "SEQUENCE:1");
    assert_eq!(read_invite(&cancel).kind, Kind::Cancelled);
    // A request whose event says it is cancelled is a cancellation too.
    let cancelled = WEB.replace("STATUS:CONFIRMED", "STATUS:CANCELLED");
    assert_eq!(read_invite(&cancelled).kind, Kind::Cancelled);
    // Methods that are not invitations to show.
    for method in ["REFRESH", "COUNTER", "DECLINECOUNTER"] {
        let text = WEB.replace("METHOD:REQUEST", &format!("METHOD:{method}"));
        assert_eq!(
            summarise(&ical::parse(&text).unwrap(), ME),
            None,
            "{method}"
        );
    }
}

/// An organiser moving one occurrence of a series: the request carries only that occurrence.
const ONE_OCCURRENCE: &str = "BEGIN:VCALENDAR\r\n\
METHOD:REQUEST\r\n\
BEGIN:VTIMEZONE\r\n\
TZID:Europe/Berlin\r\n\
BEGIN:STANDARD\r\n\
DTSTART:19961027T030000\r\n\
TZOFFSETFROM:+0200\r\n\
TZOFFSETTO:+0100\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\n\
END:STANDARD\r\n\
BEGIN:DAYLIGHT\r\n\
DTSTART:19810329T020000\r\n\
TZOFFSETFROM:+0100\r\n\
TZOFFSETTO:+0200\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\n\
END:DAYLIGHT\r\n\
END:VTIMEZONE\r\n\
BEGIN:VEVENT\r\n\
UID:5F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0\r\n\
RECURRENCE-ID;TZID=Europe/Berlin:20261109T100000\r\n\
SEQUENCE:1\r\n\
DTSTAMP:20261101T120000Z\r\n\
DTSTART;TZID=Europe/Berlin:20261109T140000\r\n\
DTEND;TZID=Europe/Berlin:20261109T143000\r\n\
SUMMARY:Stand-up (moved)\r\n\
ORGANIZER;CN=Charles Babbage:mailto:charles@example.test\r\n\
ATTENDEE;CN=Charles Babbage;PARTSTAT=ACCEPTED;ROLE=CHAIR:mailto:charles@example.test\r\n\
ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.test\r\n\
ATTENDEE;CN=Ada;PARTSTAT=ACCEPTED:mailto:ada@example.test\r\n\
END:VEVENT\r\n\
END:VCALENDAR\r\n";

#[test]
fn a_single_occurrence_says_which_one_it_is() {
    let invite = read_invite(ONE_OCCURRENCE);
    assert_eq!(
        invite.recurrence_id,
        Some(Moment::Zoned {
            local: day(2026, 11, 9).and_hms_opt(10, 0, 0).unwrap(),
            tzid: "Europe/Berlin".into()
        })
    );
    assert_eq!(invite.kind, Kind::Request(Revision::Update { sequence: 1 }));
    let shown = show_when(&invite.when, &tz("Europe/Berlin"));
    assert_eq!(shown.yours, "Mon 9 Nov 2026, 14:00–14:30");
}

#[test]
fn the_series_is_the_event_a_message_is_about_when_it_carries_exceptions_too() {
    // The master event after one of its exceptions, as some servers order them.
    let text = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\n\
                BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20261109T090000Z\r\nSUMMARY:Moved\r\n\
                DTSTART:20261109T130000Z\r\nEND:VEVENT\r\n\
                BEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Series\r\nDTSTART:20261102T090000Z\r\n\
                RRULE:FREQ=DAILY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let invite = read_invite(text);
    assert_eq!(invite.title.as_deref(), Some("Series"));
    assert_eq!(invite.recurrence_id, None);
    assert_eq!(invite.repeats.as_deref(), Some("every day"));
}

#[test]
fn identities_match_case_insensitively_and_through_any_of_the_users_addresses() {
    let calendar = ical::parse(WEB).unwrap();
    const CASES: &[(&[&str], Option<&str>)] = &[
        (&["ME@EXAMPLE.TEST"], Some("me@example.test")),
        (
            &["someone@else.test", " me@example.test "],
            Some("me@example.test"),
        ),
        (&["charles@example.test"], Some("charles@example.test")),
        (&["nobody@example.test"], None),
        (&[], None),
    ];
    for (me, expected) in CASES {
        let invite = summarise(&calendar, me).unwrap();
        let found = match &invite.me {
            Me::Invited { address, .. } => Some(address.as_str()),
            _ => None,
        };
        assert_eq!(found, *expected, "{me:?}");
    }
    // The organiser is the organiser, even though they list themselves as an attendee.
    assert_eq!(
        summarise(&calendar, &["Ada@Example.Test"]).unwrap().me,
        Me::Organiser
    );
}

fn answering<'a>(
    attendee: &'a str,
    attendance: Attendance,
    comment: Option<&'static str>,
) -> Answering<'a> {
    Answering {
        attendee,
        attendance,
        comment,
        at: at("2026-09-24T08:30:00Z"),
        product: "-//mailo//mailo test//EN",
    }
}

#[test]
fn a_reply_names_only_the_answering_attendee_and_echoes_what_the_organiser_matches_by() {
    let calendar = ical::parse(DESKTOP).unwrap();
    let event = calendar.main_event().unwrap();
    let text = ical::reply(
        &calendar,
        event,
        &answering(
            "Me@Example.TEST",
            Attendance::Accepted,
            Some("See you, then"),
        ),
    )
    .unwrap();
    assert_eq!(
        text,
        "BEGIN:VCALENDAR\r\n\
         PRODID:-//mailo//mailo test//EN\r\n\
         VERSION:2.0\r\n\
         CALSCALE:GREGORIAN\r\n\
         METHOD:REPLY\r\n\
         BEGIN:VTIMEZONE\r\n\
         TZID:Taipei Standard Time\r\n\
         BEGIN:STANDARD\r\n\
         DTSTART:16010101T000000\r\n\
         TZOFFSETFROM:+0800\r\n\
         TZOFFSETTO:+0800\r\n\
         END:STANDARD\r\n\
         END:VTIMEZONE\r\n\
         BEGIN:VEVENT\r\n\
         UID:040000008200E00074C5B7101A82E00800000000A0B1C2D3E4F5A6B7C8D9E0F1A2B3C4D\r\n \
         5E6F\r\n\
         SEQUENCE:0\r\n\
         DTSTAMP:20260924T083000Z\r\n\
         DTSTART;TZID=Taipei Standard Time:20261001T150000\r\n\
         DTEND;TZID=Taipei Standard Time:20261001T160000\r\n\
         SUMMARY;LANGUAGE=en-US:Planning\r\n\
         ORGANIZER;CN=\"Hopper, Grace\":MAILTO:grace@example.test\r\n\
         ATTENDEE;ROLE=REQ-PARTICIPANT;CN=Me Myself;PARTSTAT=ACCEPTED:MAILTO:Me@Exam\r\n \
         ple.TEST\r\n\
         COMMENT:See you\\, then\r\n\
         END:VEVENT\r\n\
         END:VCALENDAR\r\n"
    );

    // Read back: a reply, one attendee, the answer, the same event.
    let back = ical::parse(&text).unwrap();
    assert_eq!(back.method, Method::Reply);
    let answer = &back.events[0];
    assert_eq!(answer.uid, event.uid);
    assert_eq!(answer.sequence, 0);
    assert_eq!(answer.stamp, Some(at("2026-09-24T08:30:00Z")));
    assert_eq!(answer.organizer, event.organizer);
    assert_eq!(answer.attendees.len(), 1);
    assert_eq!(answer.attendees[0].party.email, "Me@Example.TEST");
    assert_eq!(answer.attendees[0].answer, PartStat::Accepted);
    assert_eq!(answer.attendees[0].rsvp, Rsvp::NotRequested);
    assert_eq!(answer.comment.as_deref(), Some("See you, then"));

    // And summarised, as the organiser's own mail client would show it.
    let seen = summarise(&back, &["grace@example.test"]).unwrap();
    assert_eq!(seen.kind, Kind::Reply);
    assert_eq!(seen.me, Me::Organiser);
    assert_eq!(seen.attendees[0].answer, PartStat::Accepted);
    assert_eq!(seen.comment.as_deref(), Some("See you, then"));
}

#[test]
fn a_reply_to_one_occurrence_carries_its_recurrence_id_and_zone() {
    let calendar = ical::parse(ONE_OCCURRENCE).unwrap();
    let event = calendar.main_event().unwrap();
    let text = ical::reply(
        &calendar,
        event,
        &answering("me@example.test", Attendance::Declined, None),
    )
    .unwrap();
    assert!(
        text.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261109T100000\r\n"),
        "{text}"
    );
    assert!(text.contains("SEQUENCE:1\r\n"), "{text}");
    assert_eq!(text.matches("BEGIN:VTIMEZONE").count(), 1, "{text}");
    assert_eq!(text.matches("ATTENDEE").count(), 1, "{text}");
    assert!(
        text.contains("PARTSTAT=DECLINED:mailto:me@example.test"),
        "{text}"
    );
    assert!(!text.contains("COMMENT"), "{text}");
    let back = ical::parse(&text).unwrap();
    assert_eq!(back.events[0].recurrence_id, event.recurrence_id);
    // Its zone came along, so the organiser's calendar can read the time.
    assert_eq!(back.zones.len(), 1);
}

#[test]
fn a_tentative_answer_in_a_utc_invitation_carries_no_zone() {
    let calendar = ical::parse(WEB).unwrap();
    let event = calendar.main_event().unwrap();
    let text = ical::reply(
        &calendar,
        event,
        &answering("me@example.test", Attendance::Tentative, Some("  ")),
    )
    .unwrap();
    assert!(!text.contains("VTIMEZONE"), "{text}");
    assert!(!text.contains("COMMENT"), "a blank comment is none: {text}");
    // Not the other guests, and not their answers.
    assert!(!text.contains("charles@example.test"), "{text}");
    assert!(!text.contains("X-NUM-GUESTS=0:mailto:ada"), "{text}");
    let back = ical::parse(&text).unwrap();
    assert_eq!(back.events[0].attendees.len(), 1);
    assert_eq!(back.events[0].attendees[0].answer, PartStat::Tentative);
}

#[test]
fn a_reply_needs_an_organiser_and_an_attendee_it_lists() {
    let calendar = ical::parse(WEB).unwrap();
    let event = calendar.main_event().unwrap();
    assert!(matches!(
        ical::reply(
            &calendar,
            event,
            &answering("stranger@example.test", Attendance::Accepted, None)
        ),
        Err(PimError::Unanswerable(_))
    ));
    let orphan =
        ical::parse(&WEB.replace("ORGANIZER;CN=Ada Lovelace:mailto:ada@example.test\r\n", ""))
            .unwrap();
    assert!(matches!(
        ical::reply(
            &orphan,
            orphan.main_event().unwrap(),
            &answering("me@example.test", Attendance::Accepted, None)
        ),
        Err(PimError::Unanswerable(_))
    ));
}

#[test]
fn a_floating_time_and_an_unknown_zone_are_said_to_be_clock_readings() {
    let floating = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:f\r\nDTSTART:20261001T090000\r\n\
                    DTEND:20261001T100000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let invite = read_invite(floating);
    assert!(
        matches!(
            &invite.when,
            When::Floating {
                why: Unplaced::AsWritten,
                end: Some(_),
                ..
            }
        ),
        "{:?}",
        invite.when
    );
    assert_eq!(
        show_when(&invite.when, &Utc).yours,
        "Thu 1 Oct 2026, 09:00–10:00 (local time, wherever you are)"
    );

    let bogus = floating.replace("DTSTART:", "DTSTART;TZID=Nowhere/Bogus Standard Time:");
    let invite = read_invite(&bogus);
    assert!(
        matches!(&invite.when, When::Floating { why: Unplaced::UnknownZone(z), .. }
            if z == "Nowhere/Bogus Standard Time"),
        "{:?}",
        invite.when
    );
    assert!(
        show_when(&invite.when, &Utc)
            .yours
            .contains("which could not be identified")
    );
}

#[test]
fn common_rules_are_said_in_words() {
    assert_eq!(describe_rule("FREQ=DAILY;COUNT=5"), "every day, 5 times");
    assert_eq!(
        describe_rule("FREQ=WEEKLY;INTERVAL=2;BYDAY=TU;UNTIL=20261215"),
        "every 2 weeks on Tuesday, until Tue 15 Dec 2026"
    );
    assert_eq!(
        describe_rule("FREQ=MONTHLY;BYDAY=1FR"),
        "repeats (details in the invitation)"
    );
}

#[test]
fn a_duration_end_and_a_dtend_end_are_both_read() {
    let calendar = ical::parse(CUSTOM_ZONE).unwrap();
    assert!(matches!(
        calendar.events[0].end,
        Some(End::Lasts(d)) if d.num_minutes() == 90
    ));
}

// ---------------------------------------------------------------------------------------
// Hostile calendars: nothing panics, and the work is bounded by the size cap.
// ---------------------------------------------------------------------------------------

#[test]
fn a_huge_folded_line_is_read_whole_and_within_bounds() {
    let summary = "x".repeat(200_000);
    let folded = mail_pim::line::write(&mail_pim::ContentLine::new("SUMMARY", summary.clone()));
    let text = format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:big\r\n{folded}END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let invite = read_invite(&text);
    assert_eq!(invite.title.as_deref(), Some(summary.as_str()));
}

#[test]
fn a_calendar_cut_off_before_its_ends_keeps_what_it_had() {
    let cut = &WEB[..WEB.find("CREATED:").unwrap()];
    let invite = read_invite(cut);
    assert_eq!(invite.uid, "0a1b2c3d4e5f@example.test");
    assert_eq!(invite.attendees.len(), 3);
    // A zone cut off mid-way still copies into an answer as a whole component.
    let cut = &DESKTOP[..DESKTOP.find("END:STANDARD").unwrap()];
    let calendar = ical::parse(cut).unwrap();
    assert!(calendar.events.is_empty());
    assert_eq!(calendar.zones.len(), 1);
    assert_eq!(calendar.zones[0].lines.last().unwrap().name, "END");
}

#[test]
fn ten_thousand_attendees_are_read_and_the_reader_is_found_among_them() {
    let mut text = String::from(
        "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:crowd\r\n\
         ORGANIZER:mailto:ada@example.test\r\nDTSTART:20261001T090000Z\r\n",
    );
    for n in 0..10_000 {
        text.push_str(&format!(
            "ATTENDEE;CN=Guest {n};PARTSTAT=NEEDS-ACTION:mailto:g{n}@example.test\r\n"
        ));
    }
    text.push_str("ATTENDEE;PARTSTAT=ACCEPTED:mailto:me@example.test\r\n");
    text.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    assert!(text.len() < ical::MAX_CALENDAR_BYTES);
    let calendar = ical::parse(&text).unwrap();
    let invite = summarise(&calendar, ME).unwrap();
    assert_eq!(invite.attendees.len(), 10_001);
    assert!(matches!(
        invite.me,
        Me::Invited {
            answer: PartStat::Accepted,
            ..
        }
    ));
    let reply = ical::reply(
        &calendar,
        calendar.main_event().unwrap(),
        &answering("me@example.test", Attendance::Declined, None),
    )
    .unwrap();
    assert_eq!(reply.matches("ATTENDEE").count(), 1);
}

#[test]
fn a_calendar_past_the_size_cap_is_refused_rather_than_read() {
    let text = format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        "y".repeat(ical::MAX_CALENDAR_BYTES)
    );
    assert!(matches!(ical::parse(&text), Err(PimError::TooLarge { .. })));
}

#[test]
fn nonsense_nesting_and_garbage_do_not_panic() {
    let deep = "BEGIN:X\r\n".repeat(50_000) + "BEGIN:VEVENT\r\nUID:deep\r\nEND:VEVENT\r\n";
    let calendar = ical::parse(&format!("BEGIN:VCALENDAR\r\n{deep}")).unwrap();
    // Buried under components it does not know, the event is not the calendar's.
    assert!(calendar.events.is_empty());

    for text in [
        "",
        "\u{0}\u{1}\u{2}",
        "BEGIN:VEVENT",
        "END:VCALENDAR\r\nEND:VEVENT\r\n",
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART;TZID=\"\":99999999T999999\r\n\
         DURATION:P99999999999W\r\nRRULE:FREQ=WEEKLY;BYDAY=XX\r\nUID:u\r\n\
         ATTENDEE;PARTSTAT=:mailto:\r\nORGANIZER:mailto:no-at-sign\r\nSEQUENCE:-4\r\n",
        "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:Loop\r\nBEGIN:STANDARD\r\n\
         DTSTART:16010101T000000\r\nTZOFFSETFROM:+9999\r\nTZOFFSETTO:+2500\r\n\
         RRULE:FREQ=YEARLY;BYMONTH=13;BYDAY=9SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
         BEGIN:VEVENT\r\nUID:l\r\nDTSTART;TZID=Loop:20261001T090000\r\nEND:VEVENT\r\n",
    ] {
        if let Ok(calendar) = ical::parse(text)
            && let Some(invite) = summarise(&calendar, ME)
        {
            let _ = show_when(&invite.when, &Utc);
        }
    }
}

#[test]
fn an_invitation_read_back_from_its_own_bytes_is_the_same_invitation() {
    // Export is the calendar object as it came; reading the exported bytes gives the same
    // invitation, which is what a calendar importing the file will see.
    for text in [WEB, DESKTOP, CUSTOM_ZONE, PHONE, ONE_OCCURRENCE] {
        let exported = text.as_bytes().to_vec();
        let again = summarise(
            &ical::parse(std::str::from_utf8(&exported).unwrap()).unwrap(),
            ME,
        );
        assert_eq!(again, Some(read_invite(text)));
    }
    // And a calendar written out property by property reads back the same.
    let calendar = ical::parse(PHONE).unwrap();
    let mut written = String::from("BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\n");
    for zone in &calendar.zones {
        for line in &zone.lines {
            written.push_str(&mail_pim::line::write(line));
        }
    }
    written.push_str("BEGIN:VEVENT\r\n");
    for line in &calendar.events[0].lines {
        written.push_str(&mail_pim::line::write(line));
    }
    written.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
    assert_eq!(ical::parse(&written).unwrap(), calendar);
}

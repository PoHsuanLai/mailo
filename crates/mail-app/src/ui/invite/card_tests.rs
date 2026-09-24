//! The card, drawn for every kind of invitation and every place the reader can stand in one.

use super::draw::InviteCard;
use super::tests::{ME, ics, request};
use super::{Card, card_of};
use chrono::{FixedOffset, TimeZone, Utc};
use dioxus::prelude::*;
use dioxus_core::VirtualDom;
use mail_domain::*;

/// The card alone, as the reader would draw it.
#[component]
fn Alone(card: Card) -> Element {
    let known = use_signal(|| None);
    rsx! { InviteCard { card, known } }
}

/// Nine hours east of UTC: never Berlin's offset, so the organiser's time always differs.
fn east() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).unwrap()
}

fn card(calendar: &str, answered: Option<(Attendance, u32)>) -> Card {
    let parsed = mail_pim::ical::parse(calendar).unwrap();
    let invite = mail_pim::summarise(&parsed, &[ME]).unwrap();
    let message = MessageId::generate();
    let answered = answered.map(|(attendance, sequence)| InviteAnswer {
        message,
        attendance,
        sequence,
        comment: Some("See you there".to_owned()),
        answered_at: Utc.timestamp_opt(1_790_000_000, 0).unwrap(),
    });
    card_of(message, &invite, answered.as_ref(), &east())
}

fn drawn(card: Card) -> String {
    let mut dom = VirtualDom::new_with_props(Alone, AloneProps { card });
    dom.rebuild_in_place();
    dioxus_ssr::render(&dom).replace("&#39;", "'")
}

const BUTTONS: &[&str] = &[
    "aria-label=\"Accept\"",
    "aria-label=\"Maybe\"",
    "aria-label=\"Decline\"",
];

#[test]
fn every_kind_and_every_place_draws_what_it_should() {
    let update = request(2);
    let cancelled = ics(
        "CANCEL",
        1,
        "CANCELLED",
        "ada@example.test",
        &[("Me", ME, "NEEDS-ACTION")],
        "",
    );
    let reply = ics(
        "REPLY",
        0,
        "CONFIRMED",
        ME,
        &[("Charles", "charles@example.test", "ACCEPTED")],
        "COMMENT:Happy to\r\n",
    );
    let published = ics("PUBLISH", 0, "CONFIRMED", "ada@example.test", &[], "");
    let organised = ics(
        "REQUEST",
        0,
        "CONFIRMED",
        ME,
        &[("Ada Lovelace", "ada@example.test", "NEEDS-ACTION")],
        "",
    );
    let elsewhere = ics(
        "REQUEST",
        0,
        "CONFIRMED",
        "ada@example.test",
        &[("Someone", "someone@example.test", "NEEDS-ACTION")],
        "",
    );
    let first = request(0);
    type Case<'a> = (
        &'a str,
        &'a str,
        Option<(Attendance, u32)>,
        bool,
        &'a [&'a str],
    );
    let cases: &[Case] = &[
        (
            "first request",
            &first,
            None,
            true,
            &[
                ">Invitation</span>",
                "Design review",
                "their time: Mon 5 Oct 2026, 14:00–15:00 (Europe/Berlin)",
                "Room 2",
                "Ada Lovelace",
                "class=\"att no\"",
            ],
        ),
        (
            "first request, answered",
            &first,
            Some((Attendance::Accepted, 0)),
            false,
            &[
                "<span class=\"said\">You accepted</span>",
                "“See you there”",
                "aria-label=\"Change answer\"",
            ],
        ),
        (
            "update, unanswered",
            &update,
            None,
            true,
            &["class=\"inv-tag updated\">Updated</span>"],
        ),
        (
            "update, answered before it",
            &update,
            Some((Attendance::Declined, 0)),
            true,
            &[">Updated</span>", "You declined an earlier version."],
        ),
        (
            "update, answered",
            &update,
            Some((Attendance::Tentative, 2)),
            false,
            &["<span class=\"said\">You said maybe</span>"],
        ),
        (
            "cancelled",
            &cancelled,
            None,
            false,
            &[
                "class=\"inv-tag cancelled\">Cancelled</span>",
                "This event will not take place.",
            ],
        ),
        (
            "reply",
            &reply,
            None,
            false,
            &[">Answer</span>", "Charles accepted", "“Happy to”"],
        ),
        (
            "published",
            &published,
            None,
            false,
            &[">Event</span>", "it asks for no answer"],
        ),
        (
            "organiser",
            &organised,
            None,
            false,
            &["You organised this event, so there is nothing to answer."],
        ),
        (
            "not listed",
            &elsewhere,
            None,
            false,
            &["None of this account's addresses is among the attendees"],
        ),
    ];
    for (case, calendar, answered, buttons, says) in cases {
        let page = drawn(card(calendar, *answered));
        for said in *says {
            assert!(page.contains(said), "{case}: {said:?} missing from {page}");
        }
        for button in BUTTONS {
            assert_eq!(
                page.contains(button),
                *buttons,
                "{case}: {button} in {page}"
            );
        }
        assert!(page.contains("aria-label=\"Save .ics\""), "{case}");
    }
}

#[test]
fn the_organisers_time_is_said_only_when_it_differs() {
    let parsed = mail_pim::ical::parse(&request(0)).unwrap();
    let invite = mail_pim::summarise(&parsed, &[ME]).unwrap();
    // Berlin in October is two hours east of UTC.
    let berlin = FixedOffset::east_opt(2 * 3600).unwrap();
    let same = card_of(MessageId::generate(), &invite, None, &berlin);
    assert_eq!(same.theirs, None);
    assert!(!drawn(same).contains("their time"));
}

#[test]
fn past_six_attendees_the_rest_fold_into_a_count() {
    let names: Vec<(String, String)> = (0..10)
        .map(|n| (format!("Person {n}"), format!("p{n}@example.test")))
        .collect();
    let people: Vec<(&str, &str, &str)> = names
        .iter()
        .map(|(name, address)| (name.as_str(), address.as_str(), "ACCEPTED"))
        .collect();
    let calendar = ics("REQUEST", 0, "CONFIRMED", "ada@example.test", &people, "");
    let page = drawn(card(&calendar, None));
    assert_eq!(page.matches("class=\"att yes\"").count(), 6, "{page}");
    assert!(page.contains(">+4</button>"), "{page}");
    assert!(
        page.contains("Person 5") && !page.contains("Person 6"),
        "{page}"
    );

    // Seven is one more than six: named, not folded into "+1".
    let seven = ics(
        "REQUEST",
        0,
        "CONFIRMED",
        "ada@example.test",
        &people[..7],
        "",
    );
    let page = drawn(card(&seven, None));
    assert_eq!(page.matches("class=\"att yes\"").count(), 7, "{page}");
    assert!(!page.contains(">+"), "{page}");
}

#[test]
fn a_long_description_is_folded_with_more() {
    let long = "DESCRIPTION:One\\nTwo\\nThree\\nFour\\nFive\r\n";
    let calendar = ics("REQUEST", 0, "CONFIRMED", "ada@example.test", &[], long);
    let page = drawn(card(&calendar, None));
    assert!(page.contains("class=\"inv-desc folded\""), "{page}");
    assert!(page.contains("aria-label=\"More\""), "{page}");
    let short = ics(
        "REQUEST",
        0,
        "CONFIRMED",
        "ada@example.test",
        &[],
        "DESCRIPTION:Bring questions.\r\n",
    );
    let page = drawn(card(&short, None));
    assert!(page.contains("class=\"inv-desc\""), "{page}");
    assert!(!page.contains("aria-label=\"More\""), "{page}");
}

#[tokio::test]
async fn every_class_the_card_draws_is_styled() {
    let update = request(2);
    let cancelled = ics("CANCEL", 1, "CANCELLED", "ada@example.test", &[], "");
    let reply = ics(
        "REPLY",
        0,
        "CONFIRMED",
        ME,
        &[
            ("Charles", "charles@example.test", "ACCEPTED"),
            ("Grace", "grace@example.test", "TENTATIVE"),
        ],
        "COMMENT:Happy to\r\nDESCRIPTION:One\\nTwo\\nThree\\nFour\\nFive\r\n",
    );
    let names: Vec<(String, String)> = (0..9)
        .map(|n| (format!("Person {n}"), format!("p{n}@example.test")))
        .collect();
    let people: Vec<(&str, &str, &str)> = names
        .iter()
        .map(|(name, address)| (name.as_str(), address.as_str(), "NEEDS-ACTION"))
        .collect();
    let crowd = ics("REQUEST", 0, "CONFIRMED", "ada@example.test", &people, "");
    let mut page = String::new();
    for (calendar, answered) in [
        (request(0), None),
        (request(0), Some((Attendance::Accepted, 0))),
        (update, Some((Attendance::Declined, 0))),
        (cancelled, None),
        (reply, None),
        (crowd, None),
    ] {
        page += &drawn(card(&calendar, answered));
    }
    // The note row, which opens only on a press.
    crate::ui::fixtures::dispatching();
    let mut dom = VirtualDom::new_with_props(
        Alone,
        AloneProps {
            card: card(&request(0), None),
        },
    );
    let seen = crate::ui::fixtures::rebuild_into(&mut dom);
    crate::ui::fixtures::click(&mut dom, seen.one("aria-label", "Accept"));
    let noting = dioxus_ssr::render(&dom);
    assert!(noting.contains("inv-noting"), "{noting}");
    page += &noting;
    let missing = crate::ui::style::tests::unstyled_classes(&page, crate::ui::style::STYLE);
    assert!(missing.is_empty(), "unstyled classes: {missing:?}");
}

#[test]
fn your_own_chip_says_what_you_answered_not_what_the_invitation_was_sent_with() {
    let first = request(0);
    let open = card(&first, None);
    let mine = |card: &Card| {
        card.attendees
            .iter()
            .find(|chip| chip.name == "Me")
            .map(|chip| chip.said)
    };
    let before = mine(&open).unwrap_or_else(|| panic!("no chip for {ME}: {:?}", open.attendees));
    assert_eq!(before, "not answered");
    let accepted = card(&first, Some((Attendance::Accepted, 0)));
    assert_eq!(mine(&accepted), Some("accepted"));
    // An answer to an earlier version is not an answer to this one.
    let update = request(2);
    let stale = card(&update, Some((Attendance::Declined, 0)));
    assert_eq!(mine(&stale), Some("not answered"));
}

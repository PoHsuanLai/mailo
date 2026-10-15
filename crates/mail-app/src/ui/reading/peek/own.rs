//! The kinds Peek draws with mailo's own readers: a calendar file, a contact card and a forwarded
//! message. Each is read into a [`Sheet`]: a title, a few facts under it and, when the part has
//! some, its text.
//!
//! The readers are the ones the rest of mailo reads these kinds with: the invitation card's
//! (`ui/invite`), the contacts' vCard reader and the message parser. Nothing here runs a part's
//! content, follows a reference in it or fetches anything; a forwarded message shows its plain
//! text only, so its HTML (and every image or link in it) is never drawn.

use crate::ui::copy;
use mail_domain::{Address, MessageId};

/// What Peek draws for a part mailo reads itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sheet {
    pub title: String,
    /// A label and its value, in the order they are drawn.
    pub facts: Vec<(&'static str, String)>,
    /// The part's own words: an event's description, a card's note, a message's text.
    pub text: Option<String>,
}

/// An iCalendar file, as the invitation card reads one: what it is, when and where, and who asked.
/// `message` is the message it came in, which the card names. Times are in the reader's zone.
pub(super) fn calendar(message: MessageId, bytes: &[u8]) -> Option<Sheet> {
    let text = String::from_utf8_lossy(bytes);
    let calendar = mail_pim::ical::parse(&text).ok()?;
    let invite = mail_pim::summarise(&calendar, &[])?;
    let card = crate::ui::invite::card_of(message, &invite, None, &chrono::Local);
    let facts = [
        Some((copy::fields::WHEN, card.when)),
        card.repeats.map(|repeats| (copy::fields::REPEATS, repeats)),
        card.location
            .map(|location| (copy::fields::WHERE, location)),
        card.organiser
            .map(|organiser| (copy::fields::ORGANISER, organiser)),
    ];
    Some(Sheet {
        title: card.title,
        facts: facts.into_iter().flatten().collect(),
        text: card.description,
    })
}

/// A vCard: the first card in it, with its addresses (the preferred first), numbers and
/// organisation.
pub(super) fn contact(bytes: &[u8]) -> Option<Sheet> {
    let card = mail_pim::vcard::parse_bytes(bytes).into_iter().next()?;
    let mut facts: Vec<(&'static str, String)> = card
        .addresses_by_preference()
        .into_iter()
        .map(|address| (copy::fields::EMAIL, address.to_owned()))
        .collect();
    facts.extend(
        card.phones
            .iter()
            .map(|phone| (copy::fields::PHONE, phone.number.clone())),
    );
    let org = card.org.join(", ");
    if !org.trim().is_empty() {
        facts.push((copy::fields::ORGANISATION, org));
    }
    Some(Sheet {
        title: card
            .display_name()
            .unwrap_or_else(|| copy::empty::NO_NAME.to_owned()),
        facts,
        text: card
            .note
            .map(|note| note.trim().to_owned())
            .filter(|note| !note.is_empty()),
    })
}

/// A message, as a forward attaches one: its subject, who sent it to whom and when, and its plain
/// text.
pub(super) fn message(bytes: &[u8]) -> Option<Sheet> {
    let parsed = mail_mime::parse(bytes).ok()?;
    let to = parsed.to.iter().map(person).collect::<Vec<_>>().join(", ");
    let facts = [
        parsed
            .from
            .as_ref()
            .map(|from| (copy::fields::FROM, person(from))),
        (!to.is_empty()).then_some((copy::fields::TO, to)),
        parsed.date.map(|date| {
            (
                copy::fields::DATE,
                mail_core::when::stamp(date, &chrono::Local, mail_core::when::Stamp::Full),
            )
        }),
    ];
    let subject = parsed.subject.trim();
    Some(Sheet {
        title: if subject.is_empty() {
            copy::empty::NO_SUBJECT.to_owned()
        } else {
            subject.to_owned()
        },
        facts: facts.into_iter().flatten().collect(),
        text: Some(
            parsed
                .text
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| copy::status::NO_TEXT.to_owned()),
        ),
    })
}

/// A person as a fact names them: their name and address, or the address alone.
fn person(address: &Address) -> String {
    match address.name.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => format!("{name} <{}>", address.email),
        _ => address.email.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(sheet: &Sheet) -> Vec<&'static str> {
        sheet.facts.iter().map(|(label, _)| *label).collect()
    }

    #[test]
    fn a_calendar_file_reads_as_its_event() {
        let ics = b"BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nMETHOD:PUBLISH\r\n\
BEGIN:VEVENT\r\nUID:one@example.test\r\nDTSTAMP:20261001T090000Z\r\n\
DTSTART:20261020T090000Z\r\nDTEND:20261020T100000Z\r\nSUMMARY:Design review\r\n\
LOCATION:Room 4\r\nDESCRIPTION:Bring the mockups.\r\n\
ORGANIZER;CN=Ada:mailto:ada@example.test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let sheet = calendar(MessageId::generate(), ics).expect("an event");
        assert_eq!(sheet.title, "Design review");
        assert_eq!(
            labels(&sheet),
            [
                copy::fields::WHEN,
                copy::fields::WHERE,
                copy::fields::ORGANISER
            ]
        );
        assert_eq!(sheet.facts[1].1, "Room 4");
        assert_eq!(sheet.text.as_deref(), Some("Bring the mockups."));
        assert_eq!(
            calendar(MessageId::generate(), b"BEGIN:VCALENDAR\r\n"),
            None
        );
    }

    #[test]
    fn a_contact_card_reads_as_its_person() {
        let vcf = b"BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ada Lovelace\r\n\
EMAIL;PREF=2:ada@home.test\r\nEMAIL;PREF=1:ada@work.test\r\nTEL:+44 20 0000\r\n\
ORG:Analytical Engines\r\nEND:VCARD\r\n";
        let sheet = contact(vcf).expect("a card");
        assert_eq!(sheet.title, "Ada Lovelace");
        assert_eq!(
            sheet.facts,
            [
                (copy::fields::EMAIL, "ada@work.test".to_owned()),
                (copy::fields::EMAIL, "ada@home.test".to_owned()),
                (copy::fields::PHONE, "+44 20 0000".to_owned()),
                (copy::fields::ORGANISATION, "Analytical Engines".to_owned()),
            ]
        );
        assert_eq!(contact(b"not a card"), None);
    }

    #[test]
    fn a_forwarded_message_reads_as_its_head_and_plain_text() {
        let eml = b"From: Ada <ada@example.test>\r\nTo: me@example.test\r\n\
Subject: Lunch on Friday\r\nDate: Tue, 13 Oct 2026 09:00:00 +0000\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nSee you at noon.\r\n";
        let sheet = message(eml).expect("a message");
        assert_eq!(sheet.title, "Lunch on Friday");
        assert_eq!(
            labels(&sheet),
            [copy::fields::FROM, copy::fields::TO, copy::fields::DATE]
        );
        assert_eq!(sheet.facts[0].1, "Ada <ada@example.test>");
        assert_eq!(sheet.text.as_deref(), Some("See you at noon."));

        // HTML only: its markup is never drawn, so there is no text to show.
        let html = b"From: ada@example.test\r\nSubject: \r\nMIME-Version: 1.0\r\n\
Content-Type: text/html\r\n\r\n<p>Hi <img src=\"https://tracker.test/p.gif\"></p>\r\n";
        let sheet = message(html).expect("a message");
        assert_eq!(sheet.title, copy::empty::NO_SUBJECT);
        assert_eq!(sheet.text.as_deref(), Some(copy::status::NO_TEXT));
    }
}

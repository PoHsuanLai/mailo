//! Writing cards, always as vCard 4.0.

use super::{Card, Name};
use crate::line::{self, ContentLine, escape};

/// One card as RFC 6350 text, CRLF line endings, folded at 75 octets.
///
/// `FN` is required in 4.0, so a card without one is given its display name, else its first
/// address, else an empty one — which is still a valid card, where omitting it is not.
pub fn write(card: &Card) -> String {
    let fallback = card
        .display_name()
        .or_else(|| card.emails.first().map(|e| e.address.clone()))
        .unwrap_or_default();
    let mut lines = vec![
        ContentLine::new("BEGIN", "VCARD"),
        ContentLine::new("VERSION", "4.0"),
    ];
    if let Some(uid) = &card.uid {
        lines.push(ContentLine::new("UID", escape(uid)));
    }
    if let Some(kind) = &card.kind {
        lines.push(ContentLine::new("KIND", kind.as_str()));
    }
    lines.push(ContentLine::new(
        "FN",
        escape(card.formatted_name.as_deref().unwrap_or(&fallback)),
    ));
    if let Some(name) = &card.name {
        lines.push(ContentLine::new("N", structured(name)));
    }
    for email in &card.emails {
        // `internet` said "an e-mail address" in 2.1 and 3.0, which in 4.0 every EMAIL is.
        let kinds: Vec<&str> = email
            .kinds
            .iter()
            .map(String::as_str)
            .filter(|k| *k != "internet")
            .collect();
        lines.push(typed(
            ContentLine::new("EMAIL", escape(&email.address)),
            &kinds,
            email.pref,
        ));
    }
    for phone in &card.phones {
        let kinds: Vec<&str> = phone.kinds.iter().map(String::as_str).collect();
        // `VALUE=text`: a number as someone typed it, spaces and brackets included, is not a
        // `tel:` URI, and 4.0 allows text for exactly that reason.
        lines.push(typed(
            ContentLine::new("TEL", escape(&phone.number)).with("VALUE", &["text"]),
            &kinds,
            phone.pref,
        ));
    }
    if !card.org.is_empty() {
        let parts: Vec<String> = card.org.iter().map(|p| escape(p)).collect();
        lines.push(ContentLine::new("ORG", parts.join(";")));
    }
    if let Some(note) = &card.note {
        lines.push(ContentLine::new("NOTE", escape(note)));
    }
    if let Some(photo) = &card.photo {
        lines.push(ContentLine::new("PHOTO", photo.clone()));
    }
    if let Some(rev) = &card.revision {
        lines.push(ContentLine::new("REV", rev.clone()));
    }
    // A URI, not text: written as it was read, with no escaping (RFC 6350 §6.6.5). Only a group
    // has members; a `MEMBER` on anything else rides in `other`, where the reader left it.
    if card.is_group() {
        lines.extend(
            card.members
                .iter()
                .map(|uri| ContentLine::new("MEMBER", uri.clone())),
        );
    }
    lines.extend(card.other.iter().cloned());
    lines.push(ContentLine::new("END", "VCARD"));
    lines.iter().map(line::write).collect()
}

/// Several cards, one after another, as one `.vcf` file holds them.
pub fn write_all(cards: &[Card]) -> String {
    cards.iter().map(write).collect()
}

fn typed(mut line: ContentLine, kinds: &[&str], pref: Option<u8>) -> ContentLine {
    if !kinds.is_empty() {
        line = line.with("TYPE", kinds);
    }
    if let Some(pref) = pref {
        line = line.with("PREF", &[&pref.to_string()]);
    }
    line
}

fn structured(name: &Name) -> String {
    [
        &name.family,
        &name.given,
        &name.additional,
        &name.prefixes,
        &name.suffixes,
    ]
    .iter()
    .map(|part| escape(part))
    .collect::<Vec<_>>()
    .join(";")
}

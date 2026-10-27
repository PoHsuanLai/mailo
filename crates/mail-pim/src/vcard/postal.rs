//! `ADR` (RFC 6350 §6.3.1): a postal address, read from and written into a card's other
//! properties.
//!
//! The card keeps `ADR` among [`Card::other`], as it came, so a card read and written again by
//! anything that does not edit its addresses keeps each one byte for byte. Only the address-book
//! editor asks for them as fields, and gives them back.

use super::Card;
use crate::line::{ContentLine, escape, split_escaped};

/// One postal address: `ADR`'s seven parts (post office box; extended address; street;
/// locality; region; postal code; country), each as written, comma-joined where the card gave
/// several values.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Postal {
    /// `TYPE` values, lower-cased: `home`, `work`.
    pub kinds: Vec<String>,
    /// Kept as read: RFC 6350 asks writers to leave it empty, and nothing here edits it.
    pub po_box: String,
    /// Kept as read, for the same reason.
    pub extended: String,
    /// The street, which may run over several lines.
    pub street: String,
    /// The town or city.
    pub locality: String,
    /// The state, province or county.
    pub region: String,
    pub code: String,
    pub country: String,
}

impl Postal {
    /// Whether every part a person writes is empty: such an address is not worth a line.
    pub fn is_empty(&self) -> bool {
        [
            &self.po_box,
            &self.extended,
            &self.street,
            &self.locality,
            &self.region,
            &self.code,
            &self.country,
        ]
        .iter()
        .all(|part| part.trim().is_empty())
    }

    fn read(line: &ContentLine) -> Self {
        let mut parts = split_escaped(&line.value, ';').into_iter();
        let mut next = || parts.next().unwrap_or_default();
        Self {
            kinds: line
                .values("TYPE")
                .map(str::to_ascii_lowercase)
                .filter(|kind| kind != "pref")
                .collect(),
            po_box: next(),
            extended: next(),
            street: next(),
            locality: next(),
            region: next(),
            code: next(),
            country: next(),
        }
    }

    fn line(&self) -> ContentLine {
        let value = [
            &self.po_box,
            &self.extended,
            &self.street,
            &self.locality,
            &self.region,
            &self.code,
            &self.country,
        ]
        .iter()
        .map(|part| escape(part))
        .collect::<Vec<_>>()
        .join(";");
        let line = ContentLine::new("ADR", value);
        if self.kinds.is_empty() {
            return line;
        }
        let kinds: Vec<&str> = self.kinds.iter().map(String::as_str).collect();
        line.with("TYPE", &kinds)
    }
}

impl Card {
    /// The card's postal addresses, in the order written.
    pub fn postal(&self) -> Vec<Postal> {
        self.other
            .iter()
            .filter(|line| line.name == "ADR")
            .map(Postal::read)
            .collect()
    }

    /// Replace the card's postal addresses with `addresses`, empty ones left out. Where the
    /// card held addresses, the new ones go where the first of them was; otherwise after
    /// everything else. Given back the addresses it holds, the card is left as it was, every
    /// `ADR` line as it was read.
    pub fn set_postal(&mut self, addresses: &[Postal]) {
        if self.postal() == addresses {
            return;
        }
        let at = self
            .other
            .iter()
            .position(|line| line.name == "ADR")
            .unwrap_or(self.other.len());
        let before = self.other[..at].iter().filter(|line| line.name != "ADR");
        let after = self.other[at..].iter().filter(|line| line.name != "ADR");
        let new = addresses
            .iter()
            .filter(|address| !address.is_empty())
            .map(Postal::line);
        self.other = before.cloned().chain(new).chain(after.cloned()).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vcard::{parse, write};

    const CARD: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:Ada\r\n\
                        ADR;TYPE=home:;;12 Crescent\\, Flat 2;Bath;Somerset;BA1;UK\r\n\
                        X-KEPT:yes\r\n\
                        ADR;TYPE=work,pref:PO 7;;;London;;;\r\nEND:VCARD\r\n";

    #[test]
    fn a_cards_postal_addresses_are_read_part_by_part() {
        let card = parse(CARD).remove(0);
        let postal = card.postal();
        assert_eq!(postal.len(), 2);
        assert_eq!(postal[0].kinds, ["home"]);
        assert_eq!(postal[0].street, "12 Crescent, Flat 2");
        assert_eq!(postal[0].locality, "Bath");
        assert_eq!(postal[1].kinds, ["work"], "pref is not a kind");
        assert_eq!(postal[1].po_box, "PO 7");
        assert_eq!(postal[1].locality, "London");
    }

    #[test]
    fn addresses_given_back_unchanged_leave_the_card_as_it_was() {
        let mut card = parse(CARD).remove(0);
        let before = card.clone();
        card.set_postal(&card.postal());
        assert_eq!(card, before);
    }

    #[test]
    fn edited_addresses_take_the_place_of_the_old_ones_and_round_trip() {
        let mut card = parse(CARD).remove(0);
        let mut postal = card.postal();
        postal[0].street = "1 New Road; Rear".to_owned();
        postal.push(Postal::default());
        postal.remove(1);
        card.set_postal(&postal);
        let names: Vec<&str> = card.other.iter().map(|line| line.name.as_str()).collect();
        assert_eq!(
            names,
            ["ADR", "X-KEPT"],
            "where the first was; the empty one left out"
        );
        let again = parse(&write(&card)).remove(0);
        assert_eq!(again.postal(), [postal[0].clone()]);
    }

    #[test]
    fn a_card_without_addresses_gets_them_after_everything_else() {
        let mut card =
            parse("BEGIN:VCARD\r\nVERSION:4.0\r\nFN:A\r\nX-A:1\r\nEND:VCARD\r\n").remove(0);
        let home = Postal {
            locality: "Paris".to_owned(),
            ..Postal::default()
        };
        card.set_postal(std::slice::from_ref(&home));
        assert_eq!(
            card.other.last().map(|line| line.name.as_str()),
            Some("ADR")
        );
        assert_eq!(card.postal(), [home]);
    }
}

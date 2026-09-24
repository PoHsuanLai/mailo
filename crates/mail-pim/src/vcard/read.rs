//! Reading cards, of any version a phone or a webmail export is likely to produce.

use super::{Card, Email, Name, Phone, Version};
use crate::line::{self, ContentLine, split_escaped, unescape};

/// Every card in `text`, in order.
///
/// Lenient by design, because the input is a file another program wrote: text outside a
/// `BEGIN:VCARD`…`END:VCARD` pair is ignored, a line that does not parse is skipped, and a card
/// cut off by the end of the file is kept with what it had. A card nested inside another (a 2.1
/// `AGENT`) is not a contact of this book and is passed over.
pub fn parse(text: &str) -> Vec<Card> {
    let mut cards = Vec::new();
    let mut current: Option<Card> = None;
    let mut depth = 0usize;
    for line in line::lines(text) {
        let is_vcard = line.value.trim().eq_ignore_ascii_case("VCARD");
        match line.name.as_str() {
            "BEGIN" if is_vcard => {
                if depth == 0 {
                    current = Some(Card::new());
                }
                depth += 1;
            }
            "END" if is_vcard => {
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some(card) = current.take()
                {
                    cards.push(card);
                }
            }
            _ if depth == 1 => {
                if let Some(card) = current.as_mut() {
                    read_property(card, line);
                }
            }
            _ => {}
        }
    }
    cards.extend(current);
    cards
}

/// [`parse`], for bytes of unknown encoding: UTF-8 when they are, otherwise Windows-1252, which
/// is what a 2.1 export without a `CHARSET` on every line was written in.
pub fn parse_bytes(bytes: &[u8]) -> Vec<Card> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => parse(text),
        Err(_) => parse(&encoding_rs::WINDOWS_1252.decode(bytes).0),
    }
}

fn read_property(card: &mut Card, line: ContentLine) {
    match line.name.as_str() {
        "VERSION" => {
            card.version = match line.value.trim() {
                "2.1" => Version::V2_1,
                "3.0" => Version::V3_0,
                _ => Version::V4_0,
            }
        }
        "UID" => card.uid = non_empty(unescape(&line.value)),
        "FN" => card.formatted_name = non_empty(unescape(&line.value)),
        "N" => card.name = Some(name(&line.value)),
        "EMAIL" => {
            let address = unescape(&line.value);
            let address = address.trim();
            let address = address.strip_prefix("mailto:").unwrap_or(address);
            if !address.is_empty() {
                let (kinds, pref) = kinds_and_pref(&line);
                card.emails.push(Email {
                    address: address.to_owned(),
                    kinds,
                    pref,
                });
            }
        }
        "TEL" => {
            let number = unescape(&line.value);
            let number = number.trim();
            let number = number.strip_prefix("tel:").unwrap_or(number);
            if !number.is_empty() {
                let (kinds, pref) = kinds_and_pref(&line);
                card.phones.push(Phone {
                    number: number.to_owned(),
                    kinds,
                    pref,
                });
            }
        }
        "ORG" => {
            let mut parts = split_escaped(&line.value, ';');
            while parts.last().is_some_and(|p| p.trim().is_empty()) {
                parts.pop();
            }
            card.org = parts;
        }
        // A 2.1 note decoded from quoted-printable carries the CRLFs its encoder saw.
        "NOTE" => card.note = non_empty(unescape(&line.value).replace("\r\n", "\n")),
        "REV" => card.revision = non_empty(line.value.trim().to_owned()),
        "PHOTO" => card.photo = photo(&line),
        // Ours to write, not to carry: the version is always 4.0 on the way out, and a product
        // id would claim that the program that wrote the original also wrote the copy.
        "PRODID" => {}
        _ => {
            // An inline binary value in an older card has no 4.0 spelling but a `data:` URI,
            // and only `PHOTO` is worth building one for.
            if line.has("ENCODING", "b") || line.has("ENCODING", "BASE64") {
                return;
            }
            card.other.push(line);
        }
    }
}

fn non_empty(text: String) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn name(value: &str) -> Name {
    let mut parts = split_escaped(value, ';').into_iter();
    let mut next = || {
        parts
            .next()
            .map(|p| p.trim().to_owned())
            .unwrap_or_default()
    };
    Name {
        family: next(),
        given: next(),
        additional: next(),
        prefixes: next(),
        suffixes: next(),
    }
}

/// `TYPE` values lower-cased, less `pref`; and the preference, from `PREF` or from a 3.0 or 2.1
/// `TYPE=pref`.
fn kinds_and_pref(line: &ContentLine) -> (Vec<String>, Option<u8>) {
    let kinds: Vec<String> = line
        .values("TYPE")
        .flat_map(|v| v.split(','))
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "pref")
        .collect();
    let pref = line
        .values("PREF")
        .find_map(|v| v.trim().parse::<u8>().ok())
        .or_else(|| line.has("TYPE", "pref").then_some(1));
    (kinds, pref)
}

/// A 4.0 URI as written, or an older card's inline base64 as a `data:` URI.
fn photo(line: &ContentLine) -> Option<String> {
    let value: String = line.value.split_whitespace().collect();
    if value.is_empty() {
        return None;
    }
    if !(line.has("ENCODING", "b") || line.has("ENCODING", "BASE64")) {
        return Some(value);
    }
    let subtype = line
        .values("TYPE")
        .next()
        .map(|t| t.to_ascii_lowercase())
        .map(|t| t.strip_prefix("image/").map(str::to_owned).unwrap_or(t))
        .unwrap_or_else(|| "jpeg".to_owned());
    Some(format!("data:image/{subtype};base64,{value}"))
}

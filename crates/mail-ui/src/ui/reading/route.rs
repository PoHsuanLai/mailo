//! Who draws an attachment: mailo, or anyview.
//!
//! mailo draws what it has always drawn itself: the pictures it decodes under its own size limits
//! (`mail_client::attachment_image`), a PDF a page at a time, and the calendars, contacts and messages it
//! reads anyway (an invitation, a contact card, a forwarded message). Everything else goes to
//! anyview, which peeks at the stored bytes and draws what it finds: a picture it can decode, a
//! few lines, an archive's entries, or just the file's facts. With the `anyview` feature off, that
//! is "no preview", as it was.
//!
//! An SVG is neither's to draw. It is a document with scripts and references of its own, not a
//! picture, and mailo never draws one from a sender (`mail_client::attachment_image`); a part that looks like
//! one stays unpreviewed, whoever would have drawn it.
//!
//! The rule reads the part's own first bytes and name, never its declared type: the type is the
//! sender's claim.

use mail_client::attachment_image;

/// How many leading bytes the rule reads.
pub(super) const HEAD: usize = 512;

/// A kind mailo draws itself, or keeps from everyone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Own {
    /// A picture or a PDF that `mail_client::attachment_image` decodes.
    Picture,
    /// An iCalendar file: an invitation or an event.
    Calendar,
    /// A vCard.
    Contact,
    /// A message, as a forward attaches one (`message/rfc822`, an `.eml`).
    Message,
    /// An SVG, which no one draws.
    Svg,
}

/// Who draws a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Read by the strip and Peek only with the `anyview` feature on.
#[cfg_attr(not(feature = "anyview"), allow(dead_code))]
pub(super) enum Hand {
    /// mailo: a kind of its [`Own`].
    Mailo,
    /// anyview, when the build has it.
    Anyview,
}

/// Which of mailo's own kinds the part called `name` whose first bytes are `head` is, if any.
pub(super) fn own(name: &str, head: &[u8]) -> Option<Own> {
    if is_svg(name, head) {
        Some(Own::Svg)
    } else if attachment_image::sniff(head).is_some() {
        Some(Own::Picture)
    } else if named(name, &["ics", "ical", "ifb"]) || begins(head, b"BEGIN:VCALENDAR") {
        Some(Own::Calendar)
    } else if named(name, &["vcf", "vcard"]) || begins(head, b"BEGIN:VCARD") {
        Some(Own::Contact)
    } else if named(name, &["eml"]) || is_message(head) {
        Some(Own::Message)
    } else {
        None
    }
}

/// Who draws the part called `name` whose first bytes are `head`.
#[cfg_attr(not(feature = "anyview"), allow(dead_code))]
pub(super) fn hand(name: &str, head: &[u8]) -> Hand {
    match own(name, head) {
        Some(_) => Hand::Mailo,
        None => Hand::Anyview,
    }
}

/// Whether `name` ends in one of `extensions`, in any case.
fn named(name: &str, extensions: &[&str]) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

/// Whether `head`, past leading white space, starts with `tag`, in any case.
fn begins(head: &[u8], tag: &[u8]) -> bool {
    head.trim_ascii_start()
        .get(..tag.len())
        .is_some_and(|got| got.eq_ignore_ascii_case(tag))
}

/// Whether the part is an SVG: by its extension, or by an `<svg` in its first bytes.
fn is_svg(name: &str, head: &[u8]) -> bool {
    named(name, &["svg", "svgz"])
        || head
            .windows(4)
            .any(|window| window.eq_ignore_ascii_case(b"<svg"))
}

/// Header fields only a message's head carries: a note that starts "To: Ada" is not one.
const MESSAGE_FIELDS: [&str; 6] = [
    "from",
    "date",
    "message-id",
    "received",
    "mime-version",
    "return-path",
];

/// Whether `head` reads as a message's header: every line up to the first empty one (or the end
/// of what was read) a field or its folded continuation, and one of them a field only a message
/// has.
fn is_message(head: &[u8]) -> bool {
    let mut known = false;
    let lines: Vec<&[u8]> = head.split(|&byte| byte == b'\n').collect();
    for (at, line) in lines.iter().enumerate() {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() {
            break;
        }
        if line[0] == b' ' || line[0] == b'\t' {
            continue;
        }
        let Some(colon) = line.iter().position(|&byte| byte == b':') else {
            // The last line read may be cut short where the read stopped; any other line without
            // a colon is not a header.
            return known && at + 1 == lines.len();
        };
        let field = &line[..colon];
        if field.is_empty()
            || !field
                .iter()
                .all(|&b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return false;
        }
        known |= MESSAGE_FIELDS
            .iter()
            .any(|name| field.eq_ignore_ascii_case(name.as_bytes()));
    }
    known
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n rest";
    const PDF: &[u8] = b"%PDF-1.7\n";

    /// Name, first bytes, who draws it.
    #[test]
    fn a_part_goes_to_mailo_for_what_it_draws_itself_and_to_anyview_for_the_rest() {
        let cases: &[(&str, &str, &[u8], Hand)] = &[
            ("a png", "beach.png", PNG, Hand::Mailo),
            ("a pdf", "report.pdf", PDF, Hand::Mailo),
            // The bytes decide, not the name.
            ("a png under a text name", "notes.txt", PNG, Hand::Mailo),
            (
                "a text file named like a png",
                "beach.png",
                b"hello",
                Hand::Anyview,
            ),
            ("an invitation by name", "invite.ics", b"", Hand::Mailo),
            (
                "an invitation by name, upper case",
                "INVITE.ICS",
                b"x",
                Hand::Mailo,
            ),
            (
                "an invitation by its start",
                "invite",
                b"BEGIN:VCALENDAR\r\n",
                Hand::Mailo,
            ),
            ("a card by name", "ada.vcf", b"", Hand::Mailo),
            (
                "a card by its start",
                "ada",
                b"  begin:vcard\r\n",
                Hand::Mailo,
            ),
            ("an svg by name", "logo.svg", b"", Hand::Mailo),
            (
                "a compressed svg by name",
                "logo.SVGZ",
                b"\x1f\x8b",
                Hand::Mailo,
            ),
            (
                "an svg by its start",
                "logo",
                b"<?xml version=\"1.0\"?><svg ",
                Hand::Mailo,
            ),
            ("a bitmap", "scan.bmp", b"BM\x36\x00\x00\x00", Hand::Anyview),
            ("an archive", "pack.zip", b"PK\x03\x04", Hand::Anyview),
            ("plain text", "notes.txt", b"first line\n", Hand::Anyview),
            ("nothing, unnamed", "", b"", Hand::Anyview),
            ("a name with no extension", "ics", b"hello", Hand::Anyview),
        ];
        for (case, name, head, want) in cases {
            assert_eq!(hand(name, head), *want, "{case}");
        }
    }

    #[test]
    fn a_card_is_found_past_leading_white_space_only() {
        assert_eq!(own("x", b"\r\n\t BEGIN:VCARD"), Some(Own::Contact));
        assert_eq!(own("x", b"x BEGIN:VCARD"), None);
        assert_eq!(own("x", b"BEGIN:VCAL"), None);
    }

    /// Name, first bytes, which of mailo's kinds.
    #[test]
    fn a_forwarded_message_is_known_by_its_name_or_its_header() {
        let cases: &[(&str, &str, &[u8], Option<Own>)] = &[
            ("by name", "Lunch.eml", b"", Some(Own::Message)),
            (
                "by its header",
                "forwarded",
                b"Received: by mx\r\n\tfor <me@example.test>\r\nFrom: Ada <ada@example.test>\r\nSubject: Hi\r\n\r\nbody",
                Some(Own::Message),
            ),
            (
                "a header cut short where the read stopped",
                "forwarded",
                b"From: ada@example.test\r\nSubj",
                Some(Own::Message),
            ),
            (
                "a note that starts like a header",
                "notes.txt",
                b"To: Ada\nSubject: lunch\n\nsee you",
                None,
            ),
            ("prose", "notes.txt", b"Dear Ada,\nFrom: me\n", None),
            ("a calendar by name", "invite.ics", b"", Some(Own::Calendar)),
            ("a contact by name", "ada.VCF", b"", Some(Own::Contact)),
            ("an svg before a picture", "logo.svg", b"\x89PNG\r\n\x1a\n", Some(Own::Svg)),
            ("a picture", "x", b"\x89PNG\r\n\x1a\n rest", Some(Own::Picture)),
        ];
        for (case, name, head, want) in cases {
            assert_eq!(own(name, head), *want, "{case}");
        }
    }
}

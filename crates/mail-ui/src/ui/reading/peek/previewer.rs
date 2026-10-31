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
//! sender's claim. Each kind mailo draws is a [`Previewer`], and [`PREVIEWERS`] is the order they
//! are asked in: the first to claim a part draws it.

use super::own::{self, Sheet};
use super::part::{Drawn, picture};
use mail_client::Client;
use mail_client::attachment_image;
use mail_client::when::Clock;
use mail_domain::{BlobId, MessageId};
use mail_ui_model::copy;
use std::fmt::Debug;

/// How many leading bytes the rule reads.
pub(in crate::ui::reading) const HEAD: usize = 512;

/// The most of a calendar, a contact or a message that is read whole to draw it.
const OWN_LIMIT: u64 = 16 << 20;

/// A stored part to draw: where it is and what the drawing is asked for.
pub(super) struct Part<'a> {
    pub client: &'a Client,
    pub message: MessageId,
    pub index: usize,
    /// The page of a PDF.
    pub page: u32,
    pub clock: Clock,
    pub blob: BlobId,
}

/// A kind of part mailo draws itself, or keeps from everyone.
pub(super) trait Previewer: Debug + Sync {
    /// Whether the part called `name` whose first bytes are `head` is of this kind.
    fn claims(&self, name: &str, head: &[u8]) -> bool;

    /// What Peek draws for `part`.
    fn draw(&self, part: &Part<'_>) -> Drawn;
}

/// Every kind mailo draws, in the order they are asked: an SVG before a picture, since an SVG
/// that begins like an image is still no picture.
pub(super) const PREVIEWERS: &[&dyn Previewer] = &[&Svg, &Picture, &Calendar, &Contact, &Message];

/// The kind that draws the part called `name` whose first bytes are `head`, if mailo does.
pub(super) fn claimant(name: &str, head: &[u8]) -> Option<&'static dyn Previewer> {
    PREVIEWERS
        .iter()
        .copied()
        .find(|previewer| previewer.claims(name, head))
}

/// Whether mailo draws the part called `name` whose first bytes are `head`, rather than anyview.
#[cfg(any(feature = "anyview", test))]
pub(in crate::ui::reading) fn mailo_draws(name: &str, head: &[u8]) -> bool {
    claimant(name, head).is_some()
}

/// An SVG, which no one draws.
#[derive(Debug)]
struct Svg;

impl Previewer for Svg {
    fn claims(&self, name: &str, head: &[u8]) -> bool {
        is_svg(name, head)
    }

    fn draw(&self, _part: &Part<'_>) -> Drawn {
        Drawn::Said(copy::status::NO_PREVIEW.to_owned())
    }
}

/// A picture or a PDF that `mail_client::attachment_image` decodes.
#[derive(Debug)]
struct Picture;

impl Previewer for Picture {
    fn claims(&self, _name: &str, head: &[u8]) -> bool {
        attachment_image::sniff(head).is_some()
    }

    fn draw(&self, part: &Part<'_>) -> Drawn {
        picture(part.client, part.message, part.index, part.page)
    }
}

/// An iCalendar file: an invitation or an event.
#[derive(Debug)]
struct Calendar;

impl Previewer for Calendar {
    fn claims(&self, name: &str, head: &[u8]) -> bool {
        named(name, &["ics", "ical", "ifb"]) || begins(head, b"BEGIN:VCALENDAR")
    }

    fn draw(&self, part: &Part<'_>) -> Drawn {
        sheet(part, |bytes| own::calendar(part.message, bytes, part.clock))
    }
}

/// A vCard.
#[derive(Debug)]
struct Contact;

impl Previewer for Contact {
    fn claims(&self, name: &str, head: &[u8]) -> bool {
        named(name, &["vcf", "vcard"]) || begins(head, b"BEGIN:VCARD")
    }

    fn draw(&self, part: &Part<'_>) -> Drawn {
        sheet(part, own::contact)
    }
}

/// A message, as a forward attaches one (`message/rfc822`, an `.eml`).
#[derive(Debug)]
struct Message;

impl Previewer for Message {
    fn claims(&self, name: &str, head: &[u8]) -> bool {
        named(name, &["eml"]) || is_message(head)
    }

    fn draw(&self, part: &Part<'_>) -> Drawn {
        sheet(part, |bytes| own::message(bytes, part.clock))
    }
}

/// `part` read whole, up to [`OWN_LIMIT`], and made a [`Sheet`] by `read`.
fn sheet(part: &Part<'_>, read: impl FnOnce(&[u8]) -> Option<Sheet>) -> Drawn {
    let blobs = part.client.blobs();
    let whole = match blobs.size(part.blob) {
        Ok(size) if size <= OWN_LIMIT => blobs.get(part.blob),
        Ok(_) => return Drawn::Said(copy::status::NO_PREVIEW.to_owned()),
        Err(why) => Err(why),
    };
    match whole {
        Ok(bytes) => read(&bytes).map_or_else(
            || Drawn::Said(copy::status::NO_PREVIEW.to_owned()),
            Drawn::Sheet,
        ),
        Err(why) => super::part::unreadable(why),
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
        let cases: &[(&str, &str, &[u8], bool)] = &[
            ("a png", "beach.png", PNG, true),
            ("a pdf", "report.pdf", PDF, true),
            // The bytes decide, not the name.
            ("a png under a text name", "notes.txt", PNG, true),
            ("a text file named like a png", "beach.png", b"hello", false),
            ("an invitation by name", "invite.ics", b"", true),
            (
                "an invitation by name, upper case",
                "INVITE.ICS",
                b"x",
                true,
            ),
            (
                "an invitation by its start",
                "invite",
                b"BEGIN:VCALENDAR\r\n",
                true,
            ),
            ("a card by name", "ada.vcf", b"", true),
            ("a card by its start", "ada", b"  begin:vcard\r\n", true),
            ("an svg by name", "logo.svg", b"", true),
            ("a compressed svg by name", "logo.SVGZ", b"\x1f\x8b", true),
            (
                "an svg by its start",
                "logo",
                b"<?xml version=\"1.0\"?><svg ",
                true,
            ),
            ("a bitmap", "scan.bmp", b"BM\x36\x00\x00\x00", false),
            ("an archive", "pack.zip", b"PK\x03\x04", false),
            ("plain text", "notes.txt", b"first line\n", false),
            ("nothing, unnamed", "", b"", false),
            ("a name with no extension", "ics", b"hello", false),
        ];
        for (case, name, head, want) in cases {
            assert_eq!(mailo_draws(name, head), *want, "{case}");
        }
    }

    /// Which kind claims a part, by its name.
    fn kind(name: &str, head: &[u8]) -> Option<String> {
        claimant(name, head).map(|previewer| format!("{previewer:?}"))
    }

    #[test]
    fn a_card_is_found_past_leading_white_space_only() {
        assert_eq!(kind("x", b"\r\n\t BEGIN:VCARD").as_deref(), Some("Contact"));
        assert_eq!(kind("x", b"x BEGIN:VCARD"), None);
        assert_eq!(kind("x", b"BEGIN:VCAL"), None);
    }

    /// Name, first bytes, which of mailo's kinds.
    #[test]
    fn a_forwarded_message_is_known_by_its_name_or_its_header() {
        let cases: &[(&str, &str, &[u8], Option<&str>)] = &[
            ("by name", "Lunch.eml", b"", Some("Message")),
            (
                "by its header",
                "forwarded",
                b"Received: by mx\r\n\tfor <me@example.test>\r\nFrom: Ada <ada@example.test>\r\nSubject: Hi\r\n\r\nbody",
                Some("Message"),
            ),
            (
                "a header cut short where the read stopped",
                "forwarded",
                b"From: ada@example.test\r\nSubj",
                Some("Message"),
            ),
            (
                "a note that starts like a header",
                "notes.txt",
                b"To: Ada\nSubject: lunch\n\nsee you",
                None,
            ),
            ("prose", "notes.txt", b"Dear Ada,\nFrom: me\n", None),
            ("a calendar by name", "invite.ics", b"", Some("Calendar")),
            ("a contact by name", "ada.VCF", b"", Some("Contact")),
            // An SVG is asked before a picture, whose bytes it may begin with.
            ("an svg before a picture", "logo.svg", b"\x89PNG\r\n\x1a\n", Some("Svg")),
            ("a picture", "x", b"\x89PNG\r\n\x1a\n rest", Some("Picture")),
        ];
        for (case, name, head, want) in cases {
            assert_eq!(kind(name, head).as_deref(), *want, "{case}");
        }
    }
}

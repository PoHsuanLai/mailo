//! Header fields written in raw 8-bit bytes of some legacy charset.
//!
//! RFC 5322 headers are ASCII, RFC 2047 encoded-words carry anything else, and RFC 6532 allows
//! raw UTF-8. Real mail also carries a fourth kind: Big5, GBK, Shift_JIS, EUC-KR, KOI8-R or a
//! windows-125x code page written straight into `Subject` or a display name, by old servers and
//! by some senders to this day. A parser that assumes UTF-8 turns every such byte into U+FFFD,
//! and a Chinese subject becomes a row of replacement characters that no search can find.
//!
//! The rule, per field:
//!
//! 1. bytes that are valid UTF-8 are left alone (which covers ASCII, and so every encoded-word);
//! 2. otherwise the charset the message itself declares — the `charset` of the top-level
//!    `Content-Type`, or of the first text part — when it decodes the field without error;
//! 3. otherwise a detector's guess, fed every undecodable field at once so short subjects are
//!    judged together with the display names beside them.
//!
//! The rewrite happens before the structured parse rather than after, because a display name
//! and an address share one field: `From: <GBK bytes> <a@b>` has to be decoded as a whole for
//! the address parser to see the angle brackets where they are.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::Encoding;
use mail_parser::{MessageParser, MimeHeaders};

/// `raw` with every top-level header field that is not UTF-8 decoded into UTF-8, or `None` when
/// every field already was — the common case, which costs one UTF-8 check of the header block.
///
/// The body is carried over byte for byte: its own `Content-Type` says how to read it, and a
/// part's bytes are not this function's to reinterpret.
pub(crate) fn headers_as_utf8(raw: &[u8]) -> Option<Vec<u8>> {
    let end = header_end(raw);
    let head = &raw[..end];
    if std::str::from_utf8(head).is_ok() {
        return None;
    }
    let fields = fields(head);
    let foreign: Vec<&[u8]> = fields
        .iter()
        .copied()
        .filter(|field| std::str::from_utf8(field).is_err())
        .collect();
    let encoding = choose(raw, &foreign);
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 4);
    for field in fields {
        if std::str::from_utf8(field).is_ok() {
            out.extend_from_slice(field);
        } else {
            // With replacement, as a last resort: a field the chosen charset cannot decode in
            // full keeps what it can, which is still more than U+FFFD for every byte.
            let (text, _) = encoding.decode_without_bom_handling(field);
            out.extend_from_slice(text.as_bytes());
        }
    }
    out.extend_from_slice(&raw[end..]);
    Some(out)
}

/// The charset to read `foreign` with: the declared one when it reads all of them cleanly, else
/// the detector's guess.
fn choose(raw: &[u8], foreign: &[&[u8]]) -> &'static Encoding {
    let hints = hints(raw);
    if let Some(declared) = hints.declared.as_deref().and_then(usable)
        && foreign.iter().all(|field| {
            declared
                .decode_without_bom_handling_and_without_replacement(field)
                .is_some()
        })
    {
        return declared;
    }
    guess(foreign, hints.tld.as_deref())
}

/// A declared charset worth trying.
///
/// Not UTF-8 — the field already failed to be UTF-8, and a sender who declares UTF-8 and writes
/// GBK is the commonest mislabel there is. Not US-ASCII either: a field with 8-bit bytes in it
/// is proof the declaration is wrong, and the Encoding Standard maps the label to windows-1252,
/// which decodes any byte at all and would turn every such subject into Latin mojibake. And not
/// anything that does not keep ASCII as ASCII (UTF-16, ISO-2022-JP), since the field's colon,
/// brackets and line breaks are ASCII and must survive the decode.
fn usable(label: &str) -> Option<&'static Encoding> {
    const ASCII: &[&str] = &[
        "us-ascii",
        "ascii",
        "us",
        "ansi_x3.4-1968",
        "iso646-us",
        "iso-ir-6",
        "csascii",
        "cp367",
        "ibm367",
    ];
    let label = label.trim().trim_matches('"');
    if ASCII.iter().any(|ascii| label.eq_ignore_ascii_case(ascii)) {
        return None;
    }
    let encoding = Encoding::for_label_no_replacement(label.as_bytes())?;
    (encoding != encoding_rs::UTF_8 && encoding.is_ascii_compatible()).then_some(encoding)
}

fn guess(foreign: &[&[u8]], tld: Option<&[u8]>) -> &'static Encoding {
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    for field in foreign {
        detector.feed(field, false);
        // A field boundary is a line break in the message; telling the detector so keeps the
        // last byte of one field from pairing with the first of the next.
        detector.feed(b"\n", false);
    }
    detector.feed(b"", true);
    detector.guess(tld, Utf8Detection::Deny)
}

/// What the message says about itself that bears on its headers' charset.
struct Hints {
    /// The top-level `Content-Type` charset, else the first text part's.
    declared: Option<String>,
    /// The sender's top-level domain, lower-case ASCII, which the detector weighs: a `.tw`
    /// sender makes Big5 likelier than GBK for the same bytes.
    tld: Option<Vec<u8>>,
}

/// Read the hints from a lossy first parse. Only ASCII is read out of it — a charset label and a
/// domain — and neither is touched by what the lossy decode did to the fields around them.
fn hints(raw: &[u8]) -> Hints {
    let Some(message) = MessageParser::default().parse(raw) else {
        return Hints {
            declared: None,
            tld: None,
        };
    };
    let charset = |part: &mail_parser::MessagePart<'_>| {
        part.content_type()
            .and_then(|ct| ct.attribute("charset"))
            .map(str::to_owned)
    };
    let is_text = |part: &&mail_parser::MessagePart<'_>| {
        part.content_type()
            .is_some_and(|ct| ct.ctype().eq_ignore_ascii_case("text"))
    };
    let declared = message
        .parts
        .first()
        .and_then(charset)
        .or_else(|| message.parts.iter().filter(is_text).find_map(charset));
    let tld = message
        .from()
        .and_then(|from| from.first())
        .and_then(|addr| addr.address())
        .and_then(tld_of);
    Hints { declared, tld }
}

/// The last label of the address's domain, when it is a plain ASCII label. The detector panics
/// on anything with a period, an upper-case letter or a non-ASCII byte, and those bytes came
/// from a stranger, so anything unusual is no hint at all.
fn tld_of(email: &str) -> Option<Vec<u8>> {
    let (_, domain) = email.rsplit_once('@')?;
    let label = domain.trim().trim_end_matches('.').rsplit('.').next()?;
    let label = label.to_ascii_lowercase();
    (!label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then(|| label.into_bytes())
}

/// Where the header block ends: after the blank line that separates it from the body, or the
/// whole message when there is none.
fn header_end(raw: &[u8]) -> usize {
    let mut at = 0;
    while at < raw.len() {
        let line_end = raw[at..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(raw.len(), |p| at + p + 1);
        let line = &raw[at..line_end];
        if line == b"\n" || line == b"\r\n" {
            return at;
        }
        at = line_end;
    }
    raw.len()
}

/// The header block split into fields, each with its continuation lines and line endings, so
/// that concatenating them gives back the block exactly.
fn fields(head: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < head.len() {
        let line_end = head[at..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(head.len(), |p| at + p + 1);
        let next = head.get(line_end).copied();
        // A field ends where the next line does not begin with folding whitespace.
        if !matches!(next, Some(b' ' | b'\t')) {
            out.push(&head[start..line_end]);
            start = line_end;
        }
        at = line_end;
    }
    if start < head.len() {
        out.push(&head[start..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_reassemble_to_the_block() {
        let head = b"A: 1\r\nB: 2\r\n  more\r\n\tand more\r\nC: 3";
        let split = fields(head);
        assert_eq!(split.len(), 3);
        assert_eq!(split.concat(), head.to_vec());
    }

    #[test]
    fn the_header_block_ends_at_the_blank_line() {
        assert_eq!(header_end(b"A: 1\r\n\r\nbody"), 6);
        assert_eq!(header_end(b"A: 1\n\nbody"), 5);
        assert_eq!(header_end(b"A: 1\r\n"), 6);
    }

    #[test]
    fn a_hostile_domain_is_no_hint_rather_than_a_panic() {
        assert_eq!(tld_of("a@example.TW"), Some(b"tw".to_vec()));
        assert_eq!(tld_of("a@example.tw."), Some(b"tw".to_vec()));
        assert_eq!(tld_of("a@例子.中国"), None);
        assert_eq!(tld_of("a@"), None);
        assert_eq!(tld_of("nobody"), None);
    }
}

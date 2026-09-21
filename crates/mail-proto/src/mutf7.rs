//! IMAP modified UTF-7 for mailbox names (RFC 3501 §5.1.3).
//!
//! Written here rather than taken as a dependency. The only crate offering it is unmaintained
//! and its decoder ends in an `unwrap()` on base64 reached from a regex that accepts any bytes,
//! so a server sending a mailbox named `&A-` panics the process. There is no `Result` anywhere
//! in it.
//!
//! Three rules this encoding punishes you for forgetting:
//!
//! - **The base64 alphabet is case-sensitive and uses `,` where standard base64 uses `/`.**
//!   Case-folding a mailbox name corrupts it.
//! - **A literal `&` encodes as `&-`.** Missing this turns one mailbox into two.
//! - **Decoding must never fail hard.** A name we cannot decode is a cosmetic problem; a
//!   connection we dropped because of one is an account that stopped working. Malformed input
//!   falls back to the literal bytes.
//!
//! Note also that advertising `UTF8=ACCEPT` does not mean a server has stopped speaking this:
//! Dovecot 2.4 with `mail_utf8_extensions=yes` still hands out modified UTF-7 names. The decoder
//! is always safe to run — an ASCII name passes through unchanged.

/// The modified base64 alphabet: standard, with `,` in place of `/`.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

fn sextet(byte: u8) -> Option<u16> {
    ALPHABET.iter().position(|c| *c == byte).map(|i| i as u16)
}

/// Encode a mailbox name for the wire.
///
/// Printable ASCII except `&` passes through. Everything else is UTF-16BE in modified base64
/// between `&` and `-`.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending: Vec<u16> = Vec::new();

    for ch in name.chars() {
        // RFC 3501: printable US-ASCII is 0x20..=0x7e, and `&` must be escaped.
        let plain = matches!(ch, '\u{20}'..='\u{7e}') && ch != '&';
        if plain {
            flush(&mut pending, &mut out);
            out.push(ch);
        } else if ch == '&' {
            flush(&mut pending, &mut out);
            out.push_str("&-");
        } else {
            let mut buf = [0u16; 2];
            pending.extend_from_slice(ch.encode_utf16(&mut buf));
        }
    }
    flush(&mut pending, &mut out);
    out
}

/// Emit any buffered UTF-16 as one `&...-` run.
fn flush(pending: &mut Vec<u16>, out: &mut String) {
    if pending.is_empty() {
        return;
    }
    out.push('&');
    let mut bits: u32 = 0;
    let mut width = 0u32;
    for unit in pending.iter() {
        bits = (bits << 16) | u32::from(*unit);
        width += 16;
        while width >= 6 {
            width -= 6;
            let index = ((bits >> width) & 0x3f) as usize;
            out.push(ALPHABET[index] as char);
        }
    }
    if width > 0 {
        // Pad the final group with zero bits; no `=` padding in this encoding.
        let index = ((bits << (6 - width)) & 0x3f) as usize;
        out.push(ALPHABET[index] as char);
    }
    out.push('-');
    pending.clear();
}

/// Decode a mailbox name from the wire.
///
/// Total. Anything malformed — a bad base64 character, an unterminated run, an unpaired
/// surrogate — yields the literal input for that run rather than an error, because a mailbox we
/// cannot name is survivable and a dropped connection is not.
pub fn decode(wire: &str) -> String {
    let bytes = wire.as_bytes();
    let mut out = String::with_capacity(wire.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != b'&' {
            // Not a shift: copy this character through. Indexing by byte is safe because a
            // conforming name is ASCII, and a non-conforming one is handled by from_utf8_lossy
            // on the slice below.
            let start = i;
            while i < bytes.len() && bytes[i] != b'&' {
                i += 1;
            }
            out.push_str(&String::from_utf8_lossy(&bytes[start..i]));
            continue;
        }
        // `&-` is a literal ampersand.
        if bytes.get(i + 1) == Some(&b'-') {
            out.push('&');
            i += 2;
            continue;
        }
        let Some(end) = bytes[i + 1..].iter().position(|b| *b == b'-') else {
            // Unterminated run: keep the rest verbatim rather than guessing.
            out.push_str(&String::from_utf8_lossy(&bytes[i..]));
            break;
        };
        let run = &bytes[i + 1..i + 1 + end];
        match decode_run(run) {
            Some(text) => out.push_str(&text),
            // Malformed: emit the run as written, including its delimiters, so the name round
            // trips as literal text instead of vanishing.
            None => out.push_str(&String::from_utf8_lossy(&bytes[i..i + 2 + end])),
        }
        i += end + 2;
    }
    out
}

/// One `&...-` run to text, or `None` if it is not valid modified base64 UTF-16BE.
fn decode_run(run: &[u8]) -> Option<String> {
    if run.is_empty() {
        return None;
    }
    let mut bits: u32 = 0;
    let mut width = 0u32;
    let mut units: Vec<u16> = Vec::new();
    for byte in run {
        bits = (bits << 6) | u32::from(sextet(*byte)?);
        width += 6;
        if width >= 16 {
            width -= 16;
            units.push(((bits >> width) & 0xffff) as u16);
        }
    }
    // Leftover bits are padding and must be zero; anything else is malformed.
    if width > 0 && (bits & ((1 << width) - 1)) != 0 {
        return None;
    }
    // A run too short to hold even one UTF-16 unit — `&A-` — is malformed. Without this it
    // decodes to the empty string and the mailbox name silently disappears, which is the
    // failure the literal-bytes fallback exists to prevent.
    if units.is_empty() {
        return None;
    }
    // Rejects unpaired surrogates, which is why this is fallible rather than lossy.
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors from RFC 3501 §5.1.3 plus the cases implementations get wrong.
    const ROUND_TRIP: &[(&str, &str)] = &[
        ("INBOX", "INBOX"),
        ("Drafts", "Drafts"),
        ("[Gmail]/All Mail", "[Gmail]/All Mail"),
        // RFC 3501's own examples.
        ("~peter/mail/台北/日本語", "~peter/mail/&U,BTFw-/&ZeVnLIqe-"),
        ("Hello world!", "Hello world!"),
        // A literal ampersand, which is the case that silently splits a mailbox in two.
        ("A&B", "A&-B"),
        ("&", "&-"),
        ("R&D", "R&-D"),
        // Outside the BMP: surrogate pairs must survive.
        ("emoji \u{1F600}", "emoji &2D3eAA-"),
        ("", ""),
    ];

    #[test]
    fn encodes_and_decodes_the_reference_vectors() {
        for (text, wire) in ROUND_TRIP {
            assert_eq!(&encode(text), wire, "encoding {text:?}");
            assert_eq!(&decode(wire), text, "decoding {wire:?}");
        }
    }

    #[test]
    fn decoding_never_fails_however_malformed() {
        // Every one of these came from a real class of bug: the crate we rejected panics on the
        // first. A mailbox we cannot name is survivable; a dropped connection is not.
        for hostile in [
            "&A-",           // not a whole UTF-16 unit
            "&",             // unterminated run
            "&////-",        // '/' is not in this alphabet
            "&2D3-",         // unpaired high surrogate
            "&AAAAAAAAAAAA", // unterminated and long
            "&-&-&-",
            "&&&",
            "\u{0}&AAA-",
        ] {
            let decoded = decode(hostile);
            assert!(
                !decoded.is_empty() || hostile.is_empty(),
                "{hostile:?} decoded to nothing rather than to literal text"
            );
        }
    }

    #[test]
    fn case_is_never_folded() {
        // The alphabet is case-sensitive, so folding corrupts the name rather than normalising
        // it. These two differ only in case and must decode differently.
        assert_ne!(decode("&AEE-"), decode("&aee-"));
    }

    #[test]
    fn ascii_passes_through_untouched() {
        // The decoder runs on every name, including from servers that speak UTF8=ACCEPT.
        for name in ["INBOX", "[Gmail]/Sent Mail", "Work/2024", "a.b.c"] {
            assert_eq!(decode(name), name);
            assert_eq!(encode(name), name);
        }
    }
}

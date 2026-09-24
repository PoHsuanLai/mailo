//! Setting a built message's `Date` to the moment it leaves.
//!
//! The bytes of a message are frozen when the user presses Send, so that an edit made while the
//! outbox waits cannot change what goes out. The `Date` written then is the moment it was
//! frozen — and for a send the user asked to go at nine tomorrow, that is today, which the
//! recipient's client would file it under. RFC 5322 §3.6.1 makes `Date` the moment the message
//! was ready to enter the mail system, and a scheduled message is not ready until its time.
//!
//! So the field is replaced as the message is handed over, and nothing else is touched. That is
//! only safe because nothing here signs the bytes: a DKIM signature added by this client would
//! cover `Date` and break. Signing, when it exists, has to happen after this.

use chrono::{DateTime, Utc};

/// `message` with its `Date` header field saying `at`, and every other byte as it was.
///
/// Exactly one `Date` comes out: the first one in the header is replaced in place, any later
/// duplicate is dropped (RFC 5322 §3.6 allows one), and a message with none gains one at the top.
/// A folded `Date` is replaced whole, continuation lines included. The field name is compared as
/// a token, so `X-Date` and `Date-Sent` are other fields and stay; the body is never read, so a
/// line in it that happens to begin `Date:` is text.
pub fn restamp(message: &[u8], at: DateTime<Utc>) -> Vec<u8> {
    let stamped = format!("Date: {}\r\n", at.to_rfc2822());
    let (head, body) = split_head(message);

    let mut out = Vec::with_capacity(message.len() + stamped.len());
    let mut placed = false;
    for field in fields(head) {
        if !is_date(field) {
            out.extend_from_slice(field);
        } else if !placed {
            out.extend_from_slice(stamped.as_bytes());
            placed = true;
        }
    }
    if !placed {
        out.splice(0..0, stamped.bytes());
    }
    out.extend_from_slice(body);
    out
}

/// The header block, and everything from the blank line that ends it.
///
/// A message with no blank line is all header, which is what a reader would make of it too.
pub(crate) fn split_head(message: &[u8]) -> (&[u8], &[u8]) {
    let mut start = 0;
    while start < message.len() {
        let end = message[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(message.len(), |at| start + at + 1);
        let line = &message[start..end];
        if line == b"\r\n" || line == b"\n" {
            return message.split_at(start);
        }
        start = end;
    }
    (message, &[])
}

/// Each header field, with its continuation lines and line endings.
pub(crate) fn fields(head: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut field_start = 0;
    let mut line_start = 0;
    while line_start < head.len() {
        let line_end = head[line_start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(head.len(), |at| line_start + at + 1);
        // A line that opens with white space continues the field above it (RFC 5322 §2.2.3).
        let continues = matches!(head[line_start], b' ' | b'\t');
        if !continues && line_start > field_start {
            out.push(&head[field_start..line_start]);
            field_start = line_start;
        }
        line_start = line_end;
    }
    if field_start < head.len() {
        out.push(&head[field_start..]);
    }
    out
}

/// Whether this field is `Date`, by its name and nothing else.
fn is_date(field: &[u8]) -> bool {
    let Some(colon) = field.iter().position(|&b| b == b':') else {
        return false;
    };
    field[..colon]
        .trim_ascii_end()
        .eq_ignore_ascii_case(b"Date")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn leaving() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 25, 9, 0, 0).unwrap()
    }

    const STAMP: &str = "Date: Fri, 25 Sep 2026 09:00:00 +0000\r\n";

    fn stamped(message: &str) -> String {
        String::from_utf8(restamp(message.as_bytes(), leaving())).unwrap()
    }

    #[test]
    fn the_date_says_when_it_left_and_nothing_else_moves() {
        const CASES: &[(&str, &str, &str)] = &[
            (
                "in the middle",
                "From: a@example.test\r\nDate: Thu, 24 Sep 2026 17:00:00 +0000\r\nSubject: hi\r\n\r\nbody\r\n",
                "From: a@example.test\r\n{STAMP}Subject: hi\r\n\r\nbody\r\n",
            ),
            (
                "folded over two lines",
                "Date: Thu, 24 Sep 2026\r\n 17:00:00 +0000\r\nSubject: hi\r\n\r\nbody\r\n",
                "{STAMP}Subject: hi\r\n\r\nbody\r\n",
            ),
            (
                "spelled in another case, with space before the colon",
                "DATE : Thu, 24 Sep 2026 17:00:00 +0000\r\nSubject: hi\r\n\r\nbody\r\n",
                "{STAMP}Subject: hi\r\n\r\nbody\r\n",
            ),
            (
                "missing",
                "From: a@example.test\r\n\r\nbody\r\n",
                "{STAMP}From: a@example.test\r\n\r\nbody\r\n",
            ),
            (
                "twice",
                "Date: one\r\nSubject: hi\r\nDate: two\r\n\r\nbody\r\n",
                "{STAMP}Subject: hi\r\n\r\nbody\r\n",
            ),
            (
                "beside fields whose names only start or end with it",
                "X-Date: keep\r\nDate-Sent: keep\r\nDate: old\r\n\r\nbody\r\n",
                "X-Date: keep\r\nDate-Sent: keep\r\n{STAMP}\r\nbody\r\n",
            ),
            (
                "and a body line that looks like one",
                "Date: old\r\n\r\nDate: this is the body\r\n",
                "{STAMP}\r\nDate: this is the body\r\n",
            ),
            (
                "with bare line feeds",
                "Date: old\nSubject: hi\n\nbody\n",
                "{STAMP}Subject: hi\n\nbody\n",
            ),
        ];
        for (case, input, expected) in CASES {
            assert_eq!(
                stamped(input),
                expected.replace("{STAMP}", STAMP),
                "a Date {case}"
            );
        }
    }

    #[test]
    fn a_built_message_reads_back_with_the_new_date() {
        let built = "From: a@example.test\r\nDate: Thu, 24 Sep 2026 17:00:00 +0000\r\n\
                     Subject: hi\r\nMIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nbody\r\n";
        let parsed = crate::parse(&restamp(built.as_bytes(), leaving())).unwrap();
        assert_eq!(parsed.date, Some(leaving()));
        assert_eq!(parsed.subject, "hi");
    }

    #[test]
    fn stamping_twice_is_stamping_once() {
        let once = restamp(b"Date: old\r\nSubject: hi\r\n\r\nbody\r\n", leaving());
        assert_eq!(restamp(&once, leaving()), once);
    }
}

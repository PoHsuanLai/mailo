use super::*;
use mail_domain::{MailboxRole, ReadState, Star};
use std::io::{BufReader, Cursor, Read};

fn read_all(bytes: &[u8]) -> Vec<MboxMessage> {
    Reader::new(Cursor::new(bytes.to_vec()))
        .collect::<Result<Vec<_>, _>>()
        .expect("reads")
}

fn written(messages: &[(&Envelope, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (envelope, raw) in messages {
        write(&mut out, envelope, raw).expect("writes");
    }
    out
}

fn envelope() -> Envelope {
    Envelope {
        sender: "ada@example.test".to_owned(),
        date: parse_date("Sat Jan  1 10:20:30 2022"),
    }
}

const FIRST: &[u8] = b"From: Ada <ada@example.test>\nSubject: one\n\nFrom the desk of Ada.\n>From a quoted line\n>>From deeper\nplain\n";
const SECOND: &[u8] = b"From: Bob <bob@example.test>\nSubject: two\n\nsecond body\n";

#[test]
fn a_body_line_beginning_from_survives_a_round_trip() {
    let bytes = written(&[(&envelope(), FIRST), (&envelope(), SECOND)]);
    let text = String::from_utf8_lossy(&bytes);
    // mboxrd: every `^>*From ` gains one `>`.
    assert!(text.contains("\n>From the desk of Ada.\n"), "{text}");
    assert!(text.contains("\n>>From a quoted line\n"), "{text}");
    assert!(text.contains("\n>>>From deeper\n"), "{text}");

    let back = read_all(&bytes);
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].raw, FIRST);
    assert_eq!(back[1].raw, SECOND);
    assert_eq!(back[0].envelope, envelope());

    // And out again, byte for byte.
    let again = written(&[
        (&back[0].envelope, &back[0].raw),
        (&back[1].envelope, &back[1].raw),
    ]);
    assert_eq!(again, bytes);
}

#[test]
fn crlf_files_split_the_same_and_keep_their_line_endings() {
    let bytes = b"From a@b Sat Jan  1 00:00:00 2022\r\nSubject: one\r\n\r\nbody\r\n>From x\r\n\r\nFrom a@b Sun Jan  2 00:00:00 2022\r\nSubject: two\r\n\r\nbody two\r\n\r\n";
    let back = read_all(bytes);
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].raw, b"Subject: one\r\n\r\nbody\r\nFrom x\r\n");
    assert_eq!(back[1].raw, b"Subject: two\r\n\r\nbody two\r\n");
}

#[test]
fn an_unquoted_from_in_prose_does_not_split_a_message() {
    // mboxo writers quote only `From `; careless ones quote nothing. Prose has no time of day.
    let bytes = b"From a@b Sat Jan  1 00:00:00 2022\nSubject: one\n\nhello\n\nFrom the desk of Ada\n\nFrom a@b Sat Jan  1 00:00:01 2022\nSubject: two\n\nx\n";
    let back = read_all(bytes);
    assert_eq!(back.len(), 2);
    assert_eq!(
        back[0].raw,
        b"Subject: one\n\nhello\n\nFrom the desk of Ada\n".to_vec()
    );
}

#[test]
fn content_length_is_honoured_where_it_lands_on_a_boundary() {
    // mboxcl2: no quoting, the length says where the body ends, and the body holds what would
    // otherwise look exactly like an envelope.
    let body = b"line\n\nFrom x@y Sat Jan  1 00:00:00 2022\nstill the body\n";
    let mut bytes = format!(
        "From a@b Sat Jan  1 00:00:00 2022\nSubject: one\nContent-Length: {}\n\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nFrom a@b Sun Jan  2 00:00:00 2022\nSubject: two\n\nx\n");
    let back = read_all(&bytes);
    assert_eq!(back.len(), 2, "{back:?}");
    assert!(back[0].raw.ends_with(body));
    assert_eq!(back[1].raw, b"Subject: two\n\nx\n");
}

#[test]
fn a_content_length_that_lands_elsewhere_is_ignored() {
    // Wrong by three bytes: splitting on it would cut the message and lose the next envelope.
    let bytes = b"From a@b Sat Jan  1 00:00:00 2022\nSubject: one\nContent-Length: 3\n\nhello\n\nFrom a@b Sun Jan  2 00:00:00 2022\nSubject: two\n\nx\n";
    let back = read_all(bytes);
    assert_eq!(back.len(), 2);
    assert_eq!(back[0].raw, b"Subject: one\nContent-Length: 3\n\nhello\n");
}

#[test]
fn a_file_that_is_not_an_mbox_says_so() {
    let mut reader = Reader::new(Cursor::new(b"Subject: hi\n\nbody\n".to_vec()));
    assert!(matches!(reader.next(), Some(Err(MboxError::NotMbox))));
    assert!(reader.next().is_none());
}

#[test]
fn takeout_envelopes_and_labels_are_read() {
    let bytes = b"From 1745238623546000000@xxx Tue Nov 07 13:20:51 +0000 2023\nX-GM-THRID: 1\nX-Gmail-Labels: Inbox,Unread,\"Travel, 2023\",Category Updates\nSubject: trip\n\nbody\n";
    let back = read_all(bytes);
    assert_eq!(back.len(), 1);
    assert_eq!(
        back[0].envelope.date.map(|d| d.to_rfc3339()),
        Some("2023-11-07T13:20:51+00:00".to_owned())
    );
    let placed = placement(&back[0].raw, Some("All mail Including Spam and Trash"));
    assert_eq!(placed.role, MailboxRole::Inbox);
    assert_eq!(placed.read(), ReadState::Unread);
    assert_eq!(placed.labels, vec!["Travel, 2023", "Category Updates"]);
}

#[test]
fn status_headers_and_the_file_name_place_ordinary_mbox_mail() {
    let flagged = b"Status: RO\nX-Status: AF\nSubject: a\n\nx\n";
    let placed = placement(flagged, Some("Receipts"));
    assert_eq!(placed.role, MailboxRole::Archive);
    assert_eq!(placed.labels, vec!["Receipts"]);
    assert_eq!(placed.star(), Star::Starred);
    assert!(placed.flags.contains(&SystemFlag::Answered));

    let mozilla = b"X-Mozilla-Status: 0000\nSubject: a\n\nx\n";
    assert_eq!(placement(mozilla, Some("Sent")).read(), ReadState::Unread);
    assert_eq!(placement(mozilla, Some("Sent")).role, MailboxRole::Sent);
    // Saying nothing is read: an archive is old mail.
    assert_eq!(
        placement(b"Subject: a\n\nx\n", None).read(),
        ReadState::Read
    );
}

#[test]
fn the_envelope_written_is_the_return_path_or_the_from_address() {
    let date = parse_date("Sat Jan  1 10:20:30 2022");
    assert_eq!(
        envelope_of(
            b"Return-Path: <bounce@list.test>\nFrom: A <a@b.test>\n\n",
            date
        )
        .sender,
        "bounce@list.test"
    );
    assert_eq!(
        envelope_of(b"From: \"Ada L\" <ada@example.test>\n\n", date).sender,
        "ada@example.test"
    );
    assert_eq!(envelope_of(b"Subject: x\n\n", None).sender, "MAILER-DAEMON");
}

/// Bytes of an mbox made as they are read, so the file never exists in memory whole.
struct Generated {
    remaining: usize,
    chunk: Vec<u8>,
    at: usize,
}

impl Generated {
    fn new(messages: usize) -> Self {
        let mut chunk =
            b"From gen@example.test Sat Jan  1 00:00:00 2022\nSubject: generated\n\n".to_vec();
        // About 4 KiB per message, with a line that has to be unquoted on the way in.
        for _ in 0..64 {
            chunk.extend_from_slice(
                b"The quick brown fox jumps over the lazy dog, again and again.\n",
            );
        }
        chunk.extend_from_slice(b">From inside the body\n\n");
        let at = chunk.len();
        Self {
            remaining: messages,
            chunk,
            at,
        }
    }
}

impl Read for Generated {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.at == self.chunk.len() {
            if self.remaining == 0 {
                return Ok(0);
            }
            self.remaining -= 1;
            self.at = 0;
        }
        let n = buf.len().min(self.chunk.len() - self.at);
        buf[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

#[test]
fn a_large_mbox_streams_one_message_at_a_time() {
    // 2,000 messages of ~4 KiB: about 8 MB that never exist in one buffer. Each message is
    // handed over and dropped before the next is read, so what the reader holds is bounded by
    // the largest message, which is what is asserted.
    let messages = 2_000;
    let reader = Reader::new(BufReader::with_capacity(8 * 1024, Generated::new(messages)));
    let mut count = 0usize;
    let mut total = 0usize;
    let mut largest = 0usize;
    for message in reader {
        let message = message.expect("reads");
        assert!(message.raw.ends_with(b"From inside the body\n"));
        count += 1;
        total += message.raw.len();
        largest = largest.max(message.raw.len());
    }
    assert_eq!(count, messages);
    assert!(total > 8_000_000, "{total} bytes streamed");
    assert!(largest < 8 * 1024, "one message is {largest} bytes");
}

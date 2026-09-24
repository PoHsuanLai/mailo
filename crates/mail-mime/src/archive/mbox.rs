//! The mbox family: one file, messages separated by `From ` envelope lines.
//!
//! There is no single mbox format, only a family that disagrees about one thing — what to do
//! with a body line that begins `From `. Reading has to accept all of them, because the file
//! was written by whatever the user's old client or their provider's export was:
//!
//! - **mboxo** prefixes `>` to a body line beginning `From `, and nothing else, so a body line
//!   that already read `>From ` cannot be told apart afterwards.
//! - **mboxrd** prefixes `>` to any line matching `^>*From `, which makes the quoting reversible.
//!   It is what this module writes.
//! - **mboxcl2** quotes nothing and puts a `Content-Length` header on each message instead.
//!
//! So the reader splits at an envelope line that follows a blank line and looks like one,
//! removes one `>` from every `^>+From ` line (mboxrd's reversal, which is right for mboxo's
//! only escape too), and where a message carries `Content-Length` it takes exactly that many
//! body bytes, unquoted — but only when the bytes after them are where the next envelope should
//! be. A length that points anywhere else is a header some other program wrote, and splitting
//! on it would cut a message in two.
//!
//! LF and CRLF files both read; lines keep whichever ending the file had.

use super::{Placement, folder_placement, header, lines, trim_eol};
use chrono::{DateTime, NaiveDateTime, Utc};
use mail_domain::SystemFlag;
use std::io::{self, BufRead, Write};

/// The envelope line's two facts: who the message came from on the wire, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// The envelope sender, or `MAILER-DAEMON` where there was none.
    pub sender: String,
    /// When it was delivered. `None` when the line's date could not be read.
    pub date: Option<DateTime<Utc>>,
}

/// One message out of an mbox, unquoted, without its envelope line or separating blank line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MboxMessage {
    pub envelope: Envelope,
    pub raw: Vec<u8>,
}

/// Why an mbox could not be read.
#[derive(Debug, thiserror::Error)]
pub enum MboxError {
    #[error("not an mbox: the first line is not a `From ` envelope line")]
    NotMbox,
    #[error("reading the mbox: {0}")]
    Io(#[from] io::Error),
}

/// Messages from an mbox, one at a time.
///
/// Holds one message in memory at a time, however large the file: a Google Takeout of every
/// message in an account is routinely several gigabytes.
pub struct Reader<R> {
    inner: R,
    /// Lines read ahead and handed back, last pushed first out.
    pushback: Vec<Vec<u8>>,
    state: State,
}

// By hand: the source need not be `Debug`, and its bytes are not worth printing.
impl<R> std::fmt::Debug for Reader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("pushback", &self.pushback.len())
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Start,
    Reading,
    Done,
}

impl<R: BufRead> Reader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            pushback: Vec::new(),
            state: State::Start,
        }
    }

    fn line(&mut self) -> io::Result<Option<Vec<u8>>> {
        if let Some(line) = self.pushback.pop() {
            return Ok(Some(line));
        }
        let mut line = Vec::new();
        match self.inner.read_until(b'\n', &mut line)? {
            0 => Ok(None),
            _ => Ok(Some(line)),
        }
    }

    fn unread(&mut self, line: Vec<u8>) {
        self.pushback.push(line);
    }

    /// Hand back `bytes` so they are read again next, as lines.
    fn unread_bytes(&mut self, bytes: &[u8]) {
        let split: Vec<Vec<u8>> = lines(bytes).map(<[u8]>::to_vec).collect();
        for line in split.into_iter().rev() {
            self.unread(line);
        }
    }

    /// The next envelope line, skipping blank lines. `None` at the end of the file.
    fn envelope_line(&mut self) -> Result<Option<Vec<u8>>, MboxError> {
        while let Some(line) = self.line()? {
            if trim_eol(&line).is_empty() {
                continue;
            }
            if line.starts_with(b"From ") {
                return Ok(Some(line));
            }
            return Err(MboxError::NotMbox);
        }
        Ok(None)
    }

    fn message(&mut self) -> Result<Option<MboxMessage>, MboxError> {
        let Some(from_line) = self.envelope_line()? else {
            return Ok(None);
        };
        let envelope = parse_envelope(trim_eol(&from_line));

        // The header section, up to and including the blank line that ends it.
        let mut raw = Vec::new();
        let mut length: Option<usize> = None;
        let mut ended = false;
        while let Some(line) = self.line()? {
            let blank = trim_eol(&line).is_empty();
            if let Some(n) = content_length(&line) {
                length = Some(n);
            }
            raw.extend_from_slice(&line);
            if blank {
                ended = true;
                break;
            }
        }
        if !ended {
            return Ok(Some(MboxMessage { envelope, raw }));
        }

        if let Some(n) = length
            && self.counted_body(n, &mut raw)?
        {
            return Ok(Some(MboxMessage { envelope, raw }));
        }
        self.scanned_body(&mut raw)?;
        Ok(Some(MboxMessage { envelope, raw }))
    }

    /// Take exactly `n` body bytes, if the next envelope (or the end) follows them.
    ///
    /// Returns whether it did. When it did not, everything read is handed back and `raw` is as
    /// it was, so the caller can scan instead.
    fn counted_body(&mut self, n: usize, raw: &mut Vec<u8>) -> io::Result<bool> {
        let mut body = Vec::with_capacity(n.min(1 << 20));
        while body.len() < n {
            let Some(line) = self.line()? else { break };
            let room = n - body.len();
            if line.len() > room {
                body.extend_from_slice(&line[..room]);
                self.unread(line[room..].to_vec());
            } else {
                body.extend_from_slice(&line);
            }
        }
        // What follows must be blank lines and then an envelope, or nothing at all.
        let mut after = Vec::new();
        let fits = loop {
            match self.line()? {
                None => break body.len() == n,
                Some(line) if trim_eol(&line).is_empty() => after.push(line),
                Some(line) => {
                    let envelope = line.starts_with(b"From ");
                    after.push(line);
                    break envelope && body.len() == n;
                }
            }
        };
        for line in after.into_iter().rev() {
            self.unread(line);
        }
        if fits {
            raw.extend_from_slice(&body);
        } else {
            self.unread_bytes(&body);
        }
        Ok(fits)
    }

    /// Read body lines up to the next envelope line, undoing `>From ` quoting.
    fn scanned_body(&mut self, raw: &mut Vec<u8>) -> io::Result<()> {
        // The blank line that ended the headers counts: an envelope straight after the headers
        // is a message with an empty body.
        let mut after_blank = true;
        while let Some(line) = self.line()? {
            if after_blank && line.starts_with(b"From ") && looks_like_envelope(trim_eol(&line)) {
                self.unread(line);
                break;
            }
            after_blank = trim_eol(&line).is_empty();
            raw.extend_from_slice(unquote(&line));
        }
        // The blank line before the next envelope, or at the end of the file, is the separator
        // the writer added, not part of the message.
        if raw.ends_with(b"\r\n\r\n") {
            raw.truncate(raw.len() - 2);
        } else if raw.ends_with(b"\n\n") {
            raw.truncate(raw.len() - 1);
        }
        Ok(())
    }
}

impl<R: BufRead> Iterator for Reader<R> {
    type Item = Result<MboxMessage, MboxError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state == State::Done {
            return None;
        }
        self.state = State::Reading;
        match self.message() {
            Ok(Some(message)) => Some(Ok(message)),
            Ok(None) => {
                self.state = State::Done;
                None
            }
            Err(e) => {
                // An error is the last thing said: past a line that is not an envelope, or a
                // failed read, there is nothing trustworthy to resume from.
                self.state = State::Done;
                Some(Err(e))
            }
        }
    }
}

/// Append one message to an mbox, mboxrd-quoted, in Unix line endings.
///
/// The envelope line, the message with every `^>*From ` line given one more `>`, a final
/// newline if the message lacked one, and the blank line that separates it from the next.
pub fn write<W: Write>(out: &mut W, envelope: &Envelope, raw: &[u8]) -> io::Result<()> {
    let sender: String = envelope
        .sender
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let sender = if sender.is_empty() {
        "MAILER-DAEMON".to_owned()
    } else {
        sender
    };
    let date = envelope.date.unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
    writeln!(out, "From {sender} {}", date.format("%a %b %e %H:%M:%S %Y"))?;
    let unix = super::lf(raw);
    for line in lines(&unix) {
        if needs_quote(line) {
            out.write_all(b">")?;
        }
        out.write_all(line)?;
    }
    if !unix.is_empty() && !unix.ends_with(b"\n") {
        out.write_all(b"\n")?;
    }
    out.write_all(b"\n")
}

/// The envelope to write for a message: its `Return-Path` where it has one, else its `From`
/// address, and `date`.
pub fn envelope_of(raw: &[u8], date: Option<DateTime<Utc>>) -> Envelope {
    let sender = header(raw, "Return-Path")
        .map(|v| v.trim_matches(['<', '>', ' ']).to_owned())
        .filter(|v| !v.is_empty())
        .or_else(|| header(raw, "From").and_then(|v| address_in(&v)))
        .unwrap_or_else(|| "MAILER-DAEMON".to_owned());
    Envelope { sender, date }
}

/// What a message's own status headers say had happened to it, where it says anything.
///
/// `Status` and `X-Status` are the Unix mail readers' (`R` read, `O` old, `F` flagged, `A`
/// answered); `X-Mozilla-Status` is a hex bit field (`0x1` read, `0x2` replied, `0x4` flagged).
/// `None` when none of them is present: then the file says nothing, and nothing is invented.
pub fn status_flags(raw: &[u8]) -> Option<Vec<SystemFlag>> {
    let status = header(raw, "Status");
    let x_status = header(raw, "X-Status");
    let mozilla =
        header(raw, "X-Mozilla-Status").and_then(|v| u32::from_str_radix(v.trim(), 16).ok());
    if status.is_none() && x_status.is_none() && mozilla.is_none() {
        return None;
    }
    let letters = format!(
        "{}{}",
        status.unwrap_or_default(),
        x_status.unwrap_or_default()
    );
    let mut flags = Vec::new();
    let bits = mozilla.unwrap_or(0);
    if letters.contains('R') || bits & 0x1 != 0 {
        flags.push(SystemFlag::Seen);
    }
    if letters.contains('A') || bits & 0x2 != 0 {
        flags.push(SystemFlag::Answered);
    }
    if letters.contains('F') || bits & 0x4 != 0 {
        flags.push(SystemFlag::Flagged);
    }
    Some(flags)
}

/// Where a message from an mbox belongs.
///
/// A Takeout `X-Gmail-Labels` header decides it outright. Otherwise the file's own name is the
/// folder — `Sent.mbox` is sent mail — and the message's status headers its flags; with no
/// status headers at all the message is taken as read, because an archive is old mail and
/// arriving as thousands of unread messages would bury the inbox.
pub fn placement(raw: &[u8], file_folder: Option<&str>) -> Placement {
    if let Some(labels) = super::takeout::labels(raw) {
        return super::takeout::placement(&labels);
    }
    let flags = status_flags(raw).unwrap_or_else(|| vec![SystemFlag::Seen]);
    folder_placement(file_folder, flags)
}

/// The address in a `From` value: inside angle brackets if there are any, else the first word
/// with an `@`.
fn address_in(value: &str) -> Option<String> {
    if let (Some(open), Some(close)) = (value.rfind('<'), value.rfind('>'))
        && open < close
    {
        let inner = value[open + 1..close].trim();
        return (!inner.is_empty()).then(|| inner.to_owned());
    }
    value
        .split_whitespace()
        .find(|w| w.contains('@'))
        .map(|w| w.trim_matches(['"', '(', ')', ',']).to_owned())
}

/// Whether a body line must be quoted to survive: `^>*From `.
fn needs_quote(line: &[u8]) -> bool {
    let rest = line
        .iter()
        .position(|b| *b != b'>')
        .map_or(&line[line.len()..], |i| &line[i..]);
    rest.starts_with(b"From ")
}

/// `^>+From ` loses one `>`; any other line is itself.
fn unquote(line: &[u8]) -> &[u8] {
    if line.first() == Some(&b'>') && needs_quote(line) {
        &line[1..]
    } else {
        line
    }
}

/// `Content-Length: n` as a header line, where it is one.
fn content_length(line: &[u8]) -> Option<usize> {
    let line = trim_eol(line);
    let colon = line.iter().position(|b| *b == b':')?;
    if !line[..colon].eq_ignore_ascii_case(b"Content-Length") {
        return None;
    }
    std::str::from_utf8(&line[colon + 1..])
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Whether a `From ` line is an envelope rather than prose that begins with the word.
///
/// Mbox writers that do not quote (mboxcl2, and careless ones) leave body lines like "From the
/// desk of…" behind a blank line, exactly where an envelope would be. Every envelope carries a
/// time of day, `hh:mm`, and prose almost never does.
fn looks_like_envelope(line: &[u8]) -> bool {
    let rest = line[b"From ".len()..].trim_ascii_start();
    // Past the sender: its first space.
    let Some(space) = rest.iter().position(|b| *b == b' ') else {
        return false;
    };
    rest[space..].windows(5).any(|w| {
        w[0].is_ascii_digit()
            && w[1].is_ascii_digit()
            && w[2] == b':'
            && w[3].is_ascii_digit()
            && w[4].is_ascii_digit()
    })
}

/// Sender and date from `From sender date`.
fn parse_envelope(line: &[u8]) -> Envelope {
    let text = String::from_utf8_lossy(line);
    let rest = text.strip_prefix("From ").unwrap_or(&text).trim_start();
    let (sender, date) = rest.split_once(' ').unwrap_or((rest, ""));
    Envelope {
        sender: sender.to_owned(),
        date: parse_date(date),
    }
}

/// An envelope date: ctime's `Sat Jan  1 00:00:00 2022`, with or without a zone after the year,
/// or Takeout's `Sat Jan 01 00:00:00 +0000 2022`.
fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let joined = words.join(" ");
    for format in ["%a %b %d %H:%M:%S %z %Y", "%a %b %d %H:%M:%S %Y %z"] {
        if let Ok(date) = DateTime::parse_from_str(&joined, format) {
            return Some(date.with_timezone(&Utc));
        }
    }
    // No zone: ctime is local time on the machine that delivered it, which nobody recorded.
    // UTC is as good a guess as any and better than none.
    let zoneless = words.get(..5).map(|w| w.join(" "))?;
    NaiveDateTime::parse_from_str(&zoneless, "%a %b %d %H:%M:%S %Y")
        .ok()
        .map(|naive| naive.and_utc())
}

#[cfg(test)]
#[path = "mbox_tests.rs"]
mod tests;

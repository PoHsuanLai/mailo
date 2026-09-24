//! Printing: a message or a thread as one self-contained HTML document.
//!
//! The document itself is [`mail_mime::print`], which is pure. This module is the part that
//! reads the store — which messages, their stored bytes — and the part that writes a file.
//! The window prints the same [`document`] through its webview.

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{Message, MessageId, ThreadId};
use mail_mime::{Pages, Parsed, Sheet};
use mail_store::{SqliteStore, Store};
use std::path::{Path, PathBuf};

/// A printable document, and the subject it is named after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Printed {
    /// The first message's subject: what a file of it is called.
    pub subject: String,
    /// The whole HTML document.
    pub html: String,
}

/// The messages `id` names, oldest first.
///
/// `id` is tried as a message first and then as a thread, because both print as a uuid and
/// the person should not have to say which one they copied — as `mailo unsubscribe` does.
pub fn messages(store: &SqliteStore, id: uuid::Uuid) -> Result<Vec<Message>, String> {
    if let Ok(message) = store.message(MessageId::from_uuid(id)) {
        return Ok(vec![message]);
    }
    let thread = store
        .thread(ThreadId::from_uuid(id))
        .map_err(|_| format!("{id} is neither a message nor a thread"))?;
    let mut messages = thread
        .messages
        .iter()
        .filter_map(|id| store.message(*id).ok())
        .collect::<Vec<_>>();
    // A conversation reads in the order it was written.
    messages.sort_by_key(|message| message.date);
    if messages.is_empty() {
        return Err(format!("thread {id} holds no messages"));
    }
    Ok(messages)
}

/// The printable document for a message or thread, dates in `zone`.
pub fn document<Tz>(
    store: &SqliteStore,
    id: uuid::Uuid,
    zone: &Tz,
    now: DateTime<Utc>,
    pages: Pages,
) -> Result<Printed, String>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let messages = messages(store, id)?;
    Ok(of_messages(store, &messages, zone, now, pages))
}

/// [`document`] for messages already in hand, in the order given.
///
/// For the window, which holds the open thread's messages already.
pub fn of_messages<Tz>(
    store: &SqliteStore,
    messages: &[Message],
    zone: &Tz,
    now: DateTime<Utc>,
    pages: Pages,
) -> Printed
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let parsed: Vec<Option<Parsed>> = messages
        .iter()
        .map(|message| parse_body(store, message))
        .collect();
    let sheets: Vec<Sheet<'_>> = messages
        .iter()
        .zip(&parsed)
        .map(|(message, parsed)| Sheet {
            message,
            parsed: parsed.as_ref(),
        })
        .collect();
    Printed {
        subject: messages
            .first()
            .map(|message| message.subject.clone())
            .unwrap_or_default(),
        html: mail_mime::print(&sheets, zone, now, pages),
    }
}

/// The message's stored bytes, parsed; `None` for the cases the reader also falls back on.
fn parse_body(store: &SqliteStore, message: &Message) -> Option<Parsed> {
    let raw = message.body.raw()?;
    let bytes = store.blobs().get(&store.connection(), raw).ok()?;
    mail_mime::parse(&bytes).ok()
}

/// The longest subject, in bytes, a file name keeps.
const MAX_STEM: usize = 200;

/// The file name a printout of `subject` gets: the subject, made safe the way an attachment's
/// name is, with `.html` on the end.
///
/// A slash in a subject is punctuation (`Q1/Q2 figures`), not a directory, so it becomes a
/// dash before [`crate::attach::safe_name`] would otherwise keep only what follows it.
pub fn file_name(subject: &str) -> String {
    let flat: String = subject
        .trim()
        .chars()
        .map(|c| if matches!(c, '/' | '\\') { '-' } else { c })
        .collect();
    let stem = flat.trim_matches(|c: char| c == '.' || c.is_whitespace());
    let stem = if stem.is_empty() { "message" } else { stem };
    // Short enough that `.html` and a ` (999)` both still fit in a 255-byte name, so neither
    // is what gets cut.
    let mut short = String::new();
    for c in stem.chars() {
        if short.len() + c.len_utf8() > MAX_STEM {
            break;
        }
        short.push(c);
    }
    crate::attach::safe_name(&format!("{}.html", short.trim_end()))
}

/// Write `printed` into `dir` under its subject's name, never over a file already there.
///
/// `create_new` rather than a check and then a write: the name is only free if claiming it
/// succeeds.
pub fn write_into(dir: &Path, printed: &Printed) -> Result<PathBuf, String> {
    use std::io::Write as _;
    let name = file_name(&printed.subject);
    let path = Path::new(&name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("message");
    for n in 1..1000 {
        let candidate = if n == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem} ({n}).html"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(printed.html.as_bytes())
                    .map_err(|e| format!("{}: {e}", candidate.display()))?;
                return Ok(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", candidate.display())),
        }
    }
    Err(format!(
        "{} and 998 numbered copies of it already exist in {}",
        name,
        dir.display()
    ))
}

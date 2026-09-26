//! Printing: a message or a thread as one self-contained HTML document.
//!
//! The document itself is [`mail_mime::print`], which is pure. This module is the part that
//! reads the store — which messages, their stored bytes — and the part that writes a file.
//! The window prints the same [`document`]: through its webview on `webview`, and as a PDF
//! made by quire on `native` (`ui/print/paper.rs`), which adds its named faces
//! ([`document_with`]) and, for a conversation whose remote images the reader consented to,
//! those images ([`Pictures`]).

use chrono::{DateTime, TimeZone, Utc};
use mail_domain::{Message, MessageId, ThreadId};
use mail_mime::{Options, Pages, Parsed, Remote, Sheet};
use mail_store::{SqliteStore, Store};
use std::collections::BTreeMap;
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
    of_messages_with(store, messages, zone, now, &Options::new(pages), None)
}

/// Where a printout's consented remote images come from.
///
/// A remote image is a read receipt, so this is only ever built from the reader's consent: which
/// messages it covers, and how to fetch what they show. [`mail_mime::remote_images`] lists what
/// the consented messages' printout would draw, and only that list is handed to `fetch`.
pub struct Pictures<'a> {
    /// Whether the reader's consent to remote images covers this message.
    pub consented: &'a dyn Fn(MessageId) -> bool,
    /// The images of `urls` that could be fetched, each as a `data:` URI of a raster image.
    /// Blocking.
    pub fetch: &'a dyn Fn(&[String]) -> BTreeMap<String, String>,
}

impl std::fmt::Debug for Pictures<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pictures").finish_non_exhaustive()
    }
}

/// [`of_messages`], made as `options` say, with the consented messages' remote images drawn
/// when `pictures` fetches them. Without `pictures` nothing is fetched and every remote image
/// is named, as [`of_messages`] does. Blocking: it reads the stored mail, and fetches.
pub fn of_messages_with<Tz>(
    store: &SqliteStore,
    messages: &[Message],
    zone: &Tz,
    now: DateTime<Utc>,
    options: &Options<'_>,
    pictures: Option<&Pictures<'_>>,
) -> Printed
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let parsed: Vec<Option<Parsed>> = messages
        .iter()
        .map(|message| parse_body(store, message))
        .collect();
    let consented: Vec<bool> = messages
        .iter()
        .map(|message| pictures.is_some_and(|pictures| (pictures.consented)(message.id)))
        .collect();
    let none = BTreeMap::new();
    let fetched = match pictures {
        Some(pictures) if consented.contains(&true) => {
            let wanted = mail_mime::remote_images(&sheets(messages, &parsed, &consented, &none));
            if wanted.is_empty() {
                none.clone()
            } else {
                (pictures.fetch)(&wanted)
            }
        }
        _ => none,
    };
    Printed {
        subject: messages
            .first()
            .map(|message| message.subject.clone())
            .unwrap_or_default(),
        html: mail_mime::print_with(
            &sheets(messages, &parsed, &consented, &fetched),
            zone,
            now,
            options,
        ),
    }
}

/// Each message as a sheet: a consented one's remote images drawn from `fetched`, every other
/// one's named.
fn sheets<'a>(
    messages: &'a [Message],
    parsed: &'a [Option<Parsed>],
    consented: &[bool],
    fetched: &'a BTreeMap<String, String>,
) -> Vec<Sheet<'a>> {
    messages
        .iter()
        .zip(parsed)
        .zip(consented)
        .map(|((message, parsed), consented)| Sheet {
            message,
            parsed: parsed.as_ref(),
            remote: if *consented {
                Remote::Allowed(fetched)
            } else {
                Remote::Blocked
            },
        })
        .collect()
}

/// [`document`], made as [`of_messages_with`] makes it.
pub fn document_with<Tz>(
    store: &SqliteStore,
    id: uuid::Uuid,
    zone: &Tz,
    now: DateTime<Utc>,
    options: &Options<'_>,
    pictures: Option<&Pictures<'_>>,
) -> Result<Printed, String>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    let messages = messages(store, id)?;
    Ok(of_messages_with(
        store, &messages, zone, now, options, pictures,
    ))
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
    file_name_with(subject, "html")
}

/// [`file_name`] with `extension` (`html`, `pdf`) on the end instead.
pub fn file_name_with(subject: &str, extension: &str) -> String {
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
    crate::attach::safe_name(&format!("{}.{extension}", short.trim_end()))
}

/// Write `printed` into `dir` under its subject's name, never over a file already there.
///
/// `create_new` rather than a check and then a write: the name is only free if claiming it
/// succeeds.
pub fn write_into(dir: &Path, printed: &Printed) -> Result<PathBuf, String> {
    write_file_into(dir, &printed.subject, "html", printed.html.as_bytes())
}

/// [`write_into`] for any printout of `subject`: `bytes`, in a file ending `.{extension}`.
pub fn write_file_into(
    dir: &Path,
    subject: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    use std::io::Write as _;
    let name = file_name_with(subject, extension);
    let path = Path::new(&name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("message");
    for n in 1..1000 {
        let candidate = if n == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem} ({n}).{extension}"))
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                file.write_all(bytes)
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

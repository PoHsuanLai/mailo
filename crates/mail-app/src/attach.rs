//! Attachments on a received message, and getting them back out.
//!
//! `mail-runtime` writes every attachment to its own blob at ingest and `Message::attachments`
//! has named them ever since. Nothing read that: no command listed them, no pane showed them,
//! and there was no way at all to get a file out of a message. A mail client people are sent
//! PDFs through is not one that can only display text.
//!
//! The dangerous part is the file name. It is chosen by whoever sent the message — `name` is a
//! MIME parameter, not a fact — so `../../../.ssh/authorized_keys` is a perfectly well-formed
//! attachment name and writing it where it asks is how a mail client hands someone else's
//! machine over. Everything written here goes through [`safe_name`].

use mail_domain::{Attachment, MessageId};
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The longest file name to write.
///
/// 255 bytes is the limit on ext4, btrfs, XFS and APFS alike. A name that does not fit is
/// truncated rather than refused: a file with a shortened name is useful, and an error is not.
const MAX_NAME: usize = 255;

/// What to call an attachment whose claimed name is unusable.
const FALLBACK: &str = "attachment";

/// A file name safe to write, derived from what the message claims.
///
/// The claim is the sender's, so this keeps only what a name may be: one path component, no
/// separators, no control characters, nothing that resolves upwards. The result is always a
/// non-empty single component that cannot escape the directory it is joined to.
///
/// Both separators are stripped, not just this platform's: a message written on Windows names
/// its parts with backslashes, and a Unix client that treats `..\\..\\evil` as one component
/// writes a file with a very strange name — harmless here, but the same code on Windows writes
/// it two directories up.
pub fn safe_name(claimed: &str) -> String {
    let last = claimed.rsplit(['/', '\\']).next().unwrap_or(claimed).trim();

    let cleaned: String = last
        .chars()
        // Control characters and NUL: a name containing a newline makes a terminal print
        // something other than what was written, and NUL truncates the name at the syscall.
        .filter(|c| !c.is_control())
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c.is_whitespace());

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return FALLBACK.to_owned();
    }
    truncate_keeping_extension(cleaned)
}

/// Shorten to [`MAX_NAME`] bytes, keeping the extension where there is one.
///
/// The extension is what decides which program opens the file, so losing it to a long name is
/// losing the point of saving it.
fn truncate_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME {
        return name.to_owned();
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() < 32)
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let room = MAX_NAME.saturating_sub(extension.len());
    let mut stem: String = String::new();
    for c in name.chars() {
        if stem.len() + c.len_utf8() > room {
            break;
        }
        stem.push(c);
    }
    format!("{stem}{extension}")
}

/// The attachments on a message, as the CLI prints them.
pub fn list(store: &SqliteStore, message: MessageId) -> Result<String, String> {
    let message = store.message(message).map_err(|e| e.to_string())?;
    if message.attachments.is_empty() {
        return Ok("no attachments on that message\n".to_owned());
    }
    let mut out = String::new();
    for (index, attachment) in message.attachments.iter().enumerate() {
        let _ = writeln!(
            out,
            "{index}  {:>9}  {:<24}  {}",
            human_size(attachment.size),
            attachment.mime,
            // What it will be written as, not what it claims: the difference is the whole
            // point, and a listing that shows the claim would mislead about what happens next.
            safe_name(&attachment.name)
        );
    }
    let _ = writeln!(
        out,
        "\nsave one with: mailo save <message-id> <number> [dir]"
    );
    Ok(out)
}

/// Where the window puts a file it has been asked to save.
///
/// `XDG_DOWNLOAD_DIR` when the desktop names one, `~/Downloads` when it does not, and the
/// working directory as a last resort. Deliberately not a file chooser: a dialog is a
/// dependency that pulls a toolkit in behind it, and every client that offers one also has a
/// default that most saves actually use. The path is reported afterwards, which is the part
/// that matters — a file saved somewhere the user cannot name is a file they have lost.
pub fn downloads_dir() -> PathBuf {
    downloads_from(
        std::env::var_os("XDG_DOWNLOAD_DIR").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// The same decision, with the environment as arguments.
///
/// Separated because the workspace forbids `unsafe`, and `std::env::set_var` has been unsafe
/// since the 2024 edition — so a test that wanted to check this rule could not set the variable
/// it is about. The same answer `plan.md` gives for the clock: the ambient thing becomes a
/// parameter, and the decision becomes a function anything can call.
pub fn downloads_from(named: Option<&std::ffi::OsStr>, home: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(named) = named {
        let path = PathBuf::from(named);
        // A relative setting would put the file wherever the process happens to have been
        // started, which for a desktop launcher is somewhere the user cannot guess.
        if path.is_absolute() {
            return path;
        }
    }
    match home {
        Some(home) => PathBuf::from(home).join("Downloads"),
        None => PathBuf::from("."),
    }
}

/// Write one attachment into `dir`, returning the path written.
///
/// Never overwrites. A message that arrives twice, or two messages with the same attachment
/// name, must not silently replace a file the user already has — so an existing name becomes
/// `report (2).pdf` rather than a lost file.
pub fn save(
    store: &SqliteStore,
    message: MessageId,
    index: usize,
    dir: &Path,
) -> Result<PathBuf, String> {
    let message = store.message(message).map_err(|e| e.to_string())?;
    let attachment: &Attachment = message.attachments.get(index).ok_or_else(|| {
        format!(
            "that message has {} attachment(s); there is no number {index}",
            message.attachments.len()
        )
    })?;

    let Some(blob) = attachment.blob() else {
        return Err(format!(
            "{} is still on the server; it downloads when opened",
            attachment.name
        ));
    };
    let bytes = store
        .blobs()
        .get(&store.connection(), blob)
        .map_err(|e| format!("cannot read the attachment: {e}"))?;

    write_new(dir, &attachment.name, &bytes)
}

/// One attachment of a message opened for reading — decrypted, or with its signature taken off —
/// whose bytes are in the opened message rather than in the store: the stored parts of an
/// encrypted message are ciphertext, and [`save`] would write that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedAttachment {
    /// As the message names it, untrusted: [`save_opened`] makes it safe.
    pub name: String,
    /// The declared media type, untrusted.
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Attachment `index` of an opened message, numbered as the reader lists an opened message's
/// attachments: in the order [`mail_mime::Parsed::attachments`] holds them, which is the order a
/// stored message's own attachments are numbered in too.
pub fn opened_attachment(
    shown: &mail_mime::Parsed,
    index: usize,
) -> Result<OpenedAttachment, String> {
    let part = shown.attachments.get(index).ok_or_else(|| {
        format!(
            "that message has {} attachment(s); there is no number {index}",
            shown.attachments.len()
        )
    })?;
    Ok(OpenedAttachment {
        name: part.name.clone(),
        mime: part.mime.clone(),
        bytes: part.bytes.clone(),
    })
}

/// Save an opened message's attachment into `dir`, as [`save`] saves a stored one: under its
/// name made safe, never over a file already there.
pub fn save_opened(attachment: &OpenedAttachment, dir: &Path) -> Result<PathBuf, String> {
    write_new(dir, &attachment.name, &attachment.bytes)
}

/// Write `bytes` into `dir` under `name` made safe, never over a file already there, returning
/// the path written. How every file the window saves reaches the disk.
pub fn write_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = free_path(dir, &safe_name(name));
    std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// [`save`], downloading the attachment first if a sync left it on the server.
///
/// `download` fetches one section of `message` and records it held; the application passes
/// [`crate::sync::fetch_part`]. Taken as a parameter so a test can check "remote, then saved"
/// without a server.
///
/// Blocking and network-bound for a part that is still remote, so the UI calls it off the
/// render thread. The answer is a sentence, shown to the user as it is.
pub fn fetch_and_save(
    store: &SqliteStore,
    message: MessageId,
    index: usize,
    dir: &Path,
    download: impl FnOnce(&str) -> Result<(), String>,
) -> Result<String, String> {
    let stored = store.message(message).map_err(|e| e.to_string())?;
    let attachment = stored.attachments.get(index).ok_or_else(|| {
        format!(
            "that message has {} attachment(s); there is no number {index}",
            stored.attachments.len()
        )
    })?;
    if let mail_domain::PartContent::Remote { section } = &attachment.content {
        download(section)?;
    }
    let path = save(store, message, index, dir)?;
    Ok(format!("Saved to {}", path.display()))
}

/// A path in `dir` that nothing is using yet: `name`, or `name (2)` and so on. A file and a
/// directory are the same kind of obstacle, so an export directory is named by it too.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    for n in 2..1000 {
        let candidate = dir.join(format!("{stem} ({n}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // A thousand copies of one name. Writing over the first is still wrong, so this is the one
    // case that lands on a name with the process id in it rather than giving up.
    dir.join(format!("{stem} ({}){extension}", std::process::id()))
}

/// A size a person can read.
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    match bytes {
        0..KB => format!("{bytes} B"),
        KB..MB => format!("{:.1} kB", bytes as f64 / KB as f64),
        _ => format!("{:.1} MB", bytes as f64 / MB as f64),
    }
}

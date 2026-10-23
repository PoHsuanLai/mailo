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

use crate::error::CoreError;
use mail_domain::{Attachment, MessageId};
use mail_store::{SqliteStore, Store};
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
        // What Windows refuses in a name, `:` among them, which names a stream of another file.
        .map(|c| if WINDOWS_REFUSES.contains(c) { '_' } else { c })
        .collect();
    // Windows drops a trailing dot or space, so `report.pdf.` would be written as `report.pdf`.
    let cleaned = cleaned
        .trim_start_matches(char::is_whitespace)
        .trim_end_matches(|c: char| c.is_whitespace() || c == '.');

    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return FALLBACK.to_owned();
    }
    if names_a_device(cleaned) {
        return truncate_keeping_extension(&format!("_{cleaned}"));
    }
    truncate_keeping_extension(cleaned)
}

/// The characters Windows refuses in a file name.
const WINDOWS_REFUSES: &str = "<>:\"|?*";

/// Whether Windows reads `name` as a device rather than a file: `CON`, `NUL`, `COM1` and the
/// rest, whatever follows a dot.
fn names_a_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    let stem = stem.to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|kind| {
            stem.strip_prefix(kind)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
        })
}

/// Whether a file called `name` is a program or a script, which runs rather than shows when the
/// system opens it: the window shows such a file in its folder instead of opening it.
pub fn runs_when_opened(name: &str) -> bool {
    const KINDS: &[&str] = &[
        "app",
        "action",
        "applescript",
        "bash",
        "bat",
        "bin",
        "cmd",
        "com",
        "command",
        "cpl",
        "csh",
        "desktop",
        "exe",
        "hta",
        "jar",
        "js",
        "jse",
        "ksh",
        "lnk",
        "msi",
        "pif",
        "pkg",
        "mpkg",
        "pl",
        "ps1",
        "py",
        "rb",
        "reg",
        "run",
        "scpt",
        "scr",
        "sh",
        "terminal",
        "tool",
        "vb",
        "vbe",
        "vbs",
        "workflow",
        "wsf",
        "wsh",
        "zsh",
        "appimage",
    ];
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| KINDS.contains(&e.to_ascii_lowercase().as_str()))
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

/// One attachment of a received message, as [`list`] shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// What it will be written as, not what it claims: the difference is the whole point, and a
    /// listing that showed the claim would mislead about what happens next.
    pub name: String,
    /// The declared media type, untrusted.
    pub mime: String,
    pub size: u64,
}

/// The attachments on a message, in the order [`save`] numbers them.
pub fn list(store: &SqliteStore, message: MessageId) -> Result<Vec<Listed>, CoreError> {
    let message = store.message(message)?;
    Ok(message
        .attachments
        .iter()
        .map(|attachment| Listed {
            name: safe_name(&attachment.name),
            mime: attachment.mime.clone(),
            size: attachment.size,
        })
        .collect())
}

/// Where the window puts a file it has been asked to save.
///
/// The folder the desktop names (`XDG_DOWNLOAD_DIR` on Linux, the system's Downloads folder on
/// macOS and Windows), `~/Downloads` when it names none, and the working directory as a last
/// resort. Deliberately not a file chooser: a dialog is a
/// dependency that pulls a toolkit in behind it, and every client that offers one also has a
/// default that most saves actually use. The path is reported afterwards, which is the part
/// that matters — a file saved somewhere the user cannot name is a file they have lost.
pub fn downloads_dir() -> PathBuf {
    downloads_from(
        mail_runtime::places::downloads_named().as_deref(),
        mail_runtime::places::home()
            .as_deref()
            .map(std::path::Path::as_os_str),
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
) -> Result<PathBuf, CoreError> {
    let message = store.message(message)?;
    let attachment: &Attachment =
        message
            .attachments
            .get(index)
            .ok_or_else(|| CoreError::NoAttachment {
                have: message.attachments.len(),
                index,
            })?;

    let Some(blob) = attachment.blob() else {
        return Err(CoreError::AttachmentOnServer {
            name: attachment.name.clone(),
        });
    };
    let bytes = store
        .blobs()
        .get(blob)
        .map_err(|e| CoreError::cannot("read the attachment", e))?;

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
) -> Result<OpenedAttachment, CoreError> {
    let part = shown
        .attachments
        .get(index)
        .ok_or_else(|| CoreError::NoAttachment {
            have: shown.attachments.len(),
            index,
        })?;
    Ok(OpenedAttachment {
        name: part.name.clone(),
        mime: part.mime.clone(),
        bytes: part.bytes.clone(),
    })
}

/// Save an opened message's attachment into `dir`, as [`save`] saves a stored one: under its
/// name made safe, never over a file already there.
pub fn save_opened(attachment: &OpenedAttachment, dir: &Path) -> Result<PathBuf, CoreError> {
    write_new(dir, &attachment.name, &attachment.bytes)
}

/// Write `bytes` into `dir` under `name` made safe, never over a file already there, returning
/// the path written. How every file the window saves reaches the disk.
pub fn write_new(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, CoreError> {
    create_unique(dir, &safe_name(name), bytes)
}

/// Write `bytes` into `dir` as `name`, or `name (2)` and so on, returning the path written.
///
/// A name is free only if claiming it succeeds (`create_new`), so neither a file written in the
/// meantime nor a link left in its place is written through. Only its owner can read the file,
/// and on macOS it is marked as come from elsewhere ([`mail_runtime::quarantine`]).
pub(crate) fn create_unique(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, CoreError> {
    use std::io::Write as _;
    std::fs::create_dir_all(dir).map_err(CoreError::at(dir))?;
    for candidate in candidates(name).take(1000) {
        let path = dir.join(&candidate);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(CoreError::at(&path))?;
                // A file that could not be marked is still the file asked for.
                let _ = mail_runtime::quarantine::mark_now(&path, AGENT);
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(CoreError::at(&path)(e)),
        }
    }
    Err(CoreError::NamesTaken {
        name: name.to_owned(),
        dir: dir.to_owned(),
    })
}

/// Who wrote a marked file, as the quarantine mark names it.
const AGENT: &str = "mailo";

/// `name`, then `name (2)`, `name (3)` and on, the number before the extension.
fn candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let path = Path::new(name);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or(name);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    std::iter::once(name.to_owned()).chain((2..).map(move |n| format!("{stem} ({n}){extension}")))
}

/// [`save`], downloading the attachment first if a sync left it on the server.
///
/// `download` fetches one section of `message` and records it held; the application passes
/// [`crate::SyncOps::fetch_part`]. Taken as a parameter so a test can check "remote, then saved"
/// without a server.
///
/// Blocking and network-bound for a part that is still remote, so the UI calls it off the
/// render thread. Returns the path written.
pub fn fetch_and_save(
    store: &SqliteStore,
    message: MessageId,
    index: usize,
    dir: &Path,
    download: impl FnOnce(&mail_domain::Section) -> Result<(), CoreError>,
) -> Result<PathBuf, CoreError> {
    let stored = store.message(message)?;
    let attachment = stored
        .attachments
        .get(index)
        .ok_or_else(|| CoreError::NoAttachment {
            have: stored.attachments.len(),
            index,
        })?;
    if let mail_domain::PartContent::Remote { section } = &attachment.content {
        download(section)?;
    }
    save(store, message, index, dir)
}

/// A name in `dir` that nothing is using yet, to suggest: `name`, or `name (2)` and so on. A file
/// and a directory are the same kind of obstacle, so an export directory is named by it too.
/// Only a suggestion: whatever writes there claims it with `create_new`, as [`write_new`] does.
pub fn free_path(dir: &Path, name: &str) -> PathBuf {
    candidates(name)
        .take(1000)
        .map(|candidate| dir.join(candidate))
        .find(|path| !path.exists())
        .unwrap_or_else(|| dir.join(name))
}

pub use mail_domain::human_size;

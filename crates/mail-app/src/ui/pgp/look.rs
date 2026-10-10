//! Opening a message the reader shows, in the window's words.
//!
//! A message is asked of OpenPGP first and of S/MIME when OpenPGP finds none, and what was found
//! is kept per message and body: all of that is [`mail_core::message::Looks`]'s. This module turns
//! what it found into what the reader draws: the lines said about the protection, and the
//! attachments inside an opened message.

use std::path::{Path, PathBuf};

use chrono::Utc;
use mail_core::SigningStore;
use mail_core::SqliteStore;
use mail_core::message::{Looks, Opening, Protection, Sealed};
use mail_domain::*;
use mail_mime::Parsed;

use super::super::text::{AttachmentRow, Kept as Where};
use super::{Said, Scheme, Tried, said, said_smime};
use mail_core::password::Password;

/// A message opened: what to say about it, and the body to show in place of the stored one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Opened {
    /// Which protection it carries.
    pub scheme: Scheme,
    pub said: Vec<Said>,
    /// The decrypted or unwrapped message, parsed. `None` when there is nothing to show but
    /// what is stored: it could not be read.
    pub shown: Option<Box<Parsed>>,
}

/// What opening a message found, as the reader keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Look {
    /// It carries no OpenPGP or S/MIME, or only its headers are here.
    Plain,
    Opened(Box<Opened>),
    /// Encrypted to the user's OpenPGP key `key`, whose passphrase is needed.
    Locked {
        key: Fingerprint,
        tried: Tried,
    },
    /// Opening it failed, in words.
    Failed(String),
}

/// What the reader draws for `opening`.
fn look_of(opening: Opening) -> Look {
    match opening {
        Opening::Plain => Look::Plain,
        Opening::Locked { key, tried } => Look::Locked { key, tried },
        Opening::Failed { scheme, why } => Look::Failed(format!(
            "Can\u{2019}t open {}: {why}",
            scheme_of(scheme).name()
        )),
        Opening::Opened(sealed) => {
            let Sealed { protection, shown } = *sealed;
            let (scheme, said) = match protection {
                Protection::OpenPgp(protected) => (Scheme::OpenPgp, said(&protected)),
                Protection::Smime(protected) => (Scheme::Smime, said_smime(&protected)),
            };
            Look::Opened(Box::new(Opened {
                scheme,
                said,
                shown,
            }))
        }
    }
}

fn scheme_of(scheme: mail_core::message::Scheme) -> Scheme {
    match scheme {
        mail_core::message::Scheme::OpenPgp => Scheme::OpenPgp,
        mail_core::message::Scheme::Smime => Scheme::Smime,
    }
}

/// What was found for `message` holding `body`, if it was opened under the keys held now.
pub(in crate::ui) fn cached(
    looks: &Looks,
    message: MessageId,
    body: Option<BlobId>,
) -> Option<Look> {
    looks.sealed_cached(message, body).map(look_of)
}

/// [`cached`], else the message opened with no passphrase, remembered. This decrypts and may
/// read the keyring: it runs on a blocking thread, for a message the reader has open, and
/// nowhere else.
pub(in crate::ui) fn lookup(
    looks: &Looks,
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    message: MessageId,
    body: Option<BlobId>,
) -> Look {
    look_of(looks.sealed(store, secrets, message, body, Utc::now()))
}

/// Open `message` again with the passphrase typed for it, and keep what that found. The
/// passphrase is dropped when this returns. Runs on a blocking thread, and only from Unlock.
pub(in crate::ui) fn unlock(
    looks: &Looks,
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    message: MessageId,
    body: Option<BlobId>,
    passphrase: Password,
) -> Look {
    look_of(looks.unlocked(store, secrets, message, body, passphrase, Utc::now()))
}

/// The attachments to list for `message` when it was opened to a body of its own: those inside
/// it. Its stored parts are then its wrapping — the signature, the ciphertext — and not
/// attachments anyone sent. `None` lists what is stored.
pub(in crate::ui) fn attachments(looks: &Looks, message: &Message) -> Option<Vec<AttachmentRow>> {
    let parsed = looks.opened_body(message)?;
    Some(
        parsed
            .attachments
            .iter()
            .enumerate()
            .map(|(index, part)| AttachmentRow {
                index,
                name: mail_core::attach::safe_name(&part.name),
                size: mail_core::attach::human_size(part.bytes.len() as u64),
                kept: Where::Opened,
            })
            .collect(),
    )
}

/// Save attachment `index` of what `message` holding `body` was opened to into `dir`, numbered
/// as [`attachments`] lists them. Writes a file: run it off the thread that draws.
pub(in crate::ui) fn save_attachment(
    looks: &Looks,
    message: MessageId,
    body: Option<BlobId>,
    index: usize,
    dir: &Path,
) -> Result<PathBuf, String> {
    let Some(Opening::Opened(opened)) = looks.sealed_last(message, body) else {
        return Err("The message is closed. Open it again.".to_owned());
    };
    let shown = opened.shown.ok_or("Can\u{2019}t read the attachments.")?;
    let attachment = mail_core::attach::opened_attachment(&shown, index)?;
    mail_core::attach::save_opened(&attachment, dir).map_err(String::from)
}

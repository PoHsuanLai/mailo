//! OpenPGP and S/MIME in the window: a protected message opened in the reader and said in plain
//! words, the passphrase asked for inline, and the keys and certificates sheet.
//!
//! What a message is — signed, encrypted, by whom — is [`crate::pgp`]'s and [`crate::smime`]'s,
//! the modules `mailo show`, `mailo pgp` and `mailo smime` use, so the window and the command
//! cannot disagree. Opening decrypts, so it is slow and may need the keyring: it runs on a
//! blocking thread, for a message the reader has open and nowhere else — [`look`] is reached
//! from [`seal::Seal`] and [`unlock`] only, and a test counts its calls — and what it found is
//! kept per message and body, like the receipt bar's and the invitation card's. The decrypted
//! body lives in that cache and nowhere else: it is shown in place of the stored one, through
//! the same blocks, and never written down.
//!
//! A passphrase typed to unlock a key, or a PKCS#12 file's password, is a
//! [`Password`](crate::password::Password): moved from the field into the one call that needs
//! it, and dropped with it. It is never in a signal, the [`Shell`](crate::view::Shell), the
//! store, a file or a log.

mod certs;
mod key_row;
pub(in crate::ui) mod keys;
mod look;
mod said;
mod seal;
mod unlock;

pub(in crate::ui) use look::{
    Look, attachments, cached, lookup, reading, save_attachment, subject, unlock,
};
#[cfg(test)]
pub(in crate::ui) use look::{looked_at, looks_at};
pub(in crate::ui) use said::{Said, said, said_smime, whose};
#[cfg(test)]
pub(in crate::ui) use said::{Tone, doubt};
pub(in crate::ui) use seal::Seal;
pub(in crate::ui) use unlock::Unlock;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use mail_domain::*;
use mail_runtime::Secrets;
use mail_store::SqliteStore;

/// Look a key up by its address's domain. Blocks on the network: run it off the thread that draws.
pub(in crate::ui) type Lookup =
    dyn Fn(&SqliteStore, &str) -> Result<Option<PgpKey>, String> + Send + Sync;
/// Ask for a file to read. Blocks until the dialog is answered.
pub(in crate::ui) type PickFile = dyn Fn() -> Option<PathBuf> + Send + Sync;
/// Ask where to write a file, suggesting a name. Blocks until the dialog is answered.
pub(in crate::ui) type SaveFile = dyn Fn(&str) -> Option<PathBuf> + Send + Sync;

/// The window's reach into the world for keys and certificates: the keyring, the Web Key
/// Directory and the file dialogs, handed in as a context so tests reach none of them.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub secrets: Arc<dyn Secrets>,
    pub lookup: Arc<Lookup>,
    pub pick: Arc<PickFile>,
    pub save: Arc<SaveFile>,
}

impl Seams {
    /// The system keyring, the network and the desktop's dialogs. In this crate's tests, an
    /// empty keyring of its own and nothing else: a window drawn by a test has no business with
    /// the user's keys, the network or a dialog.
    pub(in crate::ui) fn real() -> Seams {
        if cfg!(test) {
            return Seams {
                secrets: Arc::new(mail_runtime::MapSecrets::default()),
                lookup: Arc::new(|_, _| Err("no lookups in tests".to_owned())),
                pick: Arc::new(|| None),
                save: Arc::new(|_| None),
            };
        }
        Seams {
            secrets: Arc::new(mail_runtime::KeyringSecrets),
            lookup: Arc::new(|store, address| {
                crate::pgp::lookup_address(store, address, Utc::now()).map_err(|e| e.to_string())
            }),
            pick: Arc::new(|| rfd::FileDialog::new().pick_file()),
            save: Arc::new(|name| rfd::FileDialog::new().set_file_name(name).save_file()),
        }
    }
}

/// The seams the window was given, else the real ones.
pub(in crate::ui) fn seams() -> Seams {
    dioxus::prelude::try_consume_context::<Seams>().unwrap_or_else(Seams::real)
}

/// Whether a passphrase was given, when a key stayed locked. `ask` is only called for a
/// protected key, so a key still locked after one was given was given the wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Tried {
    /// None was given: it needs one.
    Nothing,
    /// One was given, and it did not unlock the key.
    Wrong,
}

/// Whether something started by a press is still running, so a second press starts nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Busy {
    Idle,
    Working,
}

/// Which protection a message carries, or a draft asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Scheme {
    OpenPgp,
    Smime,
}

impl Scheme {
    pub(in crate::ui) fn name(self) -> &'static str {
        match self {
            Scheme::OpenPgp => "OpenPGP",
            Scheme::Smime => "S/MIME",
        }
    }
}

/// A key's short name: its key id, in the groups of four people read fingerprints aloud in.
pub(in crate::ui) fn short(fingerprint: Fingerprint) -> String {
    grouped(&fingerprint.key_id().to_string())
}

/// A certificate's short name: the first eight bytes of its fingerprint, grouped the same way.
pub(in crate::ui) fn cert_short(fingerprint: CertFingerprint) -> String {
    grouped(&fingerprint.to_string()[..16])
}

fn grouped(hex: &str) -> String {
    hex.as_bytes()
        .chunks(4)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Who a key is for, as its first user id says, else its first address.
pub(in crate::ui) fn who(key: &PgpKey) -> String {
    key.user_ids
        .first()
        .or(key.emails.first())
        .cloned()
        .unwrap_or_else(|| "a key with no name".to_owned())
}

#[cfg(test)]
mod certs_tests;
#[cfg(test)]
mod keys_tests;
#[cfg(test)]
mod look_tests;
#[cfg(test)]
mod smime_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;

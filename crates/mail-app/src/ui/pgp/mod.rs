//! OpenPGP and S/MIME in the window: a protected message opened in the reader and said in plain
//! words, the passphrase asked for inline, and the keys and certificates Settings lists: an
//! account's own on its page, every other on Other Keys under Accounts.
//!
//! What a message is — signed, encrypted, by whom — is [`mail_core::pgp`]'s and [`mail_core::smime`]'s,
//! the modules `mailo show`, `mailo pgp` and `mailo smime` use, so the window and the command
//! cannot disagree. Opening decrypts, so it is slow and may need the keyring: it runs on a
//! blocking thread, for a message the reader has open and nowhere else — it is reached from
//! [`seal::Seal`] and [`unlock`] only, and a test counts the reads of the app's
//! [`Looks`](mail_core::message::Looks) — and what it found is kept per message and body there,
//! like the receipt bar's and the invitation card's. The decrypted body lives in that cache and
//! nowhere else: it is shown in place of the stored one, through
//! the same blocks, and never written down.
//!
//! A passphrase typed to unlock a key, or a PKCS#12 file's password, is a
//! [`Password`](mail_core::password::Password): moved from the field into the one call that needs
//! it, and dropped with it. It is never in a signal, the [`Shell`](crate::ui::view::Shell), the
//! client, a file or a log.

mod actions;
mod certs;
mod key_row;
pub(in crate::ui) mod keys;
mod keys_page;
mod look;
mod said;
mod seal;
mod unlock;

#[cfg(test)]
pub(in crate::ui) use look::Opened;
pub(in crate::ui) use look::{
    Look, Mark, attachments, cached, last, lookup, save_attachment, unlock,
};
#[cfg(test)]
pub(in crate::ui) use said::doubt;
pub(in crate::ui) use said::{Said, Tone, said, said_smime, whose};
pub(in crate::ui) use seal::Seal;
pub(in crate::ui) use unlock::{Passphrase, Unlock, WRONG};

use std::path::PathBuf;
use std::sync::Arc;

use mail_client::Client;
use mail_core::SigningStore;
use mail_domain::*;

/// Look a key up by its address's domain. Blocks on the network: run it off the thread that draws.
pub(in crate::ui) type Lookup =
    dyn Fn(&Client, &str) -> Result<Option<PgpKey>, String> + Send + Sync;
/// Ask for a file to read. Blocks until the dialog is answered.
pub(in crate::ui) type PickFile = dyn Fn() -> Option<PathBuf> + Send + Sync;
/// Ask where to write a file, suggesting a name. Blocks until the dialog is answered.
pub(in crate::ui) type SaveFile = dyn Fn(&str) -> Option<PathBuf> + Send + Sync;

/// The window's reach into the world for keys and certificates: the keyring, the Web Key
/// Directory and the file dialogs, handed in as a context so tests reach none of them.
#[derive(Clone)]
pub(in crate::ui) struct Seams {
    pub secrets: Arc<dyn SigningStore>,
    pub lookup: Arc<Lookup>,
    pub pick: Arc<PickFile>,
    pub save: Arc<SaveFile>,
}

impl Seams {
    /// The system keyring, the network and the desktop's dialogs: the launched window's, handed
    /// as a root context there (`launch::native`) and nowhere else.
    pub(in crate::ui) fn real() -> Seams {
        Seams {
            secrets: crate::edge::signing(),
            lookup: Arc::new(|client, address| {
                client
                    .net()
                    .lookup_key(address, chrono::Utc::now())
                    .map_err(|e| e.to_string())
            }),
            pick: Arc::new(|| rfd::FileDialog::new().pick_file()),
            save: Arc::new(|name| rfd::FileDialog::new().set_file_name(name).save_file()),
        }
    }

    /// A window's whose root handed none, as a test's does: a keyring that holds nothing, no
    /// lookups and no dialogs. A window drawn by a test has no business with the user's keys, the
    /// network or a dialog.
    fn none() -> Seams {
        Seams {
            secrets: Arc::new(mail_core::NoKeyring),
            lookup: Arc::new(|_, _| Err("no lookups in tests".to_owned())),
            pick: Arc::new(|| None),
            save: Arc::new(|_| None),
        }
    }
}

/// The seams the window's root handed, else ones that reach nothing.
pub(in crate::ui) fn seams() -> Seams {
    dioxus::prelude::try_consume_context::<Seams>().unwrap_or_else(Seams::none)
}

pub(in crate::ui) use mail_core::message::Tried;

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
pub(in crate::ui) mod certs_tests;
#[cfg(test)]
pub(in crate::ui) mod keys_tests;
#[cfg(test)]
mod look_tests;
#[cfg(test)]
mod smime_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;

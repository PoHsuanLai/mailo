//! Opening a message the reader shows: OpenPGP first, S/MIME when OpenPGP finds none, and what
//! that found, kept per message and body.
//!
//! What was found is kept with the count of key changes it was found under
//! ([`crate::pgp::epoch`], which S/MIME's moves too): once the keys or certificates change —
//! from the sheet, the command line, or a certificate learnt from arriving mail — the next
//! opening asks again, and nothing has to remember to clear it. The decrypted body lives in that
//! memory and nowhere else; it is never written down.
//!
//! The answers are typed: which protection a message carries, what was checked, the body it was
//! opened to. The words for them are the front-end's.

use super::looks::{Looks, Read, held};
use crate::SigningStore;
use crate::password::Password;
use crate::pgp::Ask;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::Parsed;
use mail_store::{SqliteStore, Store};

/// Which protection a message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    OpenPgp,
    Smime,
}

/// Whether a passphrase was given, when a key stayed locked. `ask` is only called for a
/// protected key, so a key still locked after one was given was given the wrong one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tried {
    /// None was given: it needs one.
    Nothing,
    /// One was given, and it did not unlock the key.
    Wrong,
}

/// What was checked about a message, by the protection it carries. The opened body and the bytes
/// it was parsed from are not in here: they are [`Sealed::shown`], once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Protection {
    OpenPgp(Box<crate::pgp::Protected>),
    Smime(Box<crate::smime::Protected>),
}

/// A message opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub protection: Protection,
    /// The decrypted or unwrapped message, parsed. `None` when there is nothing to show but what
    /// is stored: it could not be read.
    pub shown: Option<Box<Parsed>>,
}

/// What opening a message found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opening {
    /// It carries no OpenPGP or S/MIME, or only its headers are here.
    Plain,
    Opened(Box<Sealed>),
    /// Encrypted to the user's OpenPGP key `key`, whose passphrase is needed.
    Locked {
        key: Fingerprint,
        tried: Tried,
    },
    /// Opening it failed: `why` is the error as the protocol's module words it.
    Failed {
        scheme: Scheme,
        why: String,
    },
}

/// One message's opening, and the count of key changes it was found under.
#[derive(Debug)]
pub(super) struct Kept {
    body: Option<BlobId>,
    epoch: u64,
    opening: Opening,
}

/// How many messages' openings to keep. A decrypted message is its whole body, so few.
pub(super) const KEPT: usize = 32;

impl Looks {
    /// What was found for `message` holding `body`, if it was opened under the keys held now.
    pub fn sealed_cached(&self, message: MessageId, body: Option<BlobId>) -> Option<Opening> {
        let now = crate::pgp::epoch();
        held(&self.seals)
            .get(&message)
            .filter(|kept| kept.body == body && kept.epoch == now)
            .map(|kept| kept.opening.clone())
    }

    /// What was last found for `message` holding `body`, under whichever keys. What the reader
    /// draws until the message is opened again: the body and the words stay together.
    pub fn sealed_last(&self, message: MessageId, body: Option<BlobId>) -> Option<Opening> {
        held(&self.seals)
            .get(&message)
            .filter(|kept| kept.body == body)
            .map(|kept| kept.opening.clone())
    }

    /// [`sealed_cached`](Self::sealed_cached), else the message opened with no passphrase, and
    /// remembered. This decrypts and may read the keyring: run it on a blocking thread, for a
    /// message the reader has open, and nowhere else.
    pub fn sealed(
        &self,
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        message: MessageId,
        body: Option<BlobId>,
        now: DateTime<Utc>,
    ) -> Opening {
        if let Some(had) = self.sealed_cached(message, body) {
            return had;
        }
        let found = self.open(
            store,
            secrets,
            message,
            &crate::pgp::no_passphrase,
            Tried::Nothing,
            now,
        );
        self.keep_sealed(message, body, &found);
        found
    }

    /// Open `message` again with the passphrase typed for it, and keep what that found. The
    /// passphrase is dropped when this returns. Run it on a blocking thread, and only for an
    /// Unlock the person pressed.
    pub fn unlocked(
        &self,
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        message: MessageId,
        body: Option<BlobId>,
        passphrase: Password,
        now: DateTime<Utc>,
    ) -> Opening {
        let ask = |_: Fingerprint| Some(passphrase.expose().to_owned());
        let found = self.open(store, secrets, message, &ask, Tried::Wrong, now);
        drop(passphrase);
        self.keep_sealed(message, body, &found);
        found
    }

    /// The body `message` was opened to, when it has one of its own. No decryption here: only
    /// what [`sealed`](Self::sealed) already found.
    pub fn opened_body(&self, message: &Message) -> Option<Box<Parsed>> {
        match self.sealed_last(message.id, message.body.raw())? {
            Opening::Opened(sealed) => sealed.shown,
            _ => None,
        }
    }

    /// The subject `message` was opened to, when that differs from the stored one: an encrypted
    /// message's real subject travels inside it, and the outside says `...`.
    pub fn opened_subject(&self, message: &Message) -> Option<String> {
        self.opened_body(message)
            .map(|parsed| parsed.subject)
            .filter(|subject| !subject.trim().is_empty() && *subject != message.subject)
    }

    fn open(
        &self,
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        message: MessageId,
        ask: Ask<'_>,
        tried: Tried,
        now: DateTime<Utc>,
    ) -> Opening {
        self.count(Read::Seal);
        let Ok(stored) = store.message(message) else {
            return Opening::Plain;
        };
        match crate::pgp::open_message(store, secrets, &stored, ask, now) {
            Ok(None) => {}
            Ok(Some(mut protected)) => {
                return match protected.encryption {
                    Encryption::Locked { key } => Opening::Locked { key, tried },
                    _ => {
                        let shown = protected.shown.take().map(Box::new);
                        protected.raw = None;
                        Opening::Opened(Box::new(Sealed {
                            protection: Protection::OpenPgp(Box::new(protected)),
                            shown,
                        }))
                    }
                };
            }
            Err(e) => {
                return Opening::Failed {
                    scheme: Scheme::OpenPgp,
                    why: e.to_string(),
                };
            }
        }
        match crate::smime::open_message(store, secrets, &stored, now) {
            Ok(None) => Opening::Plain,
            Ok(Some(mut protected)) => {
                let shown = protected.shown.take().map(Box::new);
                protected.raw = None;
                Opening::Opened(Box::new(Sealed {
                    protection: Protection::Smime(Box::new(protected)),
                    shown,
                }))
            }
            Err(e) => Opening::Failed {
                scheme: Scheme::Smime,
                why: e.to_string(),
            },
        }
    }

    /// Keep `opening`, under the count as it stands once the opening is done: opening a signed
    /// message may itself teach a certificate, and what it found already knows it.
    fn keep_sealed(&self, message: MessageId, body: Option<BlobId>, opening: &Opening) {
        let epoch = crate::pgp::epoch();
        held(&self.seals).put(
            message,
            Kept {
                body,
                epoch,
                opening: opening.clone(),
            },
        );
    }
}

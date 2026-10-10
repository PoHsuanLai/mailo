//! OpenPGP from the user's side: keys, reading protected mail, and sending it.
//!
//! The pieces: [`keys`] makes, imports, exports and forgets keys; [`read`] opens a message when
//! the reader shows it; [`send`] seals an outgoing one as Send is pressed. The cryptography is
//! `mail_mime::openpgp`'s, the keyring and the Web Key Directory are `mail_runtime`'s, and this
//! module decides which keys to use.

pub mod keys;
pub mod read;
pub mod send;

pub use keys::{Imported, WithSecret, own_key};
pub use read::{Protected, open_bytes, open_message};
pub use send::{check, outgoing};

use chrono::{DateTime, Utc};
use mail_domain::{Fingerprint, KeySource, PgpKey};
use mail_mime::MimeError;
use mail_runtime::RuntimeError;
use mail_store::{SqliteStore, Store, StoreError};

/// Asked for the passphrase of the user's key with this fingerprint, when a protected key has
/// to sign or decrypt. `None` is "not given": the send is refused or the message stays locked.
///
/// The command line asks on the terminal; the window asks in a dialog; a test
/// answers from a table. Nothing is asked for a key that has no passphrase.
pub type Ask<'a> = &'a dyn Fn(Fingerprint) -> Option<String>;

/// The [`Ask`] that never has an answer: for callers with nobody to ask.
pub fn no_passphrase(_: Fingerprint) -> Option<String> {
    None
}

/// Why an OpenPGP step could not be done, named so the caller can say what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum PgpError {
    /// Encryption was asked for and these recipients have no key to encrypt to.
    #[error(
        "no OpenPGP key for {}; nothing was sent. Find or import one, or send without encrypting",
        .0.join(", ")
    )]
    NoKeyFor(Vec<String>),
    /// Encryption was asked for with blind recipients, whose key ids every recipient would see.
    #[error(
        "an encrypted message cannot have Bcc recipients ({}): every recipient would see the \
         blind ones' key ids. Send them a separate message",
        .0.join(", ")
    )]
    BlindRecipients(Vec<String>),
    /// Signing or encrypting was asked for from an identity with no key of its own.
    #[error("{0} has no OpenPGP key")]
    NoOwnKey(String),
    /// A protected key's passphrase was not given, or was wrong.
    #[error("the OpenPGP key {0} needs its passphrase; none was given, or it was wrong")]
    Locked(Fingerprint),
    #[error("{0} is not the address of any of your identities")]
    NoIdentity(String),
    /// A draft asks for OpenPGP and S/MIME at once (`crate::smime::send`).
    #[error(
        "a message is protected with OpenPGP or with S/MIME, not both; choose one before sending"
    )]
    BothProtections,
    #[error("{address} already has an OpenPGP key, {fingerprint}")]
    AlreadyHasKey {
        address: String,
        fingerprint: Fingerprint,
    },
    #[error(
        "the secret key {fingerprint} is for {}, none of which is one of your identities",
        addresses.join(", ")
    )]
    NotYours {
        fingerprint: Fingerprint,
        addresses: Vec<String>,
    },
    #[error("no OpenPGP key {0}")]
    NoKey(String),
    #[error("the secret half of {0} is not in your keyring")]
    NoSecret(Fingerprint),
    #[error(
        "the secret half of {0} is in your keyring, and deleting it means mail encrypted to it \
         can never be read again. Export it first, then delete it together with its secret half"
    )]
    SecretWouldBeLost(Fingerprint),
    #[error("{0}")]
    Mime(MimeError),
    #[error("{0}")]
    Runtime(#[from] RuntimeError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

impl PgpError {
    /// The step that mends this, when there is one: the front end words it.
    pub fn remedy(&self) -> Option<crate::Remedy> {
        use crate::Remedy;
        match self {
            PgpError::NoKeyFor(_) => Some(Remedy::FindPgpKey),
            PgpError::NoOwnKey(address) => Some(Remedy::MakePgpKey {
                address: address.clone(),
            }),
            PgpError::SecretWouldBeLost(fingerprint) => Some(Remedy::ExportPgpSecret {
                fingerprint: *fingerprint,
            }),
            _ => None,
        }
    }
}

impl From<MimeError> for PgpError {
    fn from(e: MimeError) -> Self {
        match e {
            MimeError::KeyLocked(fingerprint) => PgpError::Locked(fingerprint),
            other => PgpError::Mime(other),
        }
    }
}

/// A count that moves whenever the keys or certificates this process holds or trusts change —
/// imported, made, deleted, trusted, or learnt from arriving mail. Shared by OpenPGP and S/MIME
/// (`mail_runtime::epoch`): a cache of opened messages keeps the count it was made under and
/// opens them again when it has moved.
pub fn epoch() -> u64 {
    mail_runtime::epoch::keys()
}

fn epoch_changed() {
    mail_runtime::epoch::keys_changed();
}

impl crate::mail::CryptoOps<'_> {
    /// Ask `address`'s domain for its key, and keep it when there is one: the key, or `None`.
    pub async fn lookup_address(&self, address: &str) -> Result<Option<PgpKey>, PgpError> {
        lookup_address(self.0.store(), address, self.0.now()).await
    }

    /// The To and Cc addresses of an encrypted draft that have no key yet, each asked of its
    /// domain's Web Key Directory.
    pub async fn discover(&self, addresses: &[String]) -> Vec<Discovered> {
        discover(self.0.store(), addresses, self.0.now()).await
    }
}

/// Ask `address`'s domain for its key, and keep it when there is one: the key, or `None`. Needs
/// the network.
pub async fn lookup_address(
    store: &SqliteStore,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Option<PgpKey>, PgpError> {
    let http = mail_runtime::wkd::client()?;
    let Some(cert) = mail_runtime::wkd::lookup(&http, address).await? else {
        return Ok(None);
    };
    let kept = store.put_pgp_key(cert.record(KeySource::Wkd, now, &[address]))?;
    epoch_changed();
    Ok(Some(kept))
}

/// What asking one address's domain for its key came to.
#[derive(Debug)]
pub struct Discovered {
    pub address: String,
    pub outcome: Discovery,
}

/// How a key lookup for an address ended.
#[derive(Debug)]
pub enum Discovery {
    /// The domain published a key, now kept.
    Found(PgpKey),
    /// The domain publishes none.
    NoKey,
    /// The domain could not be asked.
    Failed(PgpError),
}

/// The To and Cc addresses of an encrypted draft that have no key yet, each asked of its
/// domain's Web Key Directory. An address that already has a key is not asked about and is
/// absent from the answer.
pub async fn discover(
    store: &SqliteStore,
    addresses: &[String],
    now: DateTime<Utc>,
) -> Vec<Discovered> {
    let mut out = Vec::new();
    for address in addresses {
        if matches!(keys::key_for(store, address, now), Ok(Some(_))) {
            continue;
        }
        let outcome = match lookup_address(store, address, now).await {
            Ok(Some(key)) => Discovery::Found(key),
            Ok(None) => Discovery::NoKey,
            Err(e) => Discovery::Failed(e),
        };
        out.push(Discovered {
            address: address.clone(),
            outcome,
        });
    }
    out
}

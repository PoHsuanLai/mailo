//! Protecting an outgoing message: what OpenPGP and S/MIME share, written once.
//!
//! The two schemes are the same send twice. A draft asks one of them to sign, encrypt or both;
//! nothing is sent if the other is asked too, the sender has nothing of their own to sign with,
//! a blind recipient would be named in the envelope, or a recipient has nothing to be encrypted
//! to. What differs is only whose keys, and how the bytes are sealed. [`Seal`] holds the first
//! as default methods ([`Seal::check`], [`Seal::outgoing`]) and asks the second of each scheme
//! ([`crate::pgp::OpenPgp`], [`crate::smime::Smime`]).
//!
//! Generic, not `dyn`: the caller names the scheme, and [`check_both`] is the one place that
//! names both.

use crate::error::CoreError;
use crate::message::{Protection, Scheme};
use crate::pgp::{Ask, OpenPgp};
use crate::smime::Smime;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_mime::Parsed;
use mail_secrets::SigningStore;
use mail_store::SqliteStore;

/// One way of protecting a message: OpenPGP, or S/MIME.
pub trait Seal {
    /// Why a step could not be done, typed so the caller can say what to do about it.
    type Error: std::error::Error;
    /// The other scheme: a draft asks this one or that one, never both.
    type Other: Seal;
    /// What the sender signs with and is encrypted to as well: their key or certificate record.
    type Own;
    /// What a recipient is encrypted to.
    type Cert;
    /// The unlocked secret that signs.
    type Signer;
    /// What opening a message of this scheme found.
    type Protected;

    /// Which scheme this is, as the reader names it.
    const SCHEME: Scheme;

    /// What `draft` asks of this scheme: its own field.
    fn asked(draft: &Draft) -> Option<Mode>;

    /// The sender's own key or certificate for `address`, when they have one to use at `now`.
    fn own(
        store: &SqliteStore,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Self::Own>, Self::Error>;

    /// What to encrypt to for `address` at `now`, when there is something.
    fn recipient(
        store: &SqliteStore,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<Self::Cert>, Self::Error>;

    /// The secret that signs for `own`, unlocked: `ask` is asked for the passphrase of one that
    /// has it.
    fn signer(
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        own: &Self::Own,
        ask: Ask<'_>,
    ) -> Result<Self::Signer, Self::Error>;

    /// `frozen` made what `mode` says. `to` is every `To` and `Cc` recipient with what to
    /// encrypt to; the sender's own is added by this, so the copy filed in Sent can be read.
    fn seal(
        frozen: &[u8],
        mode: Mode,
        signer: Option<&Self::Signer>,
        to: Vec<(String, Self::Cert)>,
        own: &Self::Own,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>, Self::Error>;

    /// `message` opened: its signature checked and its encryption decrypted. `None` when it
    /// carries nothing of this scheme. `ask` is asked for a passphrase if the scheme has any.
    fn open(
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        message: &Message,
        ask: Ask<'_>,
        now: DateTime<Utc>,
    ) -> Result<Option<Self::Protected>, Self::Error>;

    /// What was found, in the form the reader keeps it.
    fn wrap(protected: Self::Protected) -> Protection;

    /// The opened message and the bytes it was parsed from, which [`open`](Seal::open) hands out
    /// and the reader must not keep.
    fn parts(protected: &mut Self::Protected) -> (&mut Option<Parsed>, &mut Option<Vec<u8>>);

    /// The key whose passphrase is needed before the message opens, when that is what was found.
    /// Defaults to none: only a scheme with passphrases can be locked.
    fn locked(_protected: &Self::Protected) -> Option<Fingerprint> {
        None
    }

    /// The refusal for a draft that asks both schemes.
    fn both() -> Self::Error;
    /// The refusal for a sender with nothing of their own to protect with.
    fn no_own(address: String) -> Self::Error;
    /// The refusal for encrypting with blind recipients.
    fn blind(addresses: Vec<String>) -> Self::Error;
    /// The refusal for recipients with nothing to be encrypted to.
    fn missing(addresses: Vec<String>) -> Self::Error;

    /// Whether the sender's own `own` can be encrypted to, so the sent copy can be read.
    /// Defaults to yes: only a scheme whose own certificate may be unable to says otherwise.
    fn own_encrypts(_own: &Self::Own, _address: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    /// `frozen` before it is protected, whatever the draft asks. Defaults to as it is: only
    /// OpenPGP adds a header to every message.
    fn prepare(
        _store: &SqliteStore,
        _identity: &Identity,
        frozen: Vec<u8>,
    ) -> Result<Vec<u8>, Self::Error> {
        Ok(frozen)
    }

    /// `draft` asked of this scheme, and everything sending it needs, or `None` when it asks
    /// nothing of this scheme. Every refusal is here, so that [`check`](Seal::check) and
    /// [`outgoing`](Seal::outgoing) are one reading of the keys: a draft asking the other scheme
    /// too, a sender with nothing of their own, blind recipients under encryption, and every
    /// recipient with nothing to encrypt to, named together.
    fn ready(
        store: &SqliteStore,
        draft: &Draft,
        identity: &Identity,
        now: DateTime<Utc>,
    ) -> Result<Option<Ready<Self>>, Self::Error>
    where
        Self: Sized,
    {
        let Some(mode) = Self::asked(draft) else {
            return Ok(None);
        };
        if Self::Other::asked(draft).is_some() {
            return Err(Self::both());
        }
        let address = &identity.from.email;
        let Some(own) = Self::own(store, address, now)? else {
            return Err(Self::no_own(address.clone()));
        };
        let mut to = Vec::new();
        if mode.encrypts() {
            if !draft.bcc.is_empty() {
                return Err(Self::blind(
                    draft.bcc.iter().map(|a| a.email.clone()).collect(),
                ));
            }
            Self::own_encrypts(&own, address)?;
            let mut missing = Vec::new();
            for address in recipients(draft) {
                match Self::recipient(store, &address, now)? {
                    Some(cert) => to.push((address, cert)),
                    None => missing.push(address),
                }
            }
            if !missing.is_empty() {
                return Err(Self::missing(missing));
            }
        }
        Ok(Some(Ready { mode, own, to }))
    }

    /// What stands between `draft` and sending it as it asks, without touching the keyring: the
    /// composer asks this to warn before Send, and [`Seal::outgoing`] asks it again as the send
    /// happens.
    fn check(
        store: &SqliteStore,
        draft: &Draft,
        identity: &Identity,
        now: DateTime<Utc>,
    ) -> Result<(), Self::Error>
    where
        Self: Sized,
    {
        Self::ready(store, draft, identity, now).map(drop)
    }

    /// `frozen` — the bytes built for the wire — [prepared](Seal::prepare), then signed and
    /// encrypted as the draft asks of this scheme.
    fn outgoing(
        store: &SqliteStore,
        secrets: &dyn SigningStore,
        ask: Ask<'_>,
        draft: &Draft,
        identity: &Identity,
        frozen: Vec<u8>,
        now: DateTime<Utc>,
    ) -> Result<Vec<u8>, Self::Error>
    where
        Self: Sized,
    {
        let bytes = Self::prepare(store, identity, frozen)?;
        let Some(Ready { mode, own, to }) = Self::ready(store, draft, identity, now)? else {
            return Ok(bytes);
        };
        let signer = if mode.signs() {
            Some(Self::signer(store, secrets, &own, ask)?)
        } else {
            None
        };
        Self::seal(&bytes, mode, signer.as_ref(), to, &own, now)
    }
}

/// A draft that asks a scheme for something, and the keys that will do it.
pub struct Ready<S: Seal> {
    /// What the draft asks.
    pub mode: Mode,
    /// The sender's own key or certificate.
    pub own: S::Own,
    /// Every `To` and `Cc` recipient with what to encrypt to. Empty unless `mode` encrypts.
    pub to: Vec<(String, S::Cert)>,
}

/// What stands between `draft` and sending it, whichever scheme it asks.
///
/// S/MIME is checked first, so a draft asking both is refused before OpenPGP seals anything.
pub fn check_both(
    store: &SqliteStore,
    draft: &Draft,
    identity: &Identity,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    Smime::check(store, draft, identity, now)?;
    OpenPgp::check(store, draft, identity, now)?;
    Ok(())
}

/// The addresses the message is encrypted to: `To` and `Cc`, each once, lower-cased.
fn recipients(draft: &Draft) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for address in draft.to.iter().chain(&draft.cc) {
        let email = address.email.trim().to_ascii_lowercase();
        if !email.is_empty() && !out.contains(&email) {
            out.push(email);
        }
    }
    out
}

impl<S: Seal> std::fmt::Debug for Ready<S> {
    /// The keys are not printed: only what was asked.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ready")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

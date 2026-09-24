//! OpenPGP in the window: a protected message opened in the reader and said in plain words, the
//! passphrase asked for inline, and the keys sheet.
//!
//! What a message is — signed, encrypted, by whom — is [`crate::pgp`]'s, the module `mailo show`
//! and `mailo pgp` use, so the window and the command cannot disagree. Opening decrypts, so it is
//! slow and may need the keyring: it runs on a blocking thread, for a message the reader has
//! open and nowhere else — [`look`] is reached from [`seal::Seal`] and [`unlock`] only, and a
//! test counts its calls — and what it found is kept per message and body, like the receipt
//! bar's and the invitation card's. The decrypted body lives in that cache and nowhere else: it
//! is shown in place of the stored one, through the same blocks, and never written down.
//!
//! A passphrase typed to unlock a key is a [`Password`]: moved from the field into the one call
//! that needs it, and dropped with it. It is never in a signal, the [`Shell`](crate::view::Shell),
//! the store, a file or a log.

pub(in crate::ui) mod keys;
mod seal;
mod unlock;

pub(in crate::ui) use seal::Seal;
pub(in crate::ui) use unlock::Unlock;

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use mail_domain::*;
use mail_mime::{Parsed, SanitizePolicy};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};

use crate::password::Password;
use crate::pgp::{Ask, Protected};
use crate::view::Reading;

/// Look a key up by its address's domain. Blocks on the network: run it off the thread that draws.
pub(in crate::ui) type Lookup =
    dyn Fn(&SqliteStore, &str) -> Result<Option<PgpKey>, String> + Send + Sync;
/// Ask for a file to read. Blocks until the dialog is answered.
pub(in crate::ui) type PickFile = dyn Fn() -> Option<PathBuf> + Send + Sync;
/// Ask where to write a file, suggesting a name. Blocks until the dialog is answered.
pub(in crate::ui) type SaveFile = dyn Fn(&str) -> Option<PathBuf> + Send + Sync;

/// OpenPGP's reach into the world: the keyring, the Web Key Directory and the file dialogs,
/// handed in as a context so tests reach none of them.
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

/// How a line about a message's protection is drawn. A bad signature is the one that must be
/// impossible to miss; a signature nobody can check must not look like a good one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Tone {
    /// A plain fact: not encrypted, not signed.
    Plain,
    /// Opened, or a signature that checks.
    Good,
    /// Nothing can be said either way: a key this client does not hold.
    Unknown,
    /// A signature that does not check, or content that could not be read.
    Bad,
}

impl Tone {
    pub(in crate::ui) fn class(self) -> &'static str {
        match self {
            Tone::Plain => "seal-line",
            Tone::Good => "seal-line good",
            Tone::Unknown => "seal-line unknown",
            Tone::Bad => "seal-line bad",
        }
    }
}

/// One line the reader says about a message's protection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Said {
    pub tone: Tone,
    pub text: String,
}

/// A key's short name: its key id, in the groups of four people read fingerprints aloud in.
pub(in crate::ui) fn short(fingerprint: Fingerprint) -> String {
    grouped(&fingerprint.key_id().to_string())
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

/// What the reader says about `protected`, a line a fact: the same facts `mailo show` prints
/// ([`crate::pgp::describe`]), in the window's words. Pure, so every wording is a table test.
pub(in crate::ui) fn said(protected: &Protected) -> Vec<Said> {
    let line = |tone, text: String| Said { tone, text };
    let mut out = Vec::new();
    out.push(match &protected.encryption {
        Encryption::NotEncrypted => line(Tone::Plain, "Not encrypted".to_owned()),
        Encryption::Decrypted => line(
            Tone::Good,
            "Encrypted, and decrypted for reading".to_owned(),
        ),
        Encryption::CannotDecrypt { to } => {
            let ids: Vec<String> = to.iter().map(|id| grouped(&id.to_string())).collect();
            line(
                Tone::Unknown,
                format!(
                    "Encrypted to keys you do not hold ({}), so it cannot be read here",
                    ids.join(", ")
                ),
            )
        }
        Encryption::Locked { key } => line(
            Tone::Unknown,
            format!(
                "Encrypted to your key {}, which needs its passphrase",
                short(*key)
            ),
        ),
        Encryption::Unreadable { why } => line(
            Tone::Bad,
            format!("Encrypted, and could not be read: {why}"),
        ),
    });
    let coverage = match &protected.verification {
        Verification::NoSignature => {
            out.push(line(Tone::Plain, "Not signed".to_owned()));
            None
        }
        Verification::Good {
            signer,
            trust,
            coverage,
        } => {
            let name = protected
                .signer
                .as_ref()
                .map_or_else(|| "a key you hold".to_owned(), who);
            let trust = match trust {
                KeyTrust::Verified => "verified by you",
                KeyTrust::Unverified => "not verified by you",
            };
            out.push(line(
                Tone::Good,
                format!("Good signature by {name} · {} · {trust}", short(*signer)),
            ));
            Some(*coverage)
        }
        Verification::Bad { coverage } => {
            out.push(line(
                Tone::Bad,
                "Bad signature: the message was changed after it was signed, or the signature \
                 is forged"
                    .to_owned(),
            ));
            Some(*coverage)
        }
        Verification::UnknownKey { issuer, coverage } => {
            out.push(line(
                Tone::Unknown,
                format!(
                    "Signed by key {}, which you do not have, so the signature cannot be checked",
                    grouped(&issuer.to_string())
                ),
            ));
            Some(*coverage)
        }
    };
    if coverage == Some(Coverage::Part) {
        out.push(line(
            Tone::Unknown,
            "Only part of this message is signed; the rest could say anything".to_owned(),
        ));
    }
    out
}

/// A message opened: what to say about it, and the body to show in place of the stored one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Opened {
    pub said: Vec<Said>,
    /// The decrypted or unwrapped message, parsed. `None` when there is nothing to show but
    /// what is stored: it could not be read.
    pub shown: Option<Box<Parsed>>,
    /// `shown` drawn into blocks, per sanitizer policy, as the reader asked for them.
    pub readings: Vec<(SanitizePolicy, Reading)>,
}

/// What opening a message found, as the reader keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Look {
    /// It carries no OpenPGP, or only its headers are here.
    Plain,
    Opened(Box<Opened>),
    /// Encrypted to the user's key `key`, whose passphrase is needed.
    Locked {
        key: Fingerprint,
        tried: Tried,
    },
    /// Opening it failed, in words.
    Failed(String),
}

#[cfg(test)]
static LOOKED: std::sync::Mutex<Vec<MessageId>> = std::sync::Mutex::new(Vec::new());

/// Whether [`look`] has opened `message` in this test process.
#[cfg(test)]
pub(in crate::ui) fn looked_at(message: MessageId) -> bool {
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .contains(&message)
}

/// Open `message` with `ask` for a passphrase. `tried` is what a key still locked means. This
/// decrypts and may read the keyring: it runs on a blocking thread, for a message the reader
/// has open, and nowhere else.
fn look(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    message: MessageId,
    ask: Ask<'_>,
    tried: Tried,
) -> Look {
    #[cfg(test)]
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .push(message);
    let Ok(stored) = store.message(message) else {
        return Look::Plain;
    };
    match crate::pgp::open_message(store, secrets, &stored, ask, Utc::now()) {
        Ok(None) => Look::Plain,
        Ok(Some(protected)) => match protected.encryption {
            Encryption::Locked { key } => Look::Locked { key, tried },
            _ => Look::Opened(Box::new(opened(&stored, &protected))),
        },
        Err(e) => Look::Failed(format!("This message's OpenPGP could not be opened: {e}")),
    }
}

/// `protected` as the reader keeps it, with its blocks drawn for the policy a thread opens with.
fn opened(message: &Message, protected: &Protected) -> Opened {
    let policy = SanitizePolicy::CURRENT;
    let readings = protected
        .shown
        .as_ref()
        .map(|parsed| {
            vec![(
                policy,
                crate::view::reading(&message.body, Some(parsed), policy),
            )]
        })
        .unwrap_or_default();
    Opened {
        said: said(protected),
        shown: protected.shown.clone().map(Box::new),
        readings,
    }
}

/// How many messages' looks to keep. A decrypted message is its whole body, so few.
const KEPT: usize = 32;

type Kept = (MessageId, Option<BlobId>, Look);

static CACHE: std::sync::Mutex<Vec<Kept>> = std::sync::Mutex::new(Vec::new());

/// What was found for `message` holding `body`, if it has been opened.
pub(in crate::ui) fn cached(message: MessageId, body: Option<BlobId>) -> Option<Look> {
    let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache
        .iter()
        .find(|(had, at, _)| *had == message && *at == body)
        .map(|(_, _, look)| look.clone())
}

fn keep(message: MessageId, body: Option<BlobId>, look: &Look) {
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    cache.retain(|(had, _, _)| *had != message);
    cache.push((message, body, look.clone()));
    if cache.len() > KEPT {
        cache.remove(0);
    }
}

/// Forget every message opened: the keys changed, and what was said about a signature, or a
/// message that could not be read, may not be true any more.
pub(in crate::ui) fn forget_all() {
    CACHE
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .clear();
}

/// [`cached`], else [`look`] with no passphrase, remembered. Runs on a blocking thread.
pub(in crate::ui) fn lookup(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    message: MessageId,
    body: Option<BlobId>,
) -> Look {
    if let Some(had) = cached(message, body) {
        return had;
    }
    let found = look(
        store,
        secrets,
        message,
        &crate::pgp::no_passphrase,
        Tried::Nothing,
    );
    keep(message, body, &found);
    found
}

/// Open `message` again with the passphrase typed for it, and keep what that found. The
/// passphrase is dropped when this returns. Runs on a blocking thread, and only from Unlock.
pub(in crate::ui) fn unlock(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    message: MessageId,
    body: Option<BlobId>,
    passphrase: Password,
) -> Look {
    let ask = |_: Fingerprint| Some(passphrase.expose().to_owned());
    let found = look(store, secrets, message, &ask, Tried::Wrong);
    drop(passphrase);
    keep(message, body, &found);
    found
}

/// The blocks to show for `message` under `policy` when it was opened and has a body of its
/// own, drawn on first asking and kept. `None` shows what is stored. No decryption here: only
/// what [`lookup`] already found.
pub(in crate::ui) fn reading(message: &Message, policy: SanitizePolicy) -> Option<Reading> {
    let body = message.body.raw();
    let parsed = {
        let cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
        let (_, _, Look::Opened(opened)) = cache
            .iter()
            .find(|(had, at, _)| *had == message.id && *at == body)?
        else {
            return None;
        };
        if let Some((_, had)) = opened.readings.iter().find(|(at, _)| *at == policy) {
            return Some(had.clone());
        }
        opened.shown.clone()?
    };
    let drawn = crate::view::reading(&message.body, Some(&parsed), policy);
    let mut cache = CACHE.lock().unwrap_or_else(|held| held.into_inner());
    if let Some((_, _, Look::Opened(opened))) = cache
        .iter_mut()
        .find(|(had, at, _)| *had == message.id && *at == body)
    {
        opened.readings.push((policy, drawn.clone()));
    }
    Some(drawn)
}

/// Whether `message` was opened to a body of its own. Its stored parts are then OpenPGP's
/// wrapping — the signature, the ciphertext — and not attachments anyone sent.
pub(in crate::ui) fn opened_to_a_body(message: &Message) -> bool {
    matches!(
        cached(message.id, message.body.raw()),
        Some(Look::Opened(opened)) if opened.shown.is_some()
    )
}

/// The subject `message` was opened to, when that differs from the stored one: an encrypted
/// message's real subject travels inside it, and the outside says `...`.
pub(in crate::ui) fn subject(message: &Message) -> Option<String> {
    match cached(message.id, message.body.raw())? {
        Look::Opened(opened) => opened
            .shown
            .map(|parsed| parsed.subject)
            .filter(|subject| !subject.trim().is_empty() && *subject != message.subject),
        _ => None,
    }
}

#[cfg(test)]
mod keys_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;

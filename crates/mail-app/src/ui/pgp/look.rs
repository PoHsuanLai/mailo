//! Opening a message the reader shows, and keeping what that found.
//!
//! A message is asked of OpenPGP first and of S/MIME when OpenPGP finds none. What was found is
//! kept per message and body, with the count of key changes it was found under
//! ([`crate::pgp::epoch`], which S/MIME's moves too): once the keys or certificates change —
//! from the sheet, the command line, or a certificate learnt from arriving mail — the next
//! opening asks again, and nothing has to remember to clear it.

use std::path::{Path, PathBuf};

use chrono::Utc;
use mail_domain::*;
use mail_mime::{Parsed, SanitizePolicy};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};

use super::super::text::{AttachmentRow, Kept as Where};
use super::{Said, Scheme, Tried, said, said_smime};
use crate::password::Password;
use crate::pgp::Ask;
use crate::view::Reading;

/// A message opened: what to say about it, and the body to show in place of the stored one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Opened {
    /// Which protection it carries.
    pub scheme: Scheme,
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

#[cfg(test)]
static LOOKED: std::sync::Mutex<Vec<MessageId>> = std::sync::Mutex::new(Vec::new());

/// How many times [`look`] has opened `message` in this test process.
#[cfg(test)]
pub(in crate::ui) fn looks_at(message: MessageId) -> usize {
    LOOKED
        .lock()
        .unwrap_or_else(|held| held.into_inner())
        .iter()
        .filter(|had| **had == message)
        .count()
}

/// Whether [`look`] has opened `message` in this test process.
#[cfg(test)]
pub(in crate::ui) fn looked_at(message: MessageId) -> bool {
    looks_at(message) > 0
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
        Ok(None) => {}
        Ok(Some(protected)) => {
            return match protected.encryption {
                Encryption::Locked { key } => Look::Locked { key, tried },
                _ => Look::Opened(Box::new(opened(
                    &stored,
                    Scheme::OpenPgp,
                    said(&protected),
                    protected.shown,
                ))),
            };
        }
        Err(e) => return Look::Failed(format!("This message's OpenPGP could not be opened: {e}")),
    }
    match crate::smime::open_message(store, secrets, &stored, Utc::now()) {
        Ok(None) => Look::Plain,
        Ok(Some(protected)) => Look::Opened(Box::new(opened(
            &stored,
            Scheme::Smime,
            said_smime(&protected),
            protected.shown,
        ))),
        Err(e) => Look::Failed(format!("This message's S/MIME could not be opened: {e}")),
    }
}

/// What was found, as the reader keeps it, with its blocks drawn for the policy a thread opens
/// with.
fn opened(message: &Message, scheme: Scheme, said: Vec<Said>, shown: Option<Parsed>) -> Opened {
    let policy = SanitizePolicy::CURRENT;
    let readings = shown
        .as_ref()
        .map(|parsed| {
            vec![(
                policy,
                crate::view::reading(&message.body, Some(parsed), policy),
            )]
        })
        .unwrap_or_default();
    Opened {
        scheme,
        said,
        shown: shown.map(Box::new),
        readings,
    }
}

/// How many messages' looks to keep. A decrypted message is its whole body, so few.
const KEPT: usize = 32;

/// One message's look, and the count of key changes it was found under.
struct Kept {
    message: MessageId,
    body: Option<BlobId>,
    epoch: u64,
    look: Look,
}

static CACHE: std::sync::Mutex<Vec<Kept>> = std::sync::Mutex::new(Vec::new());

fn held() -> std::sync::MutexGuard<'static, Vec<Kept>> {
    CACHE.lock().unwrap_or_else(|held| held.into_inner())
}

/// What was found for `message` holding `body`, if it was opened under the keys held now.
pub(in crate::ui) fn cached(message: MessageId, body: Option<BlobId>) -> Option<Look> {
    let now = crate::pgp::epoch();
    held()
        .iter()
        .find(|kept| kept.message == message && kept.body == body && kept.epoch == now)
        .map(|kept| kept.look.clone())
}

/// What was last found for `message` holding `body`, under whichever keys. What the reader
/// draws until the message is opened again: the body and the words stay together.
fn last(message: MessageId, body: Option<BlobId>) -> Option<Look> {
    held()
        .iter()
        .find(|kept| kept.message == message && kept.body == body)
        .map(|kept| kept.look.clone())
}

/// Keep `look`, under the count as it stands once the look is done: opening a signed message
/// may itself teach a certificate, and what it found already knows it.
fn keep(message: MessageId, body: Option<BlobId>, look: &Look) {
    let epoch = crate::pgp::epoch();
    let mut cache = held();
    cache.retain(|kept| kept.message != message);
    cache.push(Kept {
        message,
        body,
        epoch,
        look: look.clone(),
    });
    if cache.len() > KEPT {
        cache.remove(0);
    }
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
        let cache = held();
        let kept = cache
            .iter()
            .find(|kept| kept.message == message.id && kept.body == body)?;
        let Look::Opened(opened) = &kept.look else {
            return None;
        };
        if let Some((_, had)) = opened.readings.iter().find(|(at, _)| *at == policy) {
            return Some(had.clone());
        }
        opened.shown.clone()?
    };
    let drawn = crate::view::reading(&message.body, Some(&parsed), policy);
    let mut cache = held();
    if let Some(Kept {
        look: Look::Opened(opened),
        ..
    }) = cache
        .iter_mut()
        .find(|kept| kept.message == message.id && kept.body == body)
    {
        opened.readings.push((policy, drawn.clone()));
    }
    Some(drawn)
}

/// The body `message` was opened to, when it has one of its own.
fn shown(message: &Message) -> Option<Box<Parsed>> {
    match last(message.id, message.body.raw())? {
        Look::Opened(opened) => opened.shown,
        _ => None,
    }
}

/// The subject `message` was opened to, when that differs from the stored one: an encrypted
/// message's real subject travels inside it, and the outside says `...`.
pub(in crate::ui) fn subject(message: &Message) -> Option<String> {
    shown(message)
        .map(|parsed| parsed.subject)
        .filter(|subject| !subject.trim().is_empty() && *subject != message.subject)
}

/// The attachments to list for `message` when it was opened to a body of its own: those inside
/// it. Its stored parts are then its wrapping — the signature, the ciphertext — and not
/// attachments anyone sent. `None` lists what is stored.
pub(in crate::ui) fn attachments(message: &Message) -> Option<Vec<AttachmentRow>> {
    let parsed = shown(message)?;
    Some(
        parsed
            .attachments
            .iter()
            .enumerate()
            .map(|(index, part)| AttachmentRow {
                index,
                name: crate::attach::safe_name(&part.name),
                size: crate::attach::human_size(part.bytes.len() as u64),
                kept: Where::Opened,
            })
            .collect(),
    )
}

/// Save attachment `index` of what `message` holding `body` was opened to into `dir`, numbered
/// as [`attachments`] lists them. Writes a file: run it off the thread that draws.
pub(in crate::ui) fn save_attachment(
    message: MessageId,
    body: Option<BlobId>,
    index: usize,
    dir: &Path,
) -> Result<PathBuf, String> {
    let Some(Look::Opened(opened)) = last(message, body) else {
        return Err("That message is not open any more; open it again to save this.".to_owned());
    };
    let shown = opened
        .shown
        .ok_or("That message could not be opened, so its attachments cannot be read.")?;
    let attachment = crate::attach::opened_attachment(&shown, index)?;
    crate::attach::save_opened(&attachment, dir)
}

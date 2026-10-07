//! The work behind the Keys and certificates page of Settings ([`super::keys_page`]): OpenPGP
//! keys, then S/MIME certificates, the user's own first in each, and what can be done to each.
//!
//! ⌘K "Keys and certificates…" opens Settings on it. Every change goes through
//! [`mail_core::pgp::keys`] or [`mail_core::smime::certs`], the functions `mailo pgp` and `mailo smime`
//! use, and runs on a blocking thread: a key is made, imported, exported or forgotten in the
//! keyring, and a file is read or written, none of which the thread that draws may wait on. The
//! acts that cannot be taken back — writing a secret key to a file, and deleting one — are each
//! asked again by the page, in words, before anything happens.
//!
//! Nothing here clears what the reader found when it opened a message: every change moves the
//! count of key changes, and the reader opens a message again when that has moved.

use chrono::Utc;
use mail_domain::{Fingerprint, PgpKey, SecretHeld};
use mail_runtime::SigningStore;
use mail_store::SqliteStore;
use std::path::{Path, PathBuf};

use super::super::data::account_rows;
use super::certs::CertJob;
use super::{Seams, short, who};
use mail_core::pgp::WithSecret;

/// What the page is called.
pub(in crate::ui) const TITLE: &str = "Keys and certificates";

pub(in crate::ui) use super::keys_page::KeysPage;

/// The keys in the order the page lists them: the user's own — those whose secret half the
/// keyring holds — first, then everyone else's, each group as the store orders it.
pub(in crate::ui) fn ordered(keys: Vec<PgpKey>) -> Vec<PgpKey> {
    let (mut own, others): (Vec<PgpKey>, Vec<PgpKey>) = keys
        .into_iter()
        .partition(|key| key.secret == SecretHeld::Held);
    own.extend(others);
    own
}

/// The addresses of the user's sending accounts that have no key of their own yet: what
/// Generate is offered for.
pub(in crate::ui) fn keyless(store: &SqliteStore) -> Vec<String> {
    account_rows(store)
        .into_iter()
        .filter(|row| !row.is_local())
        .map(|row| row.address)
        .filter(|address| matches!(mail_core::pgp::own_key(store, address), Ok(None)))
        .collect()
}

/// Something the page does off the thread that draws.
#[derive(Debug)]
pub(in crate::ui) enum Job {
    Generate(String),
    Import,
    SavePublic(PgpKey),
    SaveSecret(PgpKey),
    Delete(PgpKey, WithSecret),
    Verify(Fingerprint),
    Cert(CertJob),
}

/// What a job came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Done {
    /// What the page says afterwards.
    Said(String),
    /// The file at this path is an identity that needs its password: the page asks for it.
    Password(PathBuf),
}

/// Do `job`. Blocks — on the keyring, a file dialog, the disk — so the page runs it on a
/// blocking thread.
pub(in crate::ui) fn work(store: &SqliteStore, seams: &Seams, job: Job) -> Result<Done, String> {
    let secrets = seams.secrets.as_ref();
    let said = match job {
        Job::Cert(job) => return super::certs::work(store, seams, job),
        Job::Generate(address) => generate(store, secrets, &address)?,
        Job::Import => {
            let Some(path) = (seams.pick)() else {
                return Ok(Done::Said("Nothing was imported.".to_owned()));
            };
            import(store, secrets, &path)?
        }
        Job::SavePublic(key) => {
            let Some(path) = (seams.save)(&file_name(&key, "public")) else {
                return Ok(Done::Said("Nothing was saved.".to_owned()));
            };
            let armored = mail_core::pgp::keys::export_public(&key).map_err(|e| e.to_string())?;
            write(&path, &armored)?;
            format!(
                "Saved the public key of {} to {}",
                who(&key),
                path.display()
            )
        }
        Job::SaveSecret(key) => {
            let Some(path) = (seams.save)(&file_name(&key, "secret")) else {
                return Ok(Done::Said("Nothing was saved.".to_owned()));
            };
            let armored = mail_core::pgp::keys::export_secret(store, secrets, &key)
                .map_err(|e| e.to_string())?;
            write(&path, &armored)?;
            format!(
                "Saved the secret key of {} to {}",
                who(&key),
                path.display()
            )
        }
        Job::Delete(key, with_secret) => {
            mail_core::pgp::keys::delete(store, secrets, &key, with_secret)
                .map_err(|e| e.to_string())?;
            format!("Deleted the key of {}", who(&key))
        }
        Job::Verify(fingerprint) => {
            mail_core::pgp::keys::verify(store, fingerprint).map_err(|e| e.to_string())?;
            format!("Marked {} as verified", short(fingerprint))
        }
    };
    Ok(Done::Said(said))
}

fn generate(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    address: &str,
) -> Result<String, String> {
    let key = mail_core::pgp::keys::generate(store, secrets, address, Utc::now())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Made a key for {address}, {}",
        short(key.fingerprint)
    ))
}

fn import(store: &SqliteStore, secrets: &dyn SigningStore, path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let imported = mail_core::pgp::keys::import(store, secrets, &bytes, Utc::now())
        .map_err(|e| e.to_string())?;
    let names: Vec<String> = imported.iter().map(|one| who(&one.key)).collect();
    let secret = imported.iter().filter(|one| one.secret.is_some()).count();
    Ok(match (names.len(), secret) {
        (0, _) => format!("{} holds no OpenPGP key.", path.display()),
        (_, 0) => format!("Imported {}", names.join(", ")),
        _ => format!(
            "Imported {}, with {secret} secret key{} now in your keyring",
            names.join(", "),
            if secret == 1 { "" } else { "s" }
        ),
    })
}

/// The name a key's file is offered under.
fn file_name(key: &PgpKey, half: &str) -> String {
    format!("{}-{half}.asc", key.fingerprint.key_id())
}

/// Write `text` to `path`. A secret key is readable by its owner only, and so is every file the
/// page writes.
pub(super) fn write(path: &Path, text: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    // Windows has no mode bits: a new file takes the access list of the folder it is put in,
    // which under the user's profile admits only them, SYSTEM and Administrators. Setting one of
    // its own needs the Win32 security API, which is `unsafe` this workspace forbids; the page
    // says to keep a secret key offline either way.
    use std::io::Write as _;
    options
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

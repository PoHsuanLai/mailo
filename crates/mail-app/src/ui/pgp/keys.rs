//! The keys and certificates sheet: OpenPGP keys, then S/MIME certificates, the user's own first
//! in each, and what can be done to each.
//!
//! Ctrl T "Keys and certificates…" and the Space editor open it. Every change goes through
//! [`crate::pgp::keys`] or [`crate::smime::certs`], the functions `mailo pgp` and `mailo smime`
//! use, and runs on a blocking thread: a key is made, imported, exported or forgotten in the
//! keyring, and a file is read or written, none of which the thread that draws may wait on. The
//! acts that cannot be taken back — writing a secret key to a file, and deleting one — are each
//! asked again in the sheet, in words, before anything happens.
//!
//! Nothing here clears what the reader found when it opened a message: every change moves the
//! count of key changes, and the reader opens a message again when that has moved.

use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::{Fingerprint, PgpKey, SecretHeld};
use mail_runtime::Secrets;
use mail_store::{SqliteStore, Store};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::data::account_rows;
use super::certs::{CertJob, CertPart};
use super::key_row::{Confirm, KeyRow};
use super::{Busy, Seams, seams, short, who};
use crate::pgp::WithSecret;
use crate::view::{KeysSheet as Showing, Shell};
use ds::{Glyph, Icon};

/// What the sheet and its menu entry are called.
pub(in crate::ui) const TITLE: &str = "Keys and certificates";

/// Open the sheet.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().keys = Some(Showing);
}

/// Close the sheet and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().keys = None;
    dioxus::document::eval("document.querySelector('.app')?.focus()");
}

/// The keys in the order the sheet lists them: the user's own — those whose secret half the
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
        .filter(|address| matches!(crate::pgp::own_key(store, address), Ok(None)))
        .collect()
}

/// Something the sheet does off the thread that draws.
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
    /// What the sheet says afterwards.
    Said(String),
    /// The file at this path is an identity that needs its password: the sheet asks for it.
    Password(PathBuf),
}

/// Do `job`. Blocks — on the keyring, a file dialog, the disk — so the sheet runs it on a
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
            let armored = crate::pgp::keys::export_public(&key).map_err(|e| e.to_string())?;
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
            let armored =
                crate::pgp::keys::export_secret(store, secrets, &key).map_err(|e| e.to_string())?;
            write(&path, &armored)?;
            format!(
                "Saved the secret key of {} to {}. Keep that file offline.",
                who(&key),
                path.display()
            )
        }
        Job::Delete(key, with_secret) => {
            crate::pgp::keys::delete(store, secrets, &key, with_secret)
                .map_err(|e| e.to_string())?;
            format!("Deleted the key of {}", who(&key))
        }
        Job::Verify(fingerprint) => {
            crate::pgp::keys::verify(store, fingerprint).map_err(|e| e.to_string())?;
            format!("Marked {} as verified", short(fingerprint))
        }
    };
    Ok(Done::Said(said))
}

fn generate(store: &SqliteStore, secrets: &dyn Secrets, address: &str) -> Result<String, String> {
    let key = crate::pgp::keys::generate(store, secrets, address, Utc::now())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Made a key for {address}, {}. Its secret half is in your system keyring.",
        short(key.fingerprint)
    ))
}

fn import(store: &SqliteStore, secrets: &dyn Secrets, path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let imported =
        crate::pgp::keys::import(store, secrets, &bytes, Utc::now()).map_err(|e| e.to_string())?;
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
/// sheet writes.
pub(super) fn write(path: &Path, text: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    use std::io::Write as _;
    options
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// The sheet. Mounted while `shell.keys` is `Some`.
#[component]
pub(in crate::ui) fn KeysSheet(shell: Signal<Shell>) -> Element {
    // Bumped by every change, so the keys are read again.
    let mut changed = use_signal(|| 0u64);
    let mut said = use_signal(|| None::<Result<String, String>>);
    let mut busy = use_signal(|| Busy::Idle);
    let mut confirm = use_signal(|| Confirm::Nothing);
    let _ = changed();
    let store = consume_context::<Arc<SqliteStore>>();
    let (keys, failed) = match store.pgp_keys() {
        Ok(keys) => (ordered(keys), None),
        Err(why) => (Vec::new(), Some(why.to_string())),
    };
    let (certs, certs_failed) = match store.smime_certs() {
        Ok(certs) => (super::certs::ordered(certs), None),
        Err(why) => (Vec::new(), Some(why.to_string())),
    };
    let keyless = keyless(&store);
    // Made here, so the task belongs to the sheet and not to a row a deletion takes away.
    let run = use_callback(move |job: Job| {
        if *busy.peek() == Busy::Working {
            return;
        }
        busy.set(Busy::Working);
        said.set(None);
        let store = consume_context::<Arc<SqliteStore>>();
        let seams = seams();
        spawn(async move {
            let done = tokio::task::spawn_blocking(move || work(&store, &seams, job))
                .await
                .unwrap_or_else(|error| Err(format!("It stopped before it finished: {error}")));
            match done {
                Ok(Done::Said(text)) => said.set(Some(Ok(text))),
                Ok(Done::Password(path)) => confirm.set(Confirm::Password(path)),
                Err(why) => said.set(Some(Err(why))),
            }
            busy.set(Busy::Idle);
            changed += 1;
        });
    });
    let working = busy() == Busy::Working;
    let import_label = "Import from a file…";
    rsx! {
        div {
            class: "keys-wrap",
            onclick: move |_| close(shell),
            div {
                class: "keys",
                role: "dialog",
                aria_label: TITLE,
                onclick: move |event| event.stop_propagation(),
                div { class: "keys-head",
                    h3 { "{TITLE}" }
                    button {
                        class: "mini",
                        r#type: "button",
                        onclick: move |_| close(shell),
                        "Close"
                        span { class: "k", "Esc" }
                    }
                }
                match said() {
                    Some(Ok(text)) => rsx! { p { class: "capnote said keys-said", role: "status", "{text}" } },
                    Some(Err(why)) => rsx! { p { class: "capnote files-bad keys-said", role: "alert", "{why}" } },
                    None => rsx! {},
                }
                div { class: "keys-main",
                    section { class: "keys-part",
                        h4 { class: "keys-sub", "OpenPGP" }
                        p { class: "capnote",
                            "Yours sign what you send and open what is sent to you. Theirs let you encrypt to them and check what they sign."
                        }
                        ul { class: "keys-list",
                            for key in keys {
                                KeyRow { key: "{key.fingerprint}", pgp: key.clone(), confirm, run, busy: busy() }
                            }
                            if let Some(why) = failed {
                                li { class: "keys-none", "{why}" }
                            }
                        }
                        div { class: "keys-acts",
                            for address in keyless {
                                button {
                                    key: "{address}",
                                    class: "mini",
                                    r#type: "button",
                                    aria_label: "Make a key for {address}",
                                    disabled: working,
                                    onclick: {
                                        let address = address.clone();
                                        move |_| run.call(Job::Generate(address.clone()))
                                    },
                                    Glyph { icon: Icon::Plus }
                                    "Make a key for {address}"
                                }
                            }
                            button {
                                class: "mini",
                                r#type: "button",
                                aria_label: "{import_label}",
                                disabled: working,
                                onclick: move |_| run.call(Job::Import),
                                "{import_label}"
                            }
                        }
                    }
                    CertPart { certs, failed: certs_failed, confirm, run, busy: busy() }
                }
            }
        }
    }
}

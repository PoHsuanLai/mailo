//! The S/MIME half of the keys and certificates sheet: the user's own certificates first, then
//! their correspondents' and the authorities', and what can be done to each.
//!
//! Every change goes through [`crate::smime::certs`], the functions `mailo smime` uses, on the
//! sheet's blocking thread. A PKCS#12 identity file's password is asked for in the sheet only
//! when the file turns out to need one: typed into a [`Password`], moved into the one import
//! that uses it, and dropped with it — never drawn, never in a signal.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{CertFingerprint, CertSource, KeyTrust, SecretHeld, SmimeCert};
use mail_runtime::Secrets;
use mail_store::SqliteStore;

use super::super::press::{available, on_primary};
use super::key_row::{Confirm, ConfirmBar};
use super::keys::{Done, Job, write};
use super::{Busy, Seams, Tried, Unlock, cert_short, whose};
use crate::password::Password;
use crate::pgp::WithSecret;
use crate::smime::SmimeError;

/// The certificates in the order the sheet lists them: the user's own — those whose private
/// key the keyring holds — first, then everyone else's, each group as the store orders it.
pub(in crate::ui) fn ordered(certs: Vec<SmimeCert>) -> Vec<SmimeCert> {
    let (mut own, others): (Vec<SmimeCert>, Vec<SmimeCert>) = certs
        .into_iter()
        .partition(|cert| cert.secret == SecretHeld::Held);
    own.extend(others);
    own
}

/// Something the certificates' half of the sheet does off the thread that draws.
#[derive(Debug)]
pub(in crate::ui) enum CertJob {
    /// Pick a file and import it, asking its password if it turns out to need one.
    Import,
    /// Import the identity file at this path with the password typed for it.
    ImportWith(PathBuf, Password),
    Save(SmimeCert),
    Trust(CertFingerprint, KeyTrust),
    Delete(SmimeCert, WithSecret),
}

/// Do `job`. Blocks — on the keyring, a file dialog, the disk.
pub(in crate::ui) fn work(
    store: &SqliteStore,
    seams: &Seams,
    job: CertJob,
) -> Result<Done, String> {
    let secrets = seams.secrets.as_ref();
    let said = match job {
        CertJob::Import => {
            let Some(path) = (seams.pick)() else {
                return Ok(Done::Said("Nothing was imported.".to_owned()));
            };
            return import(store, secrets, &path, None);
        }
        CertJob::ImportWith(path, password) => {
            return import(store, secrets, &path, Some(password));
        }
        CertJob::Save(cert) => {
            let name = format!("{}.pem", cert_short(cert.fingerprint).replace(' ', ""));
            let Some(path) = (seams.save)(&name) else {
                return Ok(Done::Said("Nothing was saved.".to_owned()));
            };
            let pem = crate::smime::certs::export(&cert).map_err(|e| e.to_string())?;
            write(&path, &pem)?;
            format!(
                "Saved the certificate of {} to {}",
                whose(&cert),
                path.display()
            )
        }
        CertJob::Trust(fingerprint, trust) => {
            crate::smime::certs::trust(store, fingerprint, trust).map_err(|e| e.to_string())?;
            match trust {
                KeyTrust::Verified => format!(
                    "Trusted {}: signatures by it, and by certificates it issued, are believed",
                    cert_short(fingerprint)
                ),
                KeyTrust::Unverified => {
                    format!("{} is no longer trusted", cert_short(fingerprint))
                }
            }
        }
        CertJob::Delete(cert, with_secret) => {
            crate::smime::certs::delete(store, secrets, &cert, with_secret)
                .map_err(|e| e.to_string())?;
            format!("Deleted the certificate of {}", whose(&cert))
        }
    };
    Ok(Done::Said(said))
}

/// Import the file at `path`, with `password` for an identity file when one was typed. A file
/// that needs one and was given none comes back as a question. The password is dropped when
/// this returns.
fn import(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    path: &Path,
    password: Option<Password>,
) -> Result<Done, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let given = || password.as_ref().map(|typed| typed.expose().to_owned());
    let imported = match crate::smime::certs::import(store, secrets, &bytes, &given, Utc::now()) {
        Err(SmimeError::NoPassword) => return Ok(Done::Password(path.to_owned())),
        other => other.map_err(|e| e.to_string())?,
    };
    drop(password);
    let names: Vec<String> = imported.iter().map(|one| whose(&one.cert)).collect();
    Ok(Done::Said(
        match imported
            .iter()
            .find(|one| one.cert.secret == SecretHeld::Held)
        {
            Some(own) => format!(
                "Imported your identity for {}. Its private key is in your system keyring.",
                whose(&own.cert)
            ),
            None if names.is_empty() => format!("{} holds no certificate.", path.display()),
            None => format!("Imported {}", names.join(", ")),
        },
    ))
}

/// Where a certificate came from, in words.
fn source(cert: &SmimeCert) -> &'static str {
    match cert.source {
        CertSource::Identity => "your identity",
        CertSource::Imported => "imported",
        CertSource::Received => "from their signed mail",
    }
}

/// How long a certificate holds, in words.
pub(in crate::ui) fn validity(cert: &SmimeCert, now: DateTime<Utc>) -> String {
    let day = |at: DateTime<Utc>| at.format("%Y-%m-%d").to_string();
    if cert.not_after < now {
        format!("expired {}", day(cert.not_after))
    } else if now < cert.not_before {
        format!("valid from {}", day(cert.not_before))
    } else {
        format!("valid {} to {}", day(cert.not_before), day(cert.not_after))
    }
}

/// The S/MIME section: the certificates, the password asked for an identity file, and Import.
#[component]
pub(in crate::ui) fn CertPart(
    certs: Vec<SmimeCert>,
    failed: Option<String>,
    confirm: Signal<Confirm>,
    run: Callback<Job>,
    busy: Busy,
) -> Element {
    let working = busy == Busy::Working;
    let mut confirm = confirm;
    let import_label = "Import a certificate or identity…";
    let asking = match confirm() {
        Confirm::Password(path) => Some(path),
        _ => None,
    };
    rsx! {
        section { class: "keys-part",
            h4 { class: "keys-sub", "S/MIME" }
            p { class: "capnote",
                "Yours, from an identity file (.p12, .pfx), sign what you send and open what is sent to you. Theirs come from their signed mail or a certificate file (.pem, .der), and let you encrypt to them."
            }
            ul { class: "keys-list",
                for cert in certs {
                    CertRow { key: "{cert.fingerprint}", cert: cert.clone(), confirm, run, busy }
                }
                if let Some(why) = failed {
                    li { class: "keys-none", "{why}" }
                }
            }
            if let Some(path) = asking {
                {
                    let file = path
                        .file_name()
                        .map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                    rsx! {
                        div { class: "keys-ask",
                            Unlock {
                                prompt: format!("{file} is an identity file. Type the password it was saved with to import it."),
                                tried: Tried::Nothing,
                                act: "Import".to_owned(),
                                noun: "Password".to_owned(),
                                working: busy,
                                on_unlock: move |password: Password| {
                                    confirm.set(Confirm::Nothing);
                                    run.call(Job::Cert(CertJob::ImportWith(path.clone(), password)));
                                },
                            }
                            ds::Button {
                                variant: ds::ButtonVariant::Mini,
                                label: "Cancel".to_owned(),
                                aria_label: format!("Cancel: Import {file}"),
                                onclick: on_primary(move || confirm.set(Confirm::Nothing)),
                            }
                        }
                    }
                }
            }
            div { class: "keys-acts",
                ds::Button {
                    variant: ds::ButtonVariant::Mini,
                    label: import_label.to_string(),
                    aria_label: import_label.to_string(),
                    availability: available(!working),
                    onclick: on_primary(move || run.call(Job::Cert(CertJob::Import))),
                }
            }
        }
    }
}

/// One certificate, its actions, and the question asked before deleting one with a private key.
#[component]
fn CertRow(cert: SmimeCert, confirm: Signal<Confirm>, run: Callback<Job>, busy: Busy) -> Element {
    let working = busy == Busy::Working;
    let fingerprint = cert.fingerprint;
    let id = cert_short(fingerprint);
    let mine = cert.secret == SecretHeld::Held;
    let trusted = cert.trust == KeyTrust::Verified;
    let mut meta = vec![
        cert.emails.join(", "),
        validity(&cert, Utc::now()),
        format!("issued by {}", cert.issuer),
        source(&cert).to_owned(),
        if trusted {
            "trusted by you"
        } else {
            "not trusted by you"
        }
        .to_owned(),
    ];
    if mine {
        meta.push("private key held here".to_owned());
    }
    let meta = meta
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let row_class = if mine { "keys-row mine" } else { "keys-row" };
    let (copied, saved, deleted, gone) = (cert.clone(), cert.clone(), cert.clone(), cert.clone());
    let mut confirm = confirm;
    rsx! {
        li { class: "{row_class}",
            div { class: "keys-text",
                b { "{cert.subject}" }
                span { class: "keys-fpr", "{id}" }
                span { class: "keys-meta", "{meta}" }
            }
            div { class: "keys-row-acts",
                if mine {
                    span { class: "keys-tag", "yours" }
                }
                ds::Button {
                    variant: ds::ButtonVariant::Secondary,
                    label: "Copy".to_owned(),
                    aria_label: format!("Copy the certificate {id}"),
                    onclick: on_primary(move || {
                        if let Ok(pem) = crate::smime::certs::export(&copied) {
                            super::super::hover::copy(&pem);
                            super::super::motion::tell(
                                "Certificate copied".to_owned(),
                                super::super::motion::Follow::Nothing,
                            );
                        }
                    }),
                }
                ds::Button {
                    variant: ds::ButtonVariant::Secondary,
                    label: "Save…".to_owned(),
                    aria_label: format!("Save the certificate {id}"),
                    availability: available(!working),
                    onclick: on_primary(move || run.call(Job::Cert(CertJob::Save(saved.clone())))),
                }
                if trusted {
                    ds::Button {
                        variant: ds::ButtonVariant::Secondary,
                        label: "Don't trust".to_owned(),
                        aria_label: format!("Stop trusting {id}"),
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Cert(CertJob::Trust(fingerprint, KeyTrust::Unverified)))),
                    }
                } else {
                    ds::Button {
                        variant: ds::ButtonVariant::Secondary,
                        label: "Trust".to_owned(),
                        aria_label: format!("Trust {id}"),
                        title: "Only after checking the fingerprint with its owner: it then vouches for every certificate it issued".to_owned(),
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Cert(CertJob::Trust(fingerprint, KeyTrust::Verified)))),
                    }
                }
                ds::Button {
                    variant: ds::ButtonVariant::Danger,
                    label: "Delete".to_owned(),
                    aria_label: format!("Delete {id}"),
                    availability: available(!working),
                    onclick: on_primary(move || {
                        if deleted.secret == SecretHeld::Held {
                            confirm.set(Confirm::DeleteCert(fingerprint));
                        } else {
                            run.call(Job::Cert(CertJob::Delete(deleted.clone(), WithSecret::Refuse)));
                        }
                    }),
                }
            }
            if confirm() == Confirm::DeleteCert(fingerprint) {
                ConfirmBar {
                    sentence: format!(
                        "Deleting {id} also deletes its private key from your keyring. It cannot be recovered, and mail encrypted to it can never be read again. Keep the identity file you imported it from."
                    ),
                    act: "Delete the certificate and its private key".to_owned(),
                    confirm,
                    on_yes: move |_| {
                        confirm.set(Confirm::Nothing);
                        run.call(Job::Cert(CertJob::Delete(gone.clone(), WithSecret::Confirmed)));
                    },
                }
            }
        }
    }
}

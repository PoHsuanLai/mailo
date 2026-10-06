//! The S/MIME half of the keys and certificates sheet: the user's own certificates first, then
//! their correspondents' and the authorities', and what can be done to each.
//!
//! Every change goes through [`mail_core::smime::certs`], the functions `mailo smime` uses, on the
//! sheet's blocking thread. A PKCS#12 identity file's password is asked for in the sheet only
//! when the file turns out to need one: typed into a [`Password`], moved into the one import
//! that uses it, and dropped with it — never drawn, never in a signal.

use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Answers, Bezel, ButtonRole};
use ds::components::lists::list::model::ListStyle;
use ds::prelude::*;
use ds::root::common::Common;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{CertFingerprint, CertSource, KeyTrust, SecretHeld, SmimeCert};
use mail_runtime::SigningStore;
use mail_store::SqliteStore;

use super::super::press::{available, on_primary};
use super::key_row::{Confirm, ConfirmBar};
use super::keys::{Done, Job, write};
use super::{Busy, Seams, Tried, Unlock, cert_short, whose};
use mail_core::password::Password;
use mail_core::pgp::WithSecret;
use mail_core::smime::SmimeError;

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
            let pem = mail_core::smime::certs::export(&cert).map_err(|e| e.to_string())?;
            write(&path, &pem)?;
            format!(
                "Saved the certificate of {} to {}",
                whose(&cert),
                path.display()
            )
        }
        CertJob::Trust(fingerprint, trust) => {
            mail_core::smime::certs::trust(store, fingerprint, trust).map_err(|e| e.to_string())?;
            match trust {
                KeyTrust::Verified => format!("Trusted {}", cert_short(fingerprint)),
                KeyTrust::Unverified => {
                    format!("{} is no longer trusted", cert_short(fingerprint))
                }
            }
        }
        CertJob::Delete(cert, with_secret) => {
            mail_core::smime::certs::delete(store, secrets, &cert, with_secret)
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
    secrets: &dyn SigningStore,
    path: &Path,
    password: Option<Password>,
) -> Result<Done, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let given = || password.as_ref().map(|typed| typed.expose().to_owned());
    let imported = match mail_core::smime::certs::import(store, secrets, &bytes, &given, Utc::now())
    {
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
            Some(own) => format!("Imported your identity for {}", whose(&own.cert)),
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
    let items: Vec<ListItem<String>> = certs
        .into_iter()
        .map(|cert| {
            let id = cert.fingerprint.to_string();
            let name = cert.subject.clone();
            ListItem::row(
                id.clone(),
                name,
                rsx! { CertRow { key: "{id}", cert, confirm, run, busy } },
            )
        })
        .collect();
    rsx! {
            section { class: "keys-part",
                SectionHeader { title: "S/MIME" }
                Label {
                    text: "Yours sign and decrypt. Theirs encrypt and verify.",
                    role: LabelRole::Secondary,
                    style: LabelStyle::Footnote,
                }
                div { class: "keys-list",
                    List::<String> { label: "S/MIME certificates", items, style: ListStyle::Inset }
                    if let Some(why) = failed {
                        Label { text: why, role: LabelRole::Tertiary }
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
                                Button {
                                    label: "Cancel",
                                    answers: Answers::Escape,
                                    onclick: on_primary(move || confirm.set(Confirm::Nothing)),
        common: Common { aria_label: Some(format!("Cancel: Import {file}")), ..Common::default() },
    }
                            }
                        }
                    }
                }
                div { class: "keys-acts",
                    Button {
                        label: import_label,
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Cert(CertJob::Import))),
        common: Common { aria_label: Some(import_label.to_string()), ..Common::default() },
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
    let (copied, saved, deleted, gone) = (cert.clone(), cert.clone(), cert.clone(), cert.clone());
    let mut confirm = confirm;
    let actions = rsx! {
            div { class: "keys-row-acts",
                Button {
                    label: "Copy",
                    bezel: Bezel::Inline,
                    onclick: on_primary(move || {
                        if let Ok(pem) = mail_core::smime::certs::export(&copied) {
                            super::super::hover::copy(&pem);
                            super::super::motion::tell(
                                "Certificate copied".to_owned(),
                                super::super::motion::Follow::Nothing,
                            );
                        }
                    }),
                    common: Common { aria_label: Some(format!("Copy the certificate {id}")), ..Common::default() },
                }
                Button {
                    label: "Save…",
                    bezel: Bezel::Inline,
                    availability: available(!working),
                    onclick: on_primary(move || run.call(Job::Cert(CertJob::Save(saved.clone())))),
        common: Common { aria_label: Some(format!("Save the certificate {id}")), ..Common::default() },
    }
                if trusted {
                    Button {
                        label: "Don't trust",
                        bezel: Bezel::Inline,
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Cert(CertJob::Trust(fingerprint, KeyTrust::Unverified)))),
        common: Common { aria_label: Some(format!("Stop trusting {id}")), ..Common::default() },
    }
                } else {
                    Button {
                        label: "Trust",
                        bezel: Bezel::Inline,
                        title: "Check the fingerprint first".to_owned(),
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Cert(CertJob::Trust(fingerprint, KeyTrust::Verified)))),
        common: Common { aria_label: Some(format!("Trust {id}")), ..Common::default() },
    }
                }
                Button {
                    label: "Delete",
                    bezel: Bezel::Inline,
                    role: ButtonRole::Destructive,
                    availability: available(!working),
                    onclick: on_primary(move || {
                        if deleted.secret == SecretHeld::Held {
                            confirm.set(Confirm::DeleteCert(fingerprint));
                        } else {
                            run.call(Job::Cert(CertJob::Delete(deleted.clone(), WithSecret::Refuse)));
                        }
                    }),
                    common: Common { aria_label: Some(format!("Delete {id}")), ..Common::default() },
                }
            }
        };
    let title = TextLine::Runs(vec![
        TextRun::new(cert.subject.clone(), RunTone::Strong),
        TextRun::new(format!("  {id}"), RunTone::Faint),
    ]);
    rsx! {
        div { class: if mine { "keys-row mine" } else { "keys-row" },
            Row {
                leading: RowLeading::Icon(if mine { Icon::Key } else { Icon::Mail }),
                title,
                detail: Some(TextLine::from(meta)),
                accessory: Accessory::Slot(actions),
            }
            if confirm() == Confirm::DeleteCert(fingerprint) {
                ConfirmBar {
                    sentence: format!(
                        "Deleting {id} also deletes its private key. This cannot be undone."
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

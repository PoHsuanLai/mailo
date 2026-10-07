//! One OpenPGP key on the Keys and certificates page, its actions, and the questions the page
//! asks again before an act that cannot be undone — for the certificates' rows too.

use ds::components::content::label::LabelRole;
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Answers, Bezel, ButtonRole};
use ds::components::lists::row::size::RowSize;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::components::overlays::sheet_width::SheetWidth;
use ds::prelude::*;
use ds::root::common::Common;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{CertFingerprint, Fingerprint, KeySource, KeyTrust, PgpKey, SecretHeld};

use super::super::common::in_card;
use super::super::press::{available, on_primary};
use super::super::sidebar::tagged;
use super::certs::CertJob;
use super::keys::Job;
use super::{Busy, Passphrase, cert_short, short, who};
use mail_core::password::Password;
use mail_core::pgp::WithSecret;
use mail_store::Store;

/// A question the page is asking before it acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Confirm {
    Nothing,
    /// Write this key's secret half to a file.
    ExportSecret(Fingerprint),
    /// Delete this key and its secret half.
    Delete(Fingerprint),
    /// Delete this certificate and its private key.
    DeleteCert(CertFingerprint),
    /// The password of the PKCS#12 file at this path, to import it.
    Password(PathBuf),
}

/// Where a key came from, in words.
fn source(key: &PgpKey) -> &'static str {
    match key.source {
        KeySource::Generated => "made here",
        KeySource::Imported => "imported",
        KeySource::Wkd => "from its domain",
        KeySource::Autocrypt => "from their mail",
        KeySource::Gossip => "passed on by someone else",
    }
}

/// When a key was made and until when it holds, as far as that is recorded. A key whose dates
/// were never read says nothing of either, rather than "does not expire".
pub(in crate::ui) fn lifetime(key: &PgpKey, now: DateTime<Utc>) -> Vec<String> {
    let day = |at: DateTime<Utc>| at.format("%Y-%m-%d").to_string();
    let mut out = Vec::new();
    if let Some(created) = key.created {
        out.push(format!("created {}", day(created)));
    }
    match (key.created, key.expires) {
        (_, Some(expires)) if expires <= now => out.push(format!("expired {}", day(expires))),
        (_, Some(expires)) => out.push(format!("expires {}", day(expires))),
        (Some(_), None) => out.push("does not expire".to_owned()),
        (None, None) => {}
    }
    out
}

/// One key: who it is for, what is known of it, and its actions. An act that cannot be undone
/// sets `confirm`, and the page asks ([`Asking`]).
#[component]
pub(in crate::ui) fn KeyRow(
    pgp: PgpKey,
    confirm: Signal<Confirm>,
    run: Callback<Job>,
    busy: Busy,
) -> Element {
    let working = busy == Busy::Working;
    let fingerprint = pgp.fingerprint;
    let name = who(&pgp);
    let id = short(fingerprint);
    let mine = pgp.secret == SecretHeld::Held;
    let verified = pgp.trust == KeyTrust::Verified;
    let mut meta = vec![pgp.emails.join(", ")];
    meta.extend(lifetime(&pgp, Utc::now()));
    meta.push(format!("added {}", pgp.first_seen.format("%Y-%m-%d")));
    meta.push(source(&pgp).to_owned());
    meta.push(
        if verified {
            "verified by you"
        } else {
            "not verified"
        }
        .to_owned(),
    );
    if mine {
        meta.push("secret key held here".to_owned());
    }
    let meta = meta
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let (copy_key, save_key, delete_key) = (pgp.clone(), pgp.clone(), pgp.clone());
    let mut confirm = confirm;
    let actions = rsx! {
        Button {
            label: "Copy public",
            bezel: Bezel::Inline,
            onclick: on_primary(move || {
                if let Ok(armored) = mail_core::pgp::keys::export_public(&copy_key) {
                    super::super::hover::copy(&armored);
                    super::super::motion::tell(
                        "Public key copied".to_owned(),
                        super::super::motion::Follow::Nothing,
                    );
                }
            }),
            common: Common { aria_label: Some(format!("Copy the public key {id}")), ..Common::default() },
        }
        Button {
            label: "Save public…",
            bezel: Bezel::Inline,
            availability: available(!working),
            onclick: on_primary(move || run.call(Job::SavePublic(save_key.clone()))),
            common: Common { aria_label: Some(format!("Save the public key {id}")), ..Common::default() },
        }
        if mine {
            Button {
                label: "Export secret…",
                bezel: Bezel::Inline,
                availability: available(!working),
                onclick: on_primary(move || confirm.set(Confirm::ExportSecret(fingerprint))),
                common: Common { aria_label: Some(format!("Export the secret key {id}")), ..Common::default() },
            }
        }
        if !verified {
            Button {
                label: "Verify",
                bezel: Bezel::Inline,
                title: "Check the fingerprint first".to_owned(),
                availability: available(!working),
                onclick: on_primary(move || run.call(Job::Verify(fingerprint))),
                common: Common { aria_label: Some(format!("Mark {id} verified")), ..Common::default() },
            }
        }
        Button {
            label: "Delete",
            bezel: Bezel::Inline,
            role: ButtonRole::Destructive,
            availability: available(!working),
            onclick: on_primary(move || {
                if delete_key.secret == SecretHeld::Held {
                    confirm.set(Confirm::Delete(fingerprint));
                } else {
                    run.call(Job::Delete(delete_key.clone(), WithSecret::Refuse));
                }
            }),
            common: Common { aria_label: Some(format!("Delete {id}")), ..Common::default() },
        }
    };
    let title = TextLine::Runs(vec![
        TextRun::new(name, RunTone::Strong),
        TextRun::new(format!("  {id}"), RunTone::Faint),
    ]);
    rsx! {
        Row {
            leading: RowLeading::Icon(if mine { Icon::Key } else { Icon::Mail }),
            title,
            detail: Some(TextLine::from(meta)),
            size: RowSize::Settings,
            accessory: Accessory::Slot(actions),
            common: tagged("mine", if mine { "yes" } else { "no" }),
        }
    }
}

/// The question the page is asking, if any: quire's alert before an act that cannot be undone,
/// or a sheet for an identity file's password. The key or certificate is read again by its
/// fingerprint, so a question about one that is gone asks nothing.
#[component]
pub(in crate::ui) fn Asking(confirm: Signal<Confirm>, run: Callback<Job>, busy: Busy) -> Element {
    let mut confirm = confirm;
    let store = consume_context::<std::sync::Arc<mail_store::SqliteStore>>();
    let cancel = move || {
        AlertButton::new(
            "Cancel",
            AlertRole::Cancel,
            EventHandler::new(move |()| confirm.set(Confirm::Nothing)),
        )
    };
    match confirm() {
        Confirm::Nothing => rsx! {},
        Confirm::ExportSecret(fingerprint) => {
            let Some(key) = store.pgp_key(fingerprint).ok().flatten() else {
                return rsx! {};
            };
            let title = format!("Export the secret key of {}?", who(&key));
            rsx! {
                Alert {
                    title,
                    message: Some(TextLine::from("Anyone with this file can read your mail and sign as you. Keep it offline.")),
                    style: AlertStyle::Warning,
                    buttons: vec![
                        AlertButton::new("Save the Secret Key…", AlertRole::Normal, EventHandler::new(move |()| {
                            confirm.set(Confirm::Nothing);
                            run.call(Job::SaveSecret(key.clone()));
                        })),
                        cancel(),
                    ],
                }
            }
        }
        Confirm::Delete(fingerprint) => {
            let Some(key) = store.pgp_key(fingerprint).ok().flatten() else {
                return rsx! {};
            };
            let id = short(fingerprint);
            rsx! {
                Alert {
                    title: format!("Delete {id}?"),
                    message: Some(TextLine::from(format!("Deleting {id} also deletes its secret key. This cannot be undone."))),
                    style: AlertStyle::Critical,
                    buttons: vec![
                        AlertButton::new("Delete", AlertRole::Destructive, EventHandler::new(move |()| {
                            confirm.set(Confirm::Nothing);
                            run.call(Job::Delete(key.clone(), WithSecret::Confirmed));
                        })),
                        cancel(),
                    ],
                }
            }
        }
        Confirm::DeleteCert(fingerprint) => {
            let Some(cert) = store.smime_cert(fingerprint).ok().flatten() else {
                return rsx! {};
            };
            let id = cert_short(fingerprint);
            rsx! {
                Alert {
                    title: format!("Delete {id}?"),
                    message: Some(TextLine::from(format!("Deleting {id} also deletes its private key. This cannot be undone."))),
                    style: AlertStyle::Critical,
                    buttons: vec![
                        AlertButton::new("Delete", AlertRole::Destructive, EventHandler::new(move |()| {
                            confirm.set(Confirm::Nothing);
                            run.call(Job::Cert(CertJob::Delete(cert.clone(), WithSecret::Confirmed)));
                        })),
                        cancel(),
                    ],
                }
            }
        }
        Confirm::Password(path) => {
            let file = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            rsx! {
                Sheet {
                    label: "Import an identity".to_owned(),
                    width: SheetWidth::Narrow,
                    common: in_card(),
                    onclose: move |()| confirm.set(Confirm::Nothing),
                    div { class: "keys-ask",
                        Label { text: format!("{file} is an identity file. Type the password it was saved with to import it."), role: LabelRole::Secondary }
                        Passphrase {
                            prompt: format!("{file} is an identity file. Type the password it was saved with to import it."),
                            act: "Import".to_owned(),
                            noun: "Password".to_owned(),
                            working: busy,
                            on_unlock: move |password: Password| {
                                confirm.set(Confirm::Nothing);
                                run.call(Job::Cert(CertJob::ImportWith(path.clone(), password)));
                            },
                        }
                        div { class: "sheet-actions",
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
        }
    }
}

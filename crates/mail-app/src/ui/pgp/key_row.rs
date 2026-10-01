//! One OpenPGP key in the sheet, its actions, and the question asked again before an act that
//! cannot be undone — the bar the certificates' rows ask theirs in too.

use ds::components::content::label::LabelRole;
use ds::components::content::text_runs::RunTone;
use ds::components::controls::button_model::{Answers, Bezel, ButtonRole};
use ds::prelude::*;
use ds::root::common::Common;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{CertFingerprint, Fingerprint, KeySource, KeyTrust, PgpKey, SecretHeld};

use super::super::press::{available, on_primary};
use super::keys::Job;
use super::{Busy, short, who};
use mail_core::pgp::WithSecret;

/// A question the sheet is asking before it acts.
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

/// One key, its actions, and the question asked before one that cannot be undone.
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
    let asking = confirm();
    let (copy_key, save_key, secret_key, delete_key, gone) = (
        pgp.clone(),
        pgp.clone(),
        pgp.clone(),
        pgp.clone(),
        pgp.clone(),
    );
    let mut confirm = confirm;
    let actions = rsx! {
            div { class: "keys-row-acts",
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
            }
        };
    let title = TextLine::Runs(vec![
        TextRun::new(name, RunTone::Strong),
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
            match asking {
                Confirm::ExportSecret(asked) if asked == fingerprint => rsx! {
                    ConfirmBar {
                        sentence: "Anyone with this file can read your mail and sign as you. Keep it offline.".to_owned(),
                        act: "Save the secret key…".to_owned(),
                        confirm,
                        on_yes: move |_| {
                            confirm.set(Confirm::Nothing);
                            run.call(Job::SaveSecret(secret_key.clone()));
                        },
                    }
                },
                Confirm::Delete(asked) if asked == fingerprint => rsx! {
                    ConfirmBar {
                        sentence: format!(
                            "Deleting {id} also deletes its secret key. This cannot be undone."
                        ),
                        act: "Delete the key and its secret".to_owned(),
                        confirm,
                        on_yes: move |_| {
                            confirm.set(Confirm::Nothing);
                            run.call(Job::Delete(gone.clone(), WithSecret::Confirmed));
                        },
                    }
                },
                _ => rsx! {},
            }
        }
    }
}

/// The question asked again, with its two answers.
#[component]
pub(in crate::ui) fn ConfirmBar(
    sentence: String,
    act: String,
    confirm: Signal<Confirm>,
    on_yes: EventHandler<()>,
) -> Element {
    let mut confirm = confirm;
    rsx! {
            div { class: "keys-confirm", role: "alert",
                span { class: "say", Label { text: sentence, role: LabelRole::Secondary } }
                div { class: "acts",
                    Button {
                        label: "Cancel",
                        answers: Answers::Escape,
                        onclick: on_primary(move || confirm.set(Confirm::Nothing)),
        common: Common { aria_label: Some(format!("Cancel: {act}")), ..Common::default() },
    }
                    Button {
                        label: act.clone(),
                        role: ButtonRole::Destructive,
                        onclick: on_primary(move || on_yes.call(())),
        common: Common { aria_label: Some(act.to_string()), ..Common::default() },
    }
                }
            }
        }
}

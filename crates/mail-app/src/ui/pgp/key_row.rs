//! One OpenPGP key in the sheet, its actions, and the question asked again before an act that
//! cannot be undone — the bar the certificates' rows ask theirs in too.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{CertFingerprint, Fingerprint, KeySource, KeyTrust, PgpKey, SecretHeld};

use super::keys::Job;
use super::{Busy, short, who};
use crate::pgp::WithSecret;

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
    let row_class = if mine { "keys-row mine" } else { "keys-row" };
    let (copy_key, save_key, secret_key, delete_key, gone) = (
        pgp.clone(),
        pgp.clone(),
        pgp.clone(),
        pgp.clone(),
        pgp.clone(),
    );
    let mut confirm = confirm;
    rsx! {
        li { class: "{row_class}",
            div { class: "keys-text",
                b { "{name}" }
                span { class: "keys-fpr", "{id}" }
                span { class: "keys-meta", "{meta}" }
            }
            div { class: "keys-row-acts",
                if mine {
                    span { class: "keys-tag", "yours" }
                }
                button {
                    class: "ghost",
                    r#type: "button",
                    aria_label: "Copy the public key {id}",
                    onclick: move |_| {
                        if let Ok(armored) = crate::pgp::keys::export_public(&copy_key) {
                            super::super::hover::copy(&armored);
                            super::super::motion::tell(
                                "Public key copied".to_owned(),
                                super::super::motion::Follow::Nothing,
                            );
                        }
                    },
                    "Copy public"
                }
                button {
                    class: "ghost",
                    r#type: "button",
                    aria_label: "Save the public key {id}",
                    disabled: working,
                    onclick: move |_| run.call(Job::SavePublic(save_key.clone())),
                    "Save public…"
                }
                if mine {
                    button {
                        class: "ghost",
                        r#type: "button",
                        aria_label: "Export the secret key {id}",
                        disabled: working,
                        onclick: move |_| confirm.set(Confirm::ExportSecret(fingerprint)),
                        "Export secret…"
                    }
                }
                if !verified {
                    button {
                        class: "ghost",
                        r#type: "button",
                        aria_label: "Mark {id} verified",
                        title: "Only after comparing the fingerprint with its owner",
                        disabled: working,
                        onclick: move |_| run.call(Job::Verify(fingerprint)),
                        "Verify"
                    }
                }
                button {
                    class: "ghost danger",
                    r#type: "button",
                    aria_label: "Delete {id}",
                    disabled: working,
                    onclick: move |_| {
                        if delete_key.secret == SecretHeld::Held {
                            confirm.set(Confirm::Delete(fingerprint));
                        } else {
                            run.call(Job::Delete(delete_key.clone(), WithSecret::Refuse));
                        }
                    },
                    "Delete"
                }
            }
            match asking {
                Confirm::ExportSecret(asked) if asked == fingerprint => rsx! {
                    ConfirmBar {
                        sentence: format!(
                            "This writes the secret half of {id} to a file. Anyone who has that file can read your encrypted mail and sign as you: keep it offline, and never mail it."
                        ),
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
                            "Deleting {id} also deletes its secret key from your keyring. It cannot be recovered, and mail encrypted to it can never be read again. Export it first if you may need it."
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
            p { class: "say", "{sentence}" }
            div { class: "acts",
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "Cancel: {act}",
                    onclick: move |_| confirm.set(Confirm::Nothing),
                    "Cancel"
                }
                button {
                    class: "mini danger",
                    r#type: "button",
                    aria_label: "{act}",
                    onclick: move |_| on_yes.call(()),
                    "{act}"
                }
            }
        }
    }
}

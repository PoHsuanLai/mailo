//! The S/MIME half of an Encryption group: the certificates it lists, and what can be done to
//! each.
//!
//! Every change goes through [`mail_core::smime::certs`], the functions `mailo smime` uses, on the
//! sheet's blocking thread. A PKCS#12 identity file's password is asked for in the sheet only
//! when the file turns out to need one: typed into a [`Password`], moved into the one import
//! that uses it, and dropped with it — never drawn, never in a signal.

use ds::base::press::{PointerButton, Press};
use ds::components::content::text_runs::RunTone;
use ds::components::lists::row::size::RowSize;
use ds::components::menus::item::item::MenuItem as Line;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::icon::family::PlateFamily;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_client::Client;
use mail_core::SigningStore;
use mail_domain::{CertFingerprint, CertSource, KeyTrust, SecretHeld, SmimeCert};

use super::super::common::{add_mark, tile};
use super::super::press::{available, on_primary};
use super::super::sidebar::tagged;
use super::actions::{Act, Opened, RowActions, line};
use super::key_row::Confirm;
use super::keys::{Done, Job, write};
use super::{Busy, Seams, cert_short, whose};
use crate::ui::copy::menus;
use mail_core::password::Password;
use mail_core::pgp::WithSecret;
use mail_core::smime::SmimeError;

/// The certificates in the order the sheet lists them: the user's own — those whose private
/// key the keyring holds — first, then everyone else's, each group as the client orders it.
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
pub(in crate::ui) fn work(client: &Client, seams: &Seams, job: CertJob) -> Result<Done, String> {
    let secrets = seams.secrets.as_ref();
    let said = match job {
        CertJob::Import => {
            let Some(path) = (seams.pick)() else {
                return Ok(Done::Said("Nothing was imported.".to_owned()));
            };
            return import(client, secrets, &path, None);
        }
        CertJob::ImportWith(path, password) => {
            return import(client, secrets, &path, Some(password));
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
            client
                .certs()
                .trust(fingerprint, trust)
                .map_err(|e| e.to_string())?;
            match trust {
                KeyTrust::Verified => format!("Trusted {}", cert_short(fingerprint)),
                KeyTrust::Unverified => {
                    format!("{} is no longer trusted", cert_short(fingerprint))
                }
            }
        }
        CertJob::Delete(cert, with_secret) => {
            client
                .certs()
                .delete(secrets, &cert, with_secret)
                .map_err(|e| e.to_string())?;
            let said = format!("Deleted the certificate of {}", whose(&cert));
            if cert.secret == SecretHeld::Absent {
                return Ok(Done::Gone(
                    said,
                    Box::new(crate::ui::motion::Restore::Cert(cert)),
                ));
            }
            said
        }
    };
    Ok(Done::Said(said))
}

/// Import the file at `path`, with `password` for an identity file when one was typed. A file
/// that needs one and was given none comes back as a question. The password is dropped when
/// this returns.
fn import(
    client: &Client,
    secrets: &dyn SigningStore,
    path: &Path,
    password: Option<Password>,
) -> Result<Done, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let given = || password.as_ref().map(|typed| typed.expose().to_owned());
    let imported = match client.certs().import(secrets, &bytes, &given, Utc::now()) {
        Err(SmimeError::NoPassword) => return Ok(Done::Password(path.to_owned())),
        other => other.map_err(|e| e.to_string())?,
    };
    drop(password);
    let names: Vec<String> = imported.iter().map(|one| whose(&one.cert)).collect();
    if names.is_empty() {
        return Err(format!("{} holds no certificate.", path.display()));
    }
    Ok(Done::Said(
        match imported
            .iter()
            .find(|one| one.cert.secret == SecretHeld::Held)
        {
            Some(own) => format!("Imported your identity for {}", whose(&own.cert)),
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

/// The certificates' rows, and their Import row: what the page's list holds after the keys. A
/// file that needs a password is asked for by the page ([`super::key_row::Asking`]).
pub(super) fn cert_items(
    certs: Vec<SmimeCert>,
    confirm: Signal<Confirm>,
    run: Callback<Job>,
    busy: Busy,
) -> (Vec<ListItem<String>>, ListItem<String>) {
    let working = busy == Busy::Working;
    let import_label = "Import a certificate or identity…";
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
    let import = ListItem::row(
        IMPORT_CERT.to_owned(),
        import_label.to_owned(),
        rsx! {
            Row {
                leading: add_mark(),
                title: crate::ui::copy::fields::SMIME_CERTIFICATE,
                detail: Some(TextLine::from("A certificate, or your identity as a PKCS#12 file.")),
                size: RowSize::Settings,
                accessory: Accessory::Slot(rsx! {
                    Button {
                        label: "Import…",
                        availability: available(!working),
                        onclick: on_primary(move || run.call(Job::Cert(CertJob::Import))),
                        common: Common { aria_label: Some(import_label.to_owned()), ..Common::default() },
                    }
                }),
            }
        },
    );
    (items, import)
}

/// The key of the certificates' import row, which no fingerprint can be.
const IMPORT_CERT: &str = " import certificate";

/// The key of the import row, which no fingerprint can be.
pub(super) const IMPORT: &str = " import";

/// A row that says something and does nothing: an empty list's sentence.
pub(super) fn quiet_item(said: String) -> ListItem<String> {
    ListItem::row(
        " quiet".to_owned(),
        said.clone(),
        rsx! {
            Row {
                title: TextLine::Runs(vec![TextRun::new(said, RunTone::Faint)]),
                size: RowSize::Settings,
            }
        },
    )
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
    let mut confirm = confirm;
    let mut opened = use_signal(|| Opened::Closed);
    let can = available(!working);
    let trust = if trusted {
        line(Act::Distrust, menus::DONT_TRUST, None)
    } else {
        line(Act::Trust, menus::TRUST, Some(Icon::CircleCheck))
            .with_tip(menus::CHECK_THE_FINGERPRINT)
    };
    let items = vec![
        line(Act::Copy, menus::COPY_CERTIFICATE, Some(Icon::Copy)),
        line(Act::Save, menus::SAVE_CERTIFICATE, Some(Icon::Download)).with_availability(can),
        trust.with_availability(can),
        Line::Separator,
        line(Act::Delete, menus::DELETE, Some(Icon::Trash)).with_availability(can),
    ];
    let acted = cert.clone();
    let onpick = move |act: Act| match act {
        Act::Copy => {
            if let Ok(pem) = mail_core::smime::certs::export(&acted) {
                super::super::hover::copy(&pem);
                super::super::motion::tell(
                    "Certificate copied".to_owned(),
                    super::super::motion::Follow::Nothing,
                );
            }
        }
        Act::Save => run.call(Job::Cert(CertJob::Save(acted.clone()))),
        Act::Trust => run.call(Job::Cert(CertJob::Trust(fingerprint, KeyTrust::Verified))),
        Act::Distrust => run.call(Job::Cert(CertJob::Trust(fingerprint, KeyTrust::Unverified))),
        Act::Delete if acted.secret == SecretHeld::Held => {
            confirm.set(Confirm::DeleteCert(fingerprint));
        }
        Act::Delete => run.call(Job::Cert(CertJob::Delete(
            acted.clone(),
            WithSecret::Refuse,
        ))),
        Act::ExportSecret | Act::Verify => {}
    };
    let actions = rsx! {
        RowActions { label: crate::ui::copy::aria::actions_for(&id.to_string()), items, opened, onpick }
    };
    let title = TextLine::Runs(vec![
        TextRun::new(cert.subject.clone(), RunTone::Strong),
        TextRun::new(format!("  {id}"), RunTone::Faint),
    ]);
    rsx! {
        Row {
            leading: if mine {
                tile(Icon::Key, PlateFamily::Amber)
            } else {
                tile(Icon::Mail, PlateFamily::Blue)
            },
            title,
            detail: Some(TextLine::from(meta)),
            size: RowSize::Settings,
            accessory: Accessory::Slot(actions),
            // A right click anywhere on the row opens its menu at the pointer.
            onclick: move |press: Press| {
                if press.button == PointerButton::Secondary {
                    opened.set(Opened::At(press.at));
                }
            },
            common: tagged("mine", if mine { "yes" } else { "no" }),
        }
    }
}

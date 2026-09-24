//! S/MIME from the user's side: identities and certificates, reading protected mail, sending
//! it, and `mailo smime`.
//!
//! The pieces mirror OpenPGP's (`crate::pgp`): [`certs`] imports, exports, trusts and forgets
//! certificates and identities; [`read`] opens a message when the reader shows it; [`send`]
//! seals an outgoing one as Send is pressed. The cryptography is `mail_mime::smime`'s, the
//! keyring and the system's trust anchors are `mail_runtime::smime`'s, and this module decides
//! which certificates to use and says what happened.
//!
//! Revocation is never checked: nothing here fetches a CRL or asks an OCSP responder, so a
//! certificate its authority has revoked is believed as long as its chain and dates hold.

pub mod certs;
pub mod read;
pub mod send;

pub use certs::{Imported, cert_for, own_cert};
pub use read::{Protected, describe, open_bytes, open_message};
pub use send::{check, outgoing};

use crate::pgp::WithSecret;
use chrono::{DateTime, Utc};
use mail_domain::{CertFingerprint, CertSource, KeyTrust, MessageId, SecretHeld, SmimeCert};
use mail_mime::MimeError;
use mail_runtime::{RuntimeError, Secrets};
use mail_store::{SqliteStore, Store, StoreError};
use std::fmt::Write as _;

/// Why an S/MIME step could not be done, named so the caller can say what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum SmimeError {
    /// Encryption was asked for and these recipients have no certificate to encrypt to.
    #[error(
        "no S/MIME certificate to encrypt to for {}; nothing was sent. A signed message from \
         them brings theirs, or import one with `mailo smime import <file>`; or send without \
         --encrypt",
        .0.join(", ")
    )]
    NoCertFor(Vec<String>),
    /// Encryption was asked for with blind recipients, whose certificates every recipient would
    /// see named.
    #[error(
        "an encrypted message cannot have Bcc recipients ({}): every recipient would see the \
         blind ones' certificates named. Send them a separate message",
        .0.join(", ")
    )]
    BlindRecipients(Vec<String>),
    /// Signing or encrypting was asked for from an identity with no current certificate.
    #[error(
        "{0} has no current S/MIME certificate of its own; import your identity with \
         `mailo smime import <file.p12>`"
    )]
    NoOwnCert(String),
    /// The user's own certificate has a key mail cannot be encrypted to here.
    #[error(
        "your S/MIME certificate for {0} cannot be encrypted to (only RSA certificates for mail \
         can), so the sent copy could not be read; send without --encrypt"
    )]
    OwnCertCannotEncrypt(String),
    /// A draft asks for OpenPGP and S/MIME at once.
    #[error(
        "a message is protected with OpenPGP or with S/MIME, not both; choose one before sending"
    )]
    BothProtections,
    #[error("{0} is not the address of any of your identities")]
    NoIdentity(String),
    #[error(
        "the certificate {fingerprint} is for {}, none of which is one of your identities",
        addresses.join(", ")
    )]
    NotYours {
        fingerprint: CertFingerprint,
        addresses: Vec<String>,
    },
    #[error("no S/MIME certificate {0}")]
    NoCert(String),
    #[error(
        "the private key of {0} is in your keyring, and deleting it means mail encrypted to it \
         can never be read again. Keep the PKCS#12 file you imported it from, then delete with \
         --with-secret"
    )]
    SecretWouldBeLost(CertFingerprint),
    /// A PKCS#12 file was given without its password.
    #[error(
        "that PKCS#12 file needs its password: set MAILO_SMIME_PASSWORD, or run from a terminal \
         to be asked"
    )]
    NoPassword,
    #[error("{0}")]
    Mime(#[from] MimeError),
    #[error("{0}")]
    Runtime(#[from] RuntimeError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

/// A count that moves whenever the keys or certificates this process holds or trusts change —
/// imported, made, deleted, trusted, or learnt from arriving mail. Shared by OpenPGP and S/MIME
/// (`mail_runtime::epoch`): a cache of opened messages keeps the count it was made under and
/// opens them again when it has moved.
pub fn epoch() -> u64 {
    mail_runtime::epoch::keys()
}

fn epoch_changed() {
    mail_runtime::epoch::keys_changed();
}

/// A PKCS#12 file's password as the command line gets one: `MAILO_SMIME_PASSWORD` when it is
/// set, otherwise asked on the terminal with echo off, otherwise none.
pub fn terminal_password() -> Option<String> {
    if let Some(given) = std::env::var_os("MAILO_SMIME_PASSWORD") {
        return given.into_string().ok();
    }
    crate::pgp::ask_tty(
        "Password of the PKCS#12 file (not shown; MAILO_SMIME_PASSWORD also works): ",
    )
}

/// `mailo smime …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmimeCommand {
    /// Every certificate held.
    List,
    /// Import a PKCS#12 identity (asking its password) or a certificate file.
    Import { path: std::path::PathBuf },
    /// Print a certificate, PEM.
    Export { named: String },
    /// Forget a certificate.
    Delete {
        named: String,
        with_secret: WithSecret,
    },
    /// Trust a certificate as an anchor, or take that back.
    Trust {
        fingerprint: CertFingerprint,
        trust: KeyTrust,
    },
    /// Open one message and say what its S/MIME protection is.
    Show { message: MessageId },
}

/// Parse `mailo smime …`'s arguments.
pub fn parse(args: &[String]) -> Result<SmimeCommand, String> {
    let usage = "usage: mailo smime list | import <file> | export <fingerprint|address> | \
                 delete <fingerprint|address> [--with-secret] | trust <fingerprint> | \
                 untrust <fingerprint> | show <message-id>";
    let Some(verb) = args.first() else {
        return Err(usage.to_owned());
    };
    let arg = |i: usize, what: &str| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("smime {verb} needs {what}\n\n{usage}"))
    };
    let extra = |from: usize, allowed: &[&str]| -> Result<Vec<String>, String> {
        let rest: Vec<String> = args.get(from..).unwrap_or_default().to_vec();
        match rest.iter().find(|a| !allowed.contains(&a.as_str())) {
            Some(bad) => Err(format!("unknown option {bad:?}\n\n{usage}")),
            None => Ok(rest),
        }
    };
    let fingerprint = |raw: String| -> Result<CertFingerprint, String> {
        raw.parse()
            .map_err(|e| format!("{e}; `mailo smime list` lists them"))
    };
    match verb.as_str() {
        "list" | "certs" => {
            extra(1, &[])?;
            Ok(SmimeCommand::List)
        }
        "import" => {
            let path = arg(1, "a file")?;
            extra(2, &[])?;
            Ok(SmimeCommand::Import { path: path.into() })
        }
        "export" => {
            let named = arg(1, "a fingerprint or an address")?;
            extra(2, &[])?;
            Ok(SmimeCommand::Export { named })
        }
        "delete" => {
            let named = arg(1, "a fingerprint or an address")?;
            let rest = extra(2, &["--with-secret"])?;
            Ok(SmimeCommand::Delete {
                named,
                with_secret: if rest.is_empty() {
                    WithSecret::Refuse
                } else {
                    WithSecret::Confirmed
                },
            })
        }
        "trust" | "untrust" => {
            let raw = arg(1, "a fingerprint")?;
            extra(2, &[])?;
            Ok(SmimeCommand::Trust {
                fingerprint: fingerprint(raw)?,
                trust: if verb == "trust" {
                    KeyTrust::Verified
                } else {
                    KeyTrust::Unverified
                },
            })
        }
        "show" => {
            let raw = arg(1, "a message id")?;
            extra(2, &[])?;
            let id = raw
                .parse::<uuid::Uuid>()
                .map_err(|_| format!("{raw:?} is not a message id\n\n{usage}"))?;
            Ok(SmimeCommand::Show {
                message: MessageId::from_uuid(id),
            })
        }
        other => Err(format!("unknown smime command {other:?}\n\n{usage}")),
    }
}

/// Run a `mailo smime` command, as the CLI reports it. `password` is asked only for a PKCS#12
/// file.
pub fn run(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    password: &dyn Fn() -> Option<String>,
    command: &SmimeCommand,
    now: DateTime<Utc>,
) -> Result<String, String> {
    run_typed(store, secrets, password, command, now).map_err(|e| e.to_string())
}

fn run_typed(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    password: &dyn Fn() -> Option<String>,
    command: &SmimeCommand,
    now: DateTime<Utc>,
) -> Result<String, SmimeError> {
    match command {
        SmimeCommand::List => Ok(listing(&store.smime_certs()?)),
        SmimeCommand::Import { path } => {
            let bytes = std::fs::read(path)
                .map_err(|e| SmimeError::NoCert(format!("{}: {e}", path.display())))?;
            let imported = certs::import(store, secrets, &bytes, password, now)?;
            let mut out = String::new();
            for one in &imported {
                let _ = writeln!(
                    out,
                    "imported {} {}\n  {}",
                    one.cert.fingerprint,
                    one.cert.emails.join(", "),
                    one.cert.subject
                );
                if one.cert.secret == SecretHeld::Held {
                    let _ = writeln!(
                        out,
                        "  with its private key, now in your system keyring: mail from {} can be \
                         signed, and mail to it read",
                        one.cert.emails.join(", ")
                    );
                }
            }
            Ok(out)
        }
        SmimeCommand::Export { named } => Ok(certs::export(&certs::find(store, named)?)?),
        SmimeCommand::Delete { named, with_secret } => {
            let cert = certs::find(store, named)?;
            certs::delete(store, secrets, &cert, *with_secret)?;
            Ok(format!("deleted {}\n", cert.fingerprint))
        }
        SmimeCommand::Trust { fingerprint, trust } => {
            certs::trust(store, *fingerprint, *trust)?;
            Ok(match trust {
                KeyTrust::Verified => format!(
                    "{fingerprint} is now trusted: signatures by it, and by certificates it \
                     issued, are believed\n"
                ),
                KeyTrust::Unverified => format!("{fingerprint} is no longer trusted\n"),
            })
        }
        SmimeCommand::Show { message } => {
            let message = store.message(*message)?;
            match read::open_message(store, secrets, &message, now)? {
                None => Ok("no S/MIME protection\n".to_owned()),
                Some(protected) => {
                    let mut out = describe(&protected);
                    if let Some(signer) = &protected.signer {
                        let _ = writeln!(
                            out,
                            "    signer {}\n      {}\n      issued by {}\n      valid {} to {}",
                            signer.fingerprint,
                            signer.subject,
                            signer.issuer,
                            signer.not_before.format("%Y-%m-%d"),
                            signer.not_after.format("%Y-%m-%d")
                        );
                    }
                    if out.is_empty() {
                        out.push_str("    signed or encrypted, and nothing more to say\n");
                    }
                    Ok(out)
                }
            }
        }
    }
}

/// The certificate table as `mailo smime list` prints it.
pub fn listing(certs: &[SmimeCert]) -> String {
    if certs.is_empty() {
        return "no S/MIME certificates; import your identity with \
                `mailo smime import <file.p12>`\n"
            .to_owned();
    }
    let mut out = String::new();
    for cert in certs {
        let source = match cert.source {
            CertSource::Identity => "your identity",
            CertSource::Imported => "imported",
            CertSource::Received => "from signed mail",
        };
        let trust = match cert.trust {
            KeyTrust::Verified => ", trusted by you",
            KeyTrust::Unverified => "",
        };
        let key = match cert.secret {
            SecretHeld::Held => ", private key in keyring",
            SecretHeld::Absent => "",
        };
        let who = if cert.emails.is_empty() {
            cert.subject.clone()
        } else {
            cert.emails.join(", ")
        };
        let _ = writeln!(
            out,
            "{}  {who}\n    {source}{trust}{key}, valid {} to {}, issued by {}",
            cert.fingerprint,
            cert.not_before.format("%Y-%m-%d"),
            cert.not_after.format("%Y-%m-%d"),
            cert.issuer
        );
    }
    out
}

//! `mailo smime`: S/MIME certificates at the terminal, and the words for what reading a
//! protected message found.
//!
//! The certificates and the opening are [`mail_core::smime`]'s; this is the part that reads the
//! arguments, asks for a PKCS#12 password on the terminal, and says what happened.

use chrono::{DateTime, Utc};
use mail_core::Environment;
use mail_core::SigningStore;
use mail_core::error::{CoreError, UsageError};
use mail_core::pgp::WithSecret;
use mail_core::smime::{Protected, SmimeError, certs, read};
use mail_core::{SqliteStore, Store};
use mail_domain::*;
use std::fmt::Write as _;

/// A PKCS#12 file's password as the command line gets one: `MAILO_SMIME_PASSWORD` (the
/// environment's `smime_password`) when it is set, otherwise asked on the terminal with echo off,
/// otherwise none.
pub fn terminal_password(env: &Environment) -> Option<String> {
    if let Some(given) = &env.smime_password {
        return given.clone().into_string().ok();
    }
    super::pgp::ask_tty(
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
pub fn parse(args: &[String]) -> Result<SmimeCommand, CoreError> {
    let usage = "usage: mailo smime list | import <file> | export <fingerprint|address> | \
                 delete <fingerprint|address> [--with-secret] | trust <fingerprint> | \
                 untrust <fingerprint> | show <message-id>";
    let Some(verb) = args.first() else {
        return Err(UsageError::Synopsis(usage).into());
    };
    let arg = |i: usize, what: &'static str| {
        args.get(i).cloned().ok_or_else(|| UsageError::Missing {
            command: "smime",
            verb: verb.clone(),
            what,
            usage,
        })
    };
    let extra = |from: usize, allowed: &[&str]| -> Result<Vec<String>, UsageError> {
        let rest: Vec<String> = args.get(from..).unwrap_or_default().to_vec();
        match rest.iter().find(|a| !allowed.contains(&a.as_str())) {
            Some(bad) => Err(UsageError::UnknownOption {
                option: bad.clone(),
                usage,
            }),
            None => Ok(rest),
        }
    };
    let fingerprint = |raw: String| -> Result<CertFingerprint, UsageError> {
        raw.parse().map_err(UsageError::SmimeFingerprint)
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
                .map_err(|_| UsageError::NotAMessageId {
                    raw: raw.clone(),
                    usage,
                })?;
            Ok(SmimeCommand::Show {
                message: MessageId::from_uuid(id),
            })
        }
        other => Err(UsageError::UnknownCommand {
            command: "smime",
            verb: other.to_owned(),
            usage,
        }
        .into()),
    }
}

/// Run a `mailo smime` command, as the CLI reports it. `password` is asked only for a PKCS#12
/// file.
pub fn run(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    password: &dyn Fn() -> Option<String>,
    command: &SmimeCommand,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    run_typed(store, secrets, password, command, now).map_err(CoreError::from)
}

fn run_typed(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
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

/// What `mailo show` says about an S/MIME message, one line each, before its text.
pub fn describe(protected: &Protected) -> String {
    let mut out = String::new();
    match &protected.encryption {
        SmimeEncryption::NotEncrypted => {}
        SmimeEncryption::Decrypted => out.push_str("    S/MIME encrypted; decrypted for reading\n"),
        SmimeEncryption::CannotDecrypt { to } => out.push_str(&format!(
            "    S/MIME encrypted to certificates you hold no key for ({}); it cannot be read \
             here\n",
            to.join("; ")
        )),
        SmimeEncryption::Unreadable { why } => {
            out.push_str(&format!(
                "    S/MIME encrypted, and could not be read: {why}\n"
            ));
        }
    }
    let part = |coverage: &Coverage| match coverage {
        Coverage::Whole => "",
        Coverage::Part => " — only part of the message is signed; the rest could say anything",
    };
    let who = protected
        .signer
        .as_ref()
        .map(|c| {
            c.emails
                .first()
                .cloned()
                .unwrap_or_else(|| c.subject.clone())
        })
        .unwrap_or_default();
    match &protected.verification {
        SmimeVerification::NoSignature => {}
        SmimeVerification::Good { signer, coverage } => out.push_str(&format!(
            "    good S/MIME signature by {who} (certificate {signer}){}\n",
            part(coverage)
        )),
        SmimeVerification::Doubtful {
            signer,
            problems,
            coverage,
        } => {
            let said: Vec<String> = problems.iter().map(problem).collect();
            out.push_str(&format!(
                "    S/MIME signature by {who} (certificate {signer}) matches the message, but \
                 {}{}\n",
                said.join("; "),
                part(coverage)
            ));
        }
        SmimeVerification::Bad { why, coverage } => out.push_str(&format!(
            "    BAD S/MIME SIGNATURE: {}{}\n",
            bad(why),
            part(coverage)
        )),
        SmimeVerification::UnknownSigner { coverage } => out.push_str(&format!(
            "    S/MIME signed by a certificate neither the message nor you hold; it cannot be \
             checked{}\n",
            part(coverage)
        )),
    }
    out
}

fn problem(problem: &CertProblem) -> String {
    match problem {
        CertProblem::Untrusted => "its certificate is not issued by an authority you trust \
                                   (trust it with `mailo smime trust <fingerprint>`)"
            .to_owned(),
        CertProblem::Expired { not_after } => {
            format!(
                "a certificate in it expired on {}",
                not_after.format("%Y-%m-%d")
            )
        }
        CertProblem::NotYetValid { not_before } => format!(
            "a certificate in it was not valid until {}",
            not_before.format("%Y-%m-%d")
        ),
        CertProblem::NotForEmail => "its certificate is not for signing mail".to_owned(),
        CertProblem::NotFrom { from } if from.is_empty() => {
            "the message names no sender to check the certificate against".to_owned()
        }
        CertProblem::NotFrom { from } => {
            format!("its certificate is not for {from}, who the message says it is from")
        }
    }
}

fn bad(why: &BadSignature) -> String {
    match why {
        BadSignature::Altered => "the message was changed after it was signed".to_owned(),
        BadSignature::Forged => {
            "the signature does not match its certificate: forged, or damaged".to_owned()
        }
        BadSignature::Weak { algorithm } => {
            format!("made with {algorithm}, which is too weak to mean anything now")
        }
        BadSignature::Unsupported { algorithm } => {
            format!("made with {algorithm}, which this client does not check")
        }
        BadSignature::Malformed { why } => format!("unreadable: {why}"),
    }
}

//! `mailo pgp`: OpenPGP keys at the terminal, and the words for what reading a protected message
//! found.
//!
//! The keys, the lookups and the opening are [`mail_core::pgp`]'s; this is the part that reads
//! the arguments, asks for a passphrase on the terminal, and says what happened.

use chrono::{DateTime, Utc};
use mail_core::Environment;
use mail_core::error::{CoreError, UsageError};
use mail_core::pgp::keys::{self, WithSecret};
use mail_core::pgp::{Discovered, Discovery, PgpError, Protected};
use mail_domain::*;
use mail_runtime::SigningStore;
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

/// The passphrase for `fingerprint` as the command line gets one: `MAILO_PGP_PASSPHRASE` (the
/// environment's `pgp_passphrase`) when it is set, otherwise asked on the terminal with echo off,
/// otherwise none.
///
/// The environment variable is for scripts; it is visible to other processes of the same user,
/// which the prompt is not, and the prompt says so.
pub fn terminal_passphrase(env: &Environment, fingerprint: Fingerprint) -> Option<String> {
    if let Some(given) = &env.pgp_passphrase {
        return given.clone().into_string().ok();
    }
    ask_tty(&format!(
        "Passphrase for OpenPGP key {} (not shown; MAILO_PGP_PASSPHRASE also works): ",
        keys::grouped(fingerprint)
    ))
}

/// A line read from the controlling terminal with echo off. `None` without a terminal.
#[cfg(unix)]
pub(crate) fn ask_tty(prompt: &str) -> Option<String> {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let stty = |arg: &str| {
        Command::new("stty")
            .arg(arg)
            .stdin(
                tty.try_clone()
                    .map(Stdio::from)
                    .unwrap_or_else(|_| Stdio::null()),
            )
            .status()
            .is_ok_and(|s| s.success())
    };
    // No echo, or no prompt: a passphrase shown on the screen is worse than one not asked for.
    if !stty("-echo") {
        return None;
    }
    let mut out = tty.try_clone().ok()?;
    let _ = write!(out, "{prompt}");
    let _ = out.flush();
    let mut line = String::new();
    let read = std::io::BufReader::new(&tty).read_line(&mut line);
    stty("echo");
    let _ = writeln!(out);
    read.ok()?;
    Some(line.trim_end_matches(['\r', '\n']).to_owned())
}

/// A line read from the console with echo off. `None` without a console.
///
/// Windows has no `/dev/tty` and no `stty`; `rpassword` opens the console (`CONIN$`) and turns
/// its echo off for the line, which is the same promise the Unix branch keeps.
#[cfg(windows)]
pub(crate) fn ask_tty(prompt: &str) -> Option<String> {
    rpassword::prompt_password(prompt).ok()
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn ask_tty(_: &str) -> Option<String> {
    None
}

/// `mailo pgp …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PgpCommand {
    /// Every key held.
    Keys,
    /// Make a key for one of the user's identities.
    Generate { address: String },
    /// Import every key in a file.
    Import { path: std::path::PathBuf },
    /// Print a key, armored; with `secret`, its secret half.
    Export { named: String, secret: Secret },
    /// Forget a key.
    Delete {
        named: String,
        with_secret: WithSecret,
    },
    /// Ask the address's domain for its key (Web Key Directory). Needs the network, so the
    /// binary dispatches it; see [`lookup`].
    Lookup { address: String },
    /// Mark a key as verified by the user.
    Verify { fingerprint: Fingerprint },
}

/// Whether `export` prints the secret key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Secret {
    Public,
    /// `--secret`, asked for explicitly.
    Included,
}

/// Parse `mailo pgp …`'s arguments.
pub fn parse(args: &[String]) -> Result<PgpCommand, CoreError> {
    let usage = "usage: mailo pgp keys | generate <address> | import <file> | \
                 export <fingerprint|address> [--secret] | delete <fingerprint|address> \
                 [--with-secret] | lookup <address> | verify <fingerprint>";
    let arg = |i: usize, what: &'static str| {
        args.get(i).cloned().ok_or_else(|| UsageError::Missing {
            command: "pgp",
            verb: args[0].clone(),
            what,
            usage,
        })
    };
    let Some(verb) = args.first() else {
        return Err(UsageError::Synopsis(usage).into());
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
    match verb.as_str() {
        "keys" | "list" => {
            extra(1, &[])?;
            Ok(PgpCommand::Keys)
        }
        "generate" => {
            let address = arg(1, "an identity's address")?;
            extra(2, &[])?;
            Ok(PgpCommand::Generate { address })
        }
        "import" => {
            let path = arg(1, "a file")?;
            extra(2, &[])?;
            Ok(PgpCommand::Import { path: path.into() })
        }
        "export" => {
            let named = arg(1, "a fingerprint or an address")?;
            let rest = extra(2, &["--secret"])?;
            Ok(PgpCommand::Export {
                named,
                secret: if rest.is_empty() {
                    Secret::Public
                } else {
                    Secret::Included
                },
            })
        }
        "delete" => {
            let named = arg(1, "a fingerprint or an address")?;
            let rest = extra(2, &["--with-secret"])?;
            Ok(PgpCommand::Delete {
                named,
                with_secret: if rest.is_empty() {
                    WithSecret::Refuse
                } else {
                    WithSecret::Confirmed
                },
            })
        }
        "lookup" => {
            let address = arg(1, "an address")?;
            extra(2, &[])?;
            Ok(PgpCommand::Lookup { address })
        }
        "verify" => {
            let raw = arg(1, "a fingerprint")?;
            extra(2, &[])?;
            let fingerprint = raw.parse().map_err(UsageError::PgpFingerprint)?;
            Ok(PgpCommand::Verify { fingerprint })
        }
        other => Err(UsageError::UnknownCommand {
            command: "pgp",
            verb: other.to_owned(),
            usage,
        }
        .into()),
    }
}

/// Run a `mailo pgp` command that needs no network, as the CLI reports it.
pub fn run(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    command: &PgpCommand,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    run_typed(store, secrets, command, now).map_err(CoreError::from)
}

fn run_typed(
    store: &SqliteStore,
    secrets: &dyn SigningStore,
    command: &PgpCommand,
    now: DateTime<Utc>,
) -> Result<String, PgpError> {
    match command {
        PgpCommand::Keys => Ok(listing(&store.pgp_keys()?)),
        PgpCommand::Generate { address } => {
            let key = keys::generate(store, secrets, address, now)?;
            Ok(format!(
                "made an OpenPGP key for {address}\n  {}\n\
                 its secret half is in your system keyring. Mail from {address} now carries it \
                 in an Autocrypt header;\nshare it with `mailo pgp export {address}`\n",
                keys::grouped(key.fingerprint)
            ))
        }
        PgpCommand::Import { path } => {
            let bytes = std::fs::read(path)
                .map_err(|e| PgpError::NoKey(format!("{}: {e}", path.display())))?;
            let imported = keys::import(store, secrets, &bytes, now)?;
            let mut out = String::new();
            for one in &imported {
                let who = one.key.emails.join(", ");
                let _ = writeln!(out, "imported {} {who}", one.key.fingerprint);
                match one.secret {
                    None => {}
                    Some(mail_mime::openpgp::Protection::Open) => {
                        let _ = writeln!(out, "  with its secret half, now in your keyring");
                    }
                    Some(mail_mime::openpgp::Protection::Passphrase) => {
                        let _ = writeln!(
                            out,
                            "  with its secret half, now in your keyring, still behind its \
                             passphrase: you will be asked for it when it signs or decrypts"
                        );
                    }
                }
            }
            Ok(out)
        }
        PgpCommand::Export { named, secret } => {
            let key = keys::find(store, named)?;
            match secret {
                Secret::Public => keys::export_public(&key),
                Secret::Included => {
                    let armored = keys::export_secret(store, secrets, &key)?;
                    // Before the armor, where every OpenPGP reader skips it, so the output is
                    // still a key file when redirected into one.
                    Ok(format!(
                        "WARNING: this is the SECRET half of {}. Anyone who has it can read \
                         your encrypted mail and sign as you.\nKeep it offline; never mail it \
                         or paste it anywhere.\n\n{armored}",
                        key.fingerprint
                    ))
                }
            }
        }
        PgpCommand::Delete { named, with_secret } => {
            let key = keys::find(store, named)?;
            keys::delete(store, secrets, &key, *with_secret)?;
            Ok(format!("deleted {}\n", key.fingerprint))
        }
        PgpCommand::Verify { fingerprint } => {
            keys::verify(store, *fingerprint)?;
            Ok(format!(
                "marked {} as verified\n  compare it with its owner: {}\n",
                fingerprint,
                keys::grouped(*fingerprint)
            ))
        }
        PgpCommand::Lookup { .. } => Err(PgpError::NoKey(
            "lookup needs the network and is dispatched before this point".to_owned(),
        )),
    }
}

/// The key table as `mailo pgp keys` prints it.
pub fn listing(keys: &[mail_domain::PgpKey]) -> String {
    if keys.is_empty() {
        return "no OpenPGP keys; make one with `mailo pgp generate <your address>`\n".to_owned();
    }
    let mut out = String::new();
    for key in keys {
        let mine = match key.secret {
            SecretHeld::Held => "yours",
            SecretHeld::Absent => "",
        };
        let trust = match key.trust {
            KeyTrust::Verified => "verified",
            KeyTrust::Unverified => "unverified",
        };
        let source = match key.source {
            KeySource::Generated => "generated",
            KeySource::Imported => "imported",
            KeySource::Wkd => "web key directory",
            KeySource::Autocrypt => "autocrypt",
            KeySource::Gossip => "gossip",
        };
        let _ = writeln!(
            out,
            "{}  {}\n    {source}, {trust}{}{mine}, last seen {}",
            key.fingerprint,
            key.emails.join(", "),
            if mine.is_empty() { "" } else { ", " },
            key.last_seen.format("%Y-%m-%d")
        );
    }
    out
}

/// What `mailo show` says about a protected message, one line each, before its text.
pub fn describe(protected: &Protected) -> String {
    let mut out = String::new();
    match &protected.encryption {
        Encryption::NotEncrypted => {}
        Encryption::Decrypted => out.push_str("    encrypted; decrypted for reading\n"),
        Encryption::CannotDecrypt { to } => {
            let ids: Vec<String> = to.iter().map(ToString::to_string).collect();
            out.push_str(&format!(
                "    encrypted to keys you do not hold ({}); it cannot be read here\n",
                ids.join(", ")
            ));
        }
        Encryption::Locked { key } => out.push_str(&format!(
            "    encrypted to your key {key}, which needs its passphrase \
             (set MAILO_PGP_PASSPHRASE, or run from a terminal to be asked)\n"
        )),
        Encryption::Unreadable { why } => {
            out.push_str(&format!("    encrypted, and could not be read: {why}\n"));
        }
    }
    let part = |coverage: &Coverage| match coverage {
        Coverage::Whole => "",
        Coverage::Part => " — only part of the message is signed; the rest could say anything",
    };
    match &protected.verification {
        Verification::NoSignature => {}
        Verification::Good {
            signer,
            trust,
            coverage,
        } => {
            let who = protected
                .signer
                .as_ref()
                .and_then(|k| k.emails.first().cloned())
                .unwrap_or_default();
            let trust = match trust {
                KeyTrust::Verified => "verified",
                KeyTrust::Unverified => "not verified by you",
            };
            out.push_str(&format!(
                "    good signature by {who} ({signer}, {trust}){}\n",
                part(coverage)
            ));
        }
        Verification::Bad { coverage } => out.push_str(&format!(
            "    BAD SIGNATURE: the message was changed after it was signed, or the signature is \
             forged{}\n",
            part(coverage)
        )),
        Verification::UnknownKey { issuer, coverage } => out.push_str(&format!(
            "    signed by key {issuer}, which you do not have; `mailo pgp lookup <address>` or \
             `mailo pgp import <file>` to check it{}\n",
            part(coverage)
        )),
    }
    out
}

/// What `mailo pgp lookup` says about the answer from `address`'s domain.
pub fn lookup(found: Option<&PgpKey>, address: &str) -> String {
    match found {
        Some(key) => format!(
            "found {} for {address} in its domain's Web Key Directory\n  {}\n\
             not verified: compare the fingerprint with its owner, then `mailo pgp verify {}`\n",
            key.fingerprint,
            keys::grouped(key.fingerprint),
            key.fingerprint
        ),
        None => format!("{address}'s domain publishes no OpenPGP key for it\n"),
    }
}

/// What `mailo compose --encrypt` says about the addresses it had to ask a domain for a key.
pub fn discovered(found: &[Discovered]) -> String {
    let mut out = String::new();
    for one in found {
        let address = &one.address;
        match &one.outcome {
            Discovery::Found(key) => {
                let _ = writeln!(
                    out,
                    "found {address}'s key {} in its domain's Web Key Directory",
                    key.fingerprint
                );
            }
            Discovery::NoKey => {
                let _ = writeln!(
                    out,
                    "no OpenPGP key for {address}: the message cannot be sent encrypted until \
                     one is imported"
                );
            }
            Discovery::Failed(e) => {
                let _ = writeln!(out, "could not look up {address}'s key: {e}");
            }
        }
    }
    out
}

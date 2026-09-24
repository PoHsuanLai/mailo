//! OpenPGP from the user's side: keys, reading protected mail, sending it, and `mailo pgp`.
//!
//! The pieces: [`keys`] makes, imports, exports and forgets keys; [`read`] opens a message when
//! the reader shows it; [`send`] seals an outgoing one as Send is pressed. The cryptography is
//! `mail_mime::openpgp`'s, the keyring and the Web Key Directory are `mail_runtime`'s, and this
//! module decides which keys to use and says what happened.

pub mod keys;
pub mod read;
pub mod send;

pub use keys::{Imported, WithSecret, own_key};
pub use read::{Protected, describe, open_bytes, open_message};
pub use send::{check, outgoing};

use chrono::{DateTime, Utc};
use mail_domain::{Fingerprint, KeySource, KeyTrust, SecretHeld};
use mail_mime::MimeError;
use mail_runtime::{RuntimeError, Secrets};
use mail_store::{SqliteStore, Store, StoreError};
use std::fmt::Write as _;

/// Asked for the passphrase of the user's key with this fingerprint, when a protected key has
/// to sign or decrypt. `None` is "not given": the send is refused or the message stays locked.
///
/// The CLI asks on the terminal ([`terminal_passphrase`]); the window asks in a dialog; a test
/// answers from a table. Nothing is asked for a key that has no passphrase.
pub type Ask<'a> = &'a dyn Fn(Fingerprint) -> Option<String>;

/// The [`Ask`] that never has an answer: for callers with nobody to ask.
pub fn no_passphrase(_: Fingerprint) -> Option<String> {
    None
}

/// Why an OpenPGP step could not be done, named so the caller can say what to do about it.
#[derive(Debug, thiserror::Error)]
pub enum PgpError {
    /// Encryption was asked for and these recipients have no key to encrypt to.
    #[error(
        "no OpenPGP key for {}; nothing was sent. Find one with `mailo pgp lookup <address>`, \
         import one with `mailo pgp import <file>`, or send without --encrypt",
        .0.join(", ")
    )]
    NoKeyFor(Vec<String>),
    /// Encryption was asked for with blind recipients, whose key ids every recipient would see.
    #[error(
        "an encrypted message cannot have Bcc recipients ({}): every recipient would see the \
         blind ones' key ids. Send them a separate message",
        .0.join(", ")
    )]
    BlindRecipients(Vec<String>),
    /// Signing or encrypting was asked for from an identity with no key of its own.
    #[error("{0} has no OpenPGP key; make one with `mailo pgp generate {0}`")]
    NoOwnKey(String),
    /// A protected key's passphrase was not given, or was wrong.
    #[error("the OpenPGP key {0} needs its passphrase; none was given, or it was wrong")]
    Locked(Fingerprint),
    #[error("{0} is not the address of any of your identities")]
    NoIdentity(String),
    /// A draft asks for OpenPGP and S/MIME at once (`crate::smime::send`).
    #[error(
        "a message is protected with OpenPGP or with S/MIME, not both; choose one before sending"
    )]
    BothProtections,
    #[error("{address} already has an OpenPGP key, {fingerprint}")]
    AlreadyHasKey {
        address: String,
        fingerprint: Fingerprint,
    },
    #[error(
        "the secret key {fingerprint} is for {}, none of which is one of your identities",
        addresses.join(", ")
    )]
    NotYours {
        fingerprint: Fingerprint,
        addresses: Vec<String>,
    },
    #[error("no OpenPGP key {0}")]
    NoKey(String),
    #[error("the secret half of {0} is not in your keyring")]
    NoSecret(Fingerprint),
    #[error(
        "the secret half of {0} is in your keyring, and deleting it means mail encrypted to it \
         can never be read again. Export it first (`mailo pgp export {0} --secret`), then delete \
         with --with-secret"
    )]
    SecretWouldBeLost(Fingerprint),
    #[error("{0}")]
    Mime(MimeError),
    #[error("{0}")]
    Runtime(#[from] RuntimeError),
    #[error("store: {0}")]
    Store(#[from] StoreError),
}

impl From<MimeError> for PgpError {
    fn from(e: MimeError) -> Self {
        match e {
            MimeError::KeyLocked(fingerprint) => PgpError::Locked(fingerprint),
            other => PgpError::Mime(other),
        }
    }
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

/// The passphrase for `fingerprint` as the command line gets one: `MAILO_PGP_PASSPHRASE` when it
/// is set, otherwise asked on the terminal with echo off, otherwise none.
///
/// The environment variable is for scripts; it is visible to other processes of the same user,
/// which the prompt is not, and the prompt says so.
pub fn terminal_passphrase(fingerprint: Fingerprint) -> Option<String> {
    if let Some(given) = std::env::var_os("MAILO_PGP_PASSPHRASE") {
        return given.into_string().ok();
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

#[cfg(not(unix))]
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
pub fn parse(args: &[String]) -> Result<PgpCommand, String> {
    let usage = "usage: mailo pgp keys | generate <address> | import <file> | \
                 export <fingerprint|address> [--secret] | delete <fingerprint|address> \
                 [--with-secret] | lookup <address> | verify <fingerprint>";
    let arg = |i: usize, what: &str| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("pgp {} needs {what}\n\n{usage}", args[0]))
    };
    let Some(verb) = args.first() else {
        return Err(usage.to_owned());
    };
    let extra = |from: usize, allowed: &[&str]| -> Result<Vec<String>, String> {
        let rest: Vec<String> = args.get(from..).unwrap_or_default().to_vec();
        match rest.iter().find(|a| !allowed.contains(&a.as_str())) {
            Some(bad) => Err(format!("unknown option {bad:?}\n\n{usage}")),
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
            let fingerprint = raw
                .parse()
                .map_err(|e| format!("{e}; `mailo pgp keys` lists them"))?;
            Ok(PgpCommand::Verify { fingerprint })
        }
        other => Err(format!("unknown pgp command {other:?}\n\n{usage}")),
    }
}

/// Run a `mailo pgp` command that needs no network, as the CLI reports it.
pub fn run(
    store: &SqliteStore,
    secrets: &dyn Secrets,
    command: &PgpCommand,
    now: DateTime<Utc>,
) -> Result<String, String> {
    run_typed(store, secrets, command, now).map_err(|e| e.to_string())
}

fn run_typed(
    store: &SqliteStore,
    secrets: &dyn Secrets,
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

/// Ask `address`'s domain for its key, and keep it when there is one. Needs the network.
pub fn lookup(store: &SqliteStore, address: &str, now: DateTime<Utc>) -> Result<String, String> {
    let found = lookup_address(store, address, now).map_err(|e| e.to_string())?;
    Ok(match found {
        Some(key) => format!(
            "found {} for {address} in its domain's Web Key Directory\n  {}\n\
             not verified: compare the fingerprint with its owner, then `mailo pgp verify {}`\n",
            key.fingerprint,
            keys::grouped(key.fingerprint),
            key.fingerprint
        ),
        None => format!("{address}'s domain publishes no OpenPGP key for it\n"),
    })
}

/// [`lookup`]'s work: the key, kept, or `None`.
pub fn lookup_address(
    store: &SqliteStore,
    address: &str,
    now: DateTime<Utc>,
) -> Result<Option<mail_domain::PgpKey>, PgpError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| RuntimeError::Io(e.to_string()))?;
    let http = mail_runtime::wkd::client()?;
    let Some(cert) = runtime.block_on(mail_runtime::wkd::lookup(&http, address))? else {
        return Ok(None);
    };
    let kept = store.put_pgp_key(cert.record(KeySource::Wkd, now, &[address]))?;
    epoch_changed();
    Ok(Some(kept))
}

/// The To and Cc addresses of an encrypted draft that have no key yet, each asked of its
/// domain's Web Key Directory: what `mailo compose --encrypt` does before it reports.
pub fn discover(store: &SqliteStore, addresses: &[String], now: DateTime<Utc>) -> String {
    let mut out = String::new();
    for address in addresses {
        if matches!(keys::key_for(store, address, now), Ok(Some(_))) {
            continue;
        }
        match lookup_address(store, address, now) {
            Ok(Some(key)) => {
                let _ = writeln!(
                    out,
                    "found {address}'s key {} in its domain's Web Key Directory",
                    key.fingerprint
                );
            }
            Ok(None) => {
                let _ = writeln!(
                    out,
                    "no OpenPGP key for {address}: the message cannot be sent encrypted until \
                     one is imported"
                );
            }
            Err(e) => {
                let _ = writeln!(out, "could not look up {address}'s key: {e}");
            }
        }
    }
    out
}

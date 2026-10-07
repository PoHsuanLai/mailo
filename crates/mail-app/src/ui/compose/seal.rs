//! Signing and encrypting from the composer: the warning bar that says, in words, what stands
//! between the protection chosen and Send, and the send that seals.
//!
//! What stands between them is [`mail_core::pgp::check`]'s or [`mail_core::smime::check`]'s answer, the
//! same one `mailo compose` gives, asked of the store as Send is pressed and never of the
//! keyring. Signing and encrypting themselves happen in [`super::life::queue`], off the thread
//! that draws, with an OpenPGP key's passphrase asked for in the bar when it has one. S/MIME's
//! private keys have none.

use ds::components::overlays::inline_banner::InlineBanner;
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{Draft, Fingerprint};
use mail_store::SqliteStore;

use super::super::pgp::{Busy, Passphrase, Scheme, Tried, WRONG, seams, short};
use super::super::press::on_primary;
use mail_core::password::Password;
use mail_core::pgp::PgpError;
use mail_core::smime::SmimeError;

/// What the warning bar says about signing and encrypting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum SealBar {
    Clear,
    /// OpenPGP encryption was asked for, and these recipients have no key to encrypt to.
    NoKeyFor(Vec<String>),
    /// The sending identity has no OpenPGP key of its own to sign or encrypt with.
    NoOwnKey(String),
    /// S/MIME encryption was asked for, and these recipients have no certificate to encrypt to.
    NoCertFor(Vec<String>),
    /// The sending identity has no current S/MIME certificate of its own.
    NoOwnCert(String),
    /// The sending identity's S/MIME certificate cannot be encrypted to, so the sent copy could
    /// not be read.
    OwnCertCannotEncrypt(String),
    /// Encryption was asked for with these blind recipients.
    Blind(Scheme, Vec<String>),
    /// Their domains are being asked for these recipients' OpenPGP keys.
    Looking(Vec<String>),
    /// The identity's OpenPGP key needs its passphrase.
    Locked {
        key: Fingerprint,
        tried: Tried,
    },
    /// Being signed and encrypted.
    Sealing,
}

/// What stands between `draft` and being sent as it asks, as the bar says it; `Clear` when
/// nothing does. Reads the store only.
pub(in crate::ui) fn sealable(
    store: &SqliteStore,
    draft: &Draft,
    now: DateTime<Utc>,
) -> Result<SealBar, String> {
    let identity =
        mail_core::compose::identity_of(store, draft.account.clone(), Some(draft.identity))?;
    match mail_core::pgp::check(store, draft, &identity, now) {
        Ok(()) => {}
        Err(PgpError::NoKeyFor(addresses)) => return Ok(SealBar::NoKeyFor(addresses)),
        Err(PgpError::NoOwnKey(address)) => return Ok(SealBar::NoOwnKey(address)),
        Err(PgpError::BlindRecipients(addresses)) => {
            return Ok(SealBar::Blind(Scheme::OpenPgp, addresses));
        }
        Err(other) => return Err(other.to_string()),
    }
    Ok(
        match mail_core::smime::check(store, draft, &identity, now) {
            Ok(()) => SealBar::Clear,
            Err(SmimeError::NoCertFor(addresses)) => SealBar::NoCertFor(addresses),
            Err(SmimeError::NoOwnCert(address)) => SealBar::NoOwnCert(address),
            Err(SmimeError::OwnCertCannotEncrypt(address)) => {
                SealBar::OwnCertCannotEncrypt(address)
            }
            Err(SmimeError::BlindRecipients(addresses)) => SealBar::Blind(Scheme::Smime, addresses),
            Err(other) => return Err(other.to_string()),
        },
    )
}

/// Ask each address's domain for its OpenPGP key, then ask again what stands in the way.
/// Blocks on the network: run on a blocking thread, from "Look up keys" only. Returns the bar,
/// and what the lookups found in words.
pub(in crate::ui) fn look_up(
    store: &SqliteStore,
    lookup: &super::super::pgp::Lookup,
    draft: &Draft,
    addresses: &[String],
    now: DateTime<Utc>,
) -> Result<(SealBar, String), String> {
    let mut said = Vec::new();
    for address in addresses {
        said.push(match lookup(store, address) {
            Ok(Some(key)) => format!("found {address}'s key {}", short(key.fingerprint)),
            Ok(None) => format!("{address}'s domain publishes no key"),
            Err(why) => format!("could not ask about {address}: {why}"),
        });
    }
    let bar = sealable(store, draft, now)?;
    let mut sentence = said.join("; ");
    if let Some(first) = sentence.get(..1) {
        sentence = first.to_uppercase() + &sentence[1..];
    }
    Ok((bar, format!("{sentence}.")))
}

/// What the bar's buttons do. The page acts on them, because each ends in a send or a lookup
/// that must outlive the bar.
#[derive(Debug)]
pub(in crate::ui) enum BarAct {
    /// Take encryption off and send.
    WithoutEncryption,
    /// Take the protection off and send.
    Plain,
    LookUp(Vec<String>),
    /// Open Settings on Keys and certificates: to make a key, or import a certificate.
    OpenKeys,
    Unlock(Password),
}

/// A button in the bar.
fn act(words: &'static str, on_act: EventHandler<BarAct>, what: fn() -> BarAct) -> Element {
    rsx! {
        Button {
            size: ControlSize::Small,
            label: words.to_string(),
            onclick: on_primary(move || on_act.call(what())),
            common: Common { aria_label: Some(words.to_string()), ..Common::default() },
        }
    }
}

/// The signing and encrypting half of the warning bar: one banner, what stopped Send and what
/// could be done about it.
#[component]
pub(in crate::ui) fn SealWarn(bar: SealBar, on_act: EventHandler<BarAct>) -> Element {
    let without = "Send without encryption";
    let (severity, text, actions, detail) = match bar {
        SealBar::Clear => return rsx! {},
        SealBar::Sealing => (
            Severity::Info,
            "Signing and encrypting…".to_owned(),
            rsx! {},
            None,
        ),
        SealBar::Looking(addresses) => {
            let listed = addresses.join(", ");
            (
                Severity::Info,
                format!("Looking up keys for {listed}…"),
                rsx! {},
                None,
            )
        }
        SealBar::NoKeyFor(addresses) => {
            let listed = addresses.join(", ");
            let look = "Look up keys";
            (
                Severity::Warn,
                format!("No OpenPGP key for {listed}"),
                rsx! {
                    Button {
                        size: ControlSize::Small,
                        label: look.to_string(),
                        onclick: on_primary(move || on_act.call(BarAct::LookUp(addresses.clone()))),
                        common: Common { aria_label: Some(look.to_string()), ..Common::default() },
                    }
                    {act(without, on_act, || BarAct::WithoutEncryption)}
                },
                None,
            )
        }
        SealBar::NoOwnKey(address) => (
            Severity::Warn,
            format!("No OpenPGP key for {address}"),
            rsx! {
                {act("Create a key…", on_act, || BarAct::OpenKeys)}
                {act("Send without OpenPGP", on_act, || BarAct::Plain)}
            },
            None,
        ),
        SealBar::NoCertFor(addresses) => {
            let listed = addresses.join(", ");
            // S/MIME has no directory to ask: a certificate arrives with its owner's signed mail.
            (
                Severity::Warn,
                format!("No S/MIME certificate for {listed}"),
                rsx! { {act(without, on_act, || BarAct::WithoutEncryption)} },
                None,
            )
        }
        SealBar::NoOwnCert(address) => (
            Severity::Warn,
            format!("No S/MIME certificate for {address}"),
            rsx! {
                {act("Import your certificate…", on_act, || BarAct::OpenKeys)}
                {act("Send without S/MIME", on_act, || BarAct::Plain)}
            },
            None,
        ),
        SealBar::OwnCertCannotEncrypt(address) => (
            Severity::Warn,
            format!("Your S/MIME certificate for {address} can\u{2019}t decrypt your copy"),
            rsx! { {act(without, on_act, || BarAct::WithoutEncryption)} },
            None,
        ),
        SealBar::Blind(_, addresses) => {
            let listed = addresses.join(", ");
            (
                Severity::Warn,
                format!("Encrypting would reveal your Bcc recipients ({listed})"),
                rsx! { {act(without, on_act, || BarAct::WithoutEncryption)} },
                None,
            )
        }
        SealBar::Locked { key, tried } => {
            let prompt = format!("Passphrase for key {}", short(key));
            (
                if tried == Tried::Wrong {
                    Severity::Danger
                } else {
                    Severity::Warn
                },
                prompt.clone(),
                rsx! {
                    Passphrase {
                        prompt,
                        act: "Unlock and send".to_owned(),
                        working: Busy::Idle,
                        on_unlock: move |passphrase| on_act.call(BarAct::Unlock(passphrase)),
                    }
                },
                (tried == Tried::Wrong).then(|| WRONG.to_owned()),
            )
        }
    };
    rsx! {
        InlineBanner {
            severity,
            icon: Some(Icon::Key),
            text,
            detail: detail.map(TextLine::from),
            actions,
        }
    }
}

/// Seal and queue the saved draft off the thread that draws, with `passphrase` for its OpenPGP
/// key if one was typed. Answers what the send did: when it may leave, or the bar to show
/// instead. The passphrase is dropped when the send is done.
pub(in crate::ui) async fn seal_and_queue(
    store: Arc<SqliteStore>,
    draft: mail_domain::DraftId,
    leaves: mail_core::compose::Leaves,
    remind: Option<DateTime<Utc>>,
    passphrase: Option<Password>,
) -> Result<DateTime<Utc>, Sealed> {
    let secrets = seams().secrets;
    let done = tokio::task::spawn_blocking(move || {
        // Whether a passphrase was handed to the key: a key locked after one was is locked for
        // the wrong one, and a key locked with none given needs one.
        let given = std::cell::Cell::new(Tried::Nothing);
        let ask = |_: Fingerprint| {
            let typed = passphrase.as_ref().map(|typed| typed.expose().to_owned());
            if typed.is_some() {
                given.set(Tried::Wrong);
            }
            typed
        };
        let queued = super::life::queue_reminding(
            &store,
            secrets.as_ref(),
            &ask,
            draft,
            leaves,
            remind,
            Utc::now(),
        );
        drop(passphrase);
        match queued {
            Ok(due) => Ok(due),
            Err(refused) => match refused.locked() {
                Some(key) => Err(Sealed::Locked {
                    key,
                    tried: given.get(),
                }),
                None => Err(Sealed::Refused(refused.to_string())),
            },
        }
    })
    .await;
    done.unwrap_or_else(|error| Err(Sealed::Refused(format!("The send stopped: {error}"))))
}

/// Why sealing did not queue the send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum Sealed {
    Locked { key: Fingerprint, tried: Tried },
    Refused(String),
}

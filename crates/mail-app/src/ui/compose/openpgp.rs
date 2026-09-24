//! OpenPGP in the composer: a property row to sign or encrypt, and the warning bar that says, in
//! words, what stands between the choice and Send.
//!
//! What stands between them is [`crate::pgp::check`]'s answer, the same one `mailo compose`
//! gives, asked of the store as Send is pressed and never of the keyring. Signing and encrypting
//! themselves happen in [`super::life::queue`], off the thread that draws, with the key's
//! passphrase asked for in the bar when it has one.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use mail_domain::{Draft, Fingerprint, OpenPgp};
use mail_store::SqliteStore;

use super::super::icon::{Glyph, Icon};
use super::super::menu::{Menu, MenuItem, Right, Tile};
use super::super::pgp::{Busy, Tried, Unlock, seams, short};
use super::page::{Float, Page};
use crate::password::Password;
use crate::pgp::PgpError;

/// What the warning bar says about OpenPGP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum PgpBar {
    Clear,
    /// Encryption was asked for, and these recipients have no key to encrypt to.
    NoKeyFor(Vec<String>),
    /// The sending identity has no key of its own to sign or encrypt with.
    NoOwnKey(String),
    /// Encryption was asked for with these blind recipients.
    Blind(Vec<String>),
    /// Their domains are being asked for these recipients' keys.
    Looking(Vec<String>),
    /// The identity's key needs its passphrase.
    Locked {
        key: Fingerprint,
        tried: Tried,
    },
    /// Being signed and encrypted.
    Sealing,
}

/// The four choices, with the menu's key and words for each.
const CHOICES: [(OpenPgp, &str, &str, &str); 4] = [
    (OpenPgp::None, "pgp-none", "None", "sent as it is"),
    (
        OpenPgp::Sign,
        "pgp-sign",
        "Sign",
        "anyone can read it, and check it is from you",
    ),
    (
        OpenPgp::Encrypt,
        "pgp-encrypt",
        "Encrypt",
        "only the recipients can read it",
    ),
    (
        OpenPgp::SignAndEncrypt,
        "pgp-sign-encrypt",
        "Sign and encrypt",
        "only they can read it, and they can check it is from you",
    ),
];

/// What the row calls a choice.
pub(in crate::ui) fn label(openpgp: OpenPgp) -> &'static str {
    CHOICES
        .iter()
        .find(|(choice, ..)| *choice == openpgp)
        .map_or("None", |(_, _, name, _)| name)
}

/// The OpenPGP menu, the current choice checked.
pub(in crate::ui) fn items(openpgp: OpenPgp) -> Vec<MenuItem> {
    CHOICES
        .iter()
        .map(|(choice, key, name, help)| MenuItem {
            key: (*key).to_owned(),
            tile: Tile::Icon(Icon::Key),
            name: (*name).to_owned(),
            help: Some((*help).to_owned()),
            right: Right::Check(*choice == openpgp),
            group: None,
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
        })
        .collect()
}

/// A choice from the menu: the draft's OpenPGP, and a bar about the old one gone.
pub(in crate::ui) fn pick(page: &mut Page, key: &str) {
    if let Some((choice, ..)) = CHOICES.iter().find(|(_, had, ..)| *had == key) {
        page.openpgp = *choice;
        page.pgp_bar = PgpBar::Clear;
        page.touch();
    }
    page.float = Float::Closed;
}

/// `openpgp` with encryption taken off: a signature stays when one was asked for.
pub(in crate::ui) fn without_encryption(openpgp: OpenPgp) -> OpenPgp {
    match openpgp {
        OpenPgp::SignAndEncrypt | OpenPgp::Sign => OpenPgp::Sign,
        OpenPgp::Encrypt | OpenPgp::None => OpenPgp::None,
    }
}

/// What stands between `draft` and being sent as it asks, as the bar says it; `Clear` when
/// nothing does. Reads the store only.
pub(in crate::ui) fn sealable(
    store: &SqliteStore,
    draft: &Draft,
    now: DateTime<Utc>,
) -> Result<PgpBar, String> {
    let identity = crate::compose::identity_of(store, draft.account, Some(draft.identity))?;
    Ok(match crate::pgp::check(store, draft, &identity, now) {
        Ok(()) => PgpBar::Clear,
        Err(PgpError::NoKeyFor(addresses)) => PgpBar::NoKeyFor(addresses),
        Err(PgpError::NoOwnKey(address)) => PgpBar::NoOwnKey(address),
        Err(PgpError::BlindRecipients(addresses)) => PgpBar::Blind(addresses),
        Err(other) => return Err(other.to_string()),
    })
}

/// Ask each address's domain for its key, then ask again what stands in the way. Blocks on the
/// network: run on a blocking thread, from "Look up keys" only. Returns the bar, and what the
/// lookups found in words.
pub(in crate::ui) fn look_up(
    store: &SqliteStore,
    lookup: &super::super::pgp::Lookup,
    draft: &Draft,
    addresses: &[String],
    now: DateTime<Utc>,
) -> Result<(PgpBar, String), String> {
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

/// The OpenPGP row: what the draft asks, and the menu to change it.
#[component]
pub(in crate::ui) fn OpenPgpRow(page: Signal<Page>) -> Element {
    let openpgp = page.read().openpgp;
    let open = page.read().float == Float::OpenPgp;
    let shown = label(openpgp);
    let name = "OpenPGP";
    rsx! {
        div { class: "prop-row", "data-row": "openpgp",
            div { class: "k", Glyph { icon: Icon::Key, class: None }, "OpenPGP" }
            div { class: "v",
                button {
                    class: "pval",
                    r#type: "button",
                    aria_label: "{name}: {shown}",
                    onclick: move |_| {
                        let next = if open { Float::Closed } else { Float::OpenPgp };
                        page.write().float = next;
                    },
                    "{shown}"
                    span { class: "car", "▾" }
                }
                if open {
                    div { class: "p-menu",
                        Menu {
                            title: "OpenPGP".to_owned(),
                            items: items(openpgp),
                            filterable: false,
                            on_pick: move |key: String| pick(&mut page.write(), &key),
                            on_close: move |_| page.write().float = Float::Closed,
                            on_query: move |_| {},
                            slim: true,
                            active: None,
                        }
                    }
                }
            }
        }
    }
}

/// What the bar's buttons do. The page acts on them, because each ends in a send or a lookup
/// that must outlive the bar.
#[derive(Debug)]
pub(in crate::ui) enum BarAct {
    /// Take encryption off and send.
    WithoutEncryption,
    /// Take OpenPGP off and send.
    Plain,
    LookUp(Vec<String>),
    CreateKey,
    Unlock(Password),
}

/// The OpenPGP half of the warning bar.
#[component]
pub(in crate::ui) fn PgpWarn(bar: PgpBar, on_act: EventHandler<BarAct>) -> Element {
    let without = "Send without encryption";
    let plain = "Send without OpenPGP";
    let body = match bar {
        PgpBar::Clear => return rsx! {},
        PgpBar::Sealing => rsx! {
            span { "Signing and encrypting…" }
        },
        PgpBar::Looking(addresses) => {
            let listed = addresses.join(", ");
            rsx! {
                span { "Asking the domains of {listed} for their keys…" }
            }
        }
        PgpBar::NoKeyFor(addresses) => {
            let listed = addresses.join(", ");
            let look = "Look up keys";
            rsx! {
                span { class: "grow", "No OpenPGP key for {listed}, so this cannot be encrypted to them. Nothing was sent." }
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{look}",
                    onclick: move |_| on_act.call(BarAct::LookUp(addresses.clone())),
                    "{look}"
                }
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{without}",
                    onclick: move |_| on_act.call(BarAct::WithoutEncryption),
                    "{without}"
                }
            }
        }
        PgpBar::NoOwnKey(address) => {
            let create = "Create a key…";
            rsx! {
                span { class: "grow", "{address} has no OpenPGP key of its own to sign or encrypt with. Nothing was sent." }
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{create}",
                    onclick: move |_| on_act.call(BarAct::CreateKey),
                    "{create}"
                }
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{plain}",
                    onclick: move |_| on_act.call(BarAct::Plain),
                    "{plain}"
                }
            }
        }
        PgpBar::Blind(addresses) => {
            let listed = addresses.join(", ");
            rsx! {
                span { class: "grow",
                    "Encrypted mail names every key it is encrypted to, so everyone would learn it went to your Bcc recipients too ({listed}). Send them a separate message, or send this one without encryption."
                }
                button {
                    class: "mini",
                    r#type: "button",
                    aria_label: "{without}",
                    onclick: move |_| on_act.call(BarAct::WithoutEncryption),
                    "{without}"
                }
            }
        }
        PgpBar::Locked { key, tried } => rsx! {
            Unlock {
                prompt: format!("Your OpenPGP key {} needs its passphrase to sign or encrypt this message.", short(key)),
                tried,
                act: "Unlock and send".to_owned(),
                working: Busy::Idle,
                on_unlock: move |passphrase| on_act.call(BarAct::Unlock(passphrase)),
            }
        },
    };
    rsx! {
        div { class: "c-warn pgp-warn", role: "alert",
            Glyph { icon: Icon::Key, class: None }
            {body}
        }
    }
}

/// Seal and queue the saved draft off the thread that draws, with `passphrase` for its key if
/// one was typed. Answers what the send did: when it may leave, or the bar to show instead.
/// The passphrase is dropped when the send is done.
pub(in crate::ui) async fn seal_and_queue(
    store: Arc<SqliteStore>,
    draft: mail_domain::DraftId,
    leaves: crate::compose::Leaves,
    passphrase: Option<Password>,
) -> Result<DateTime<Utc>, Sealed> {
    let secrets = seams().secrets;
    let done = tokio::task::spawn_blocking(move || {
        let asked = std::cell::Cell::new(None::<Fingerprint>);
        let ask = |key: Fingerprint| {
            asked.set(Some(key));
            passphrase.as_ref().map(|typed| typed.expose().to_owned())
        };
        let queued = super::life::queue(&store, secrets.as_ref(), &ask, draft, leaves, Utc::now());
        let given = match passphrase {
            Some(_) => Tried::Wrong,
            None => Tried::Nothing,
        };
        drop(passphrase);
        match (queued, asked.get()) {
            (Ok(due), _) => Ok(due),
            // `ask` is asked only for a protected key: asked, and refused, is a key that is still
            // locked — for want of a passphrase, or for the wrong one.
            (Err(_), Some(key)) => Err(Sealed::Locked { key, tried: given }),
            (Err(why), None) => Err(Sealed::Refused(why)),
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

//! The composer's one protection choice: none, or OpenPGP, or S/MIME, each to sign, encrypt, or
//! both. Chosen from the foot's ⋯ › Security; a Security row is drawn only while it is not None,
//! and its menu sets it back.
//!
//! A message is protected one way or the other, never both — the two wrap the same content in
//! two envelopes, and one inside the other opens for nobody. So the page holds a single
//! [`Protection`], which cannot say "both", and one menu picks it; the draft's two fields are
//! written from it, one of them always `None` ([`Protection::fields`]).
//! There is no second row whose choice the first would have to undo.

use super::props::ChoiceRow;
use dioxus::prelude::*;
use ds::prelude::*;
use mail_domain::{Draft, Mode};

use super::super::menu::{MenuItem, Right};
use super::page::{Float, Page};
use super::seal::SealBar;

/// How the message is protected when it is sent: the scheme, and what it does to the message.
/// No protection is [`Protection::None`], not a scheme asked to do nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Protection {
    None,
    OpenPgp(Mode),
    Smime(Mode),
}

impl Protection {
    /// What `draft` asks. A draft that asks for both — only another client or a hand-edited
    /// file could make one, and sending refuses it — is read as its OpenPGP choice, so the page
    /// shows one and saving it leaves one.
    pub(in crate::ui) fn of(draft: &Draft) -> Protection {
        match (draft.openpgp, draft.smime) {
            (Some(mode), _) => Protection::OpenPgp(mode),
            (None, Some(mode)) => Protection::Smime(mode),
            (None, None) => Protection::None,
        }
    }

    /// The draft's `(openpgp, smime)` fields: one of them always `None`.
    pub(in crate::ui) fn fields(self) -> (Option<Mode>, Option<Mode>) {
        match self {
            Protection::None => (None, None),
            Protection::OpenPgp(mode) => (Some(mode), None),
            Protection::Smime(mode) => (None, Some(mode)),
        }
    }

    /// This protection with encryption taken off: a signature stays when one was asked for.
    pub(in crate::ui) fn without_encryption(self) -> Protection {
        match self {
            Protection::OpenPgp(mode) if mode.signs() => Protection::OpenPgp(Mode::Sign),
            Protection::Smime(mode) if mode.signs() => Protection::Smime(Mode::Sign),
            Protection::None | Protection::OpenPgp(_) | Protection::Smime(_) => Protection::None,
        }
    }
}

/// The seven choices, with the menu's key, group, words and help for each.
const CHOICES: [(Protection, &str, Option<&str>, &str, &str); 7] = [
    (Protection::None, "none", None, "None", "sent as it is"),
    (
        Protection::OpenPgp(Mode::Sign),
        "pgp-sign",
        Some("OpenPGP"),
        "Sign",
        "anyone can read it, and check it is from you",
    ),
    (
        Protection::OpenPgp(Mode::Encrypt),
        "pgp-encrypt",
        Some("OpenPGP"),
        "Encrypt",
        "only the recipients can read it",
    ),
    (
        Protection::OpenPgp(Mode::SignAndEncrypt),
        "pgp-sign-encrypt",
        Some("OpenPGP"),
        "Sign and Encrypt",
        "only they can read it, and they can check it is from you",
    ),
    (
        Protection::Smime(Mode::Sign),
        "smime-sign",
        Some("S/MIME"),
        "Sign",
        "anyone can read it, and check your certificate",
    ),
    (
        Protection::Smime(Mode::Encrypt),
        "smime-encrypt",
        Some("S/MIME"),
        "Encrypt",
        "only recipients whose certificates you hold can read it",
    ),
    (
        Protection::Smime(Mode::SignAndEncrypt),
        "smime-sign-encrypt",
        Some("S/MIME"),
        "Sign and Encrypt",
        "only they can read it, and they can check your certificate",
    ),
];

/// What the row calls a choice: its scheme and its words, or "None".
pub(in crate::ui) fn label(protection: Protection) -> String {
    CHOICES
        .iter()
        .find(|(choice, ..)| *choice == protection)
        .map_or_else(
            || "None".to_owned(),
            |(_, _, group, name, _)| match group {
                Some(group) => format!("{group} · {name}"),
                None => (*name).to_owned(),
            },
        )
}

/// The protection menu, the current choice checked, each scheme's choices under its name.
pub(in crate::ui) fn items(protection: Protection) -> Vec<MenuItem> {
    CHOICES
        .iter()
        .map(|(choice, key, group, name, help)| MenuItem {
            help: Some((*help).to_owned()),
            right: Right::Check(*choice == protection),
            group: group.map(str::to_owned),
            ..MenuItem::icon(*key, Icon::Key, *name)
        })
        .collect()
}

/// A choice from the menu: the page's protection, and a bar about the old one gone. A key the
/// menu does not have changes nothing.
pub(in crate::ui) fn pick(page: &mut Page, key: &str) {
    if let Some((choice, ..)) = CHOICES.iter().find(|(_, had, ..)| *had == key) {
        page.protection = *choice;
        page.seal_bar = SealBar::Clear;
        page.touch();
    }
    page.float = Float::Closed;
}

/// Every choice as `(key, scheme, name, checked)`: what ⋯ › Security lists, a submenu per scheme.
pub(in crate::ui) fn choices(
    protection: Protection,
) -> Vec<(&'static str, Option<&'static str>, &'static str, bool)> {
    CHOICES
        .iter()
        .map(|(choice, key, group, name, _)| (*key, *group, *name, *choice == protection))
        .collect()
}

/// The Security row, only while the message is signed or encrypted: how, and the menu that sets
/// it back to None.
#[component]
pub(in crate::ui) fn ProtectionRow(page: Signal<Page>) -> Element {
    let protection = page.read().protection;
    if protection == Protection::None {
        return rsx! {};
    }
    rsx! {
        ChoiceRow {
            page,
            label: "Security",
            row: "security",
            value: label(protection),
            float: Float::Protection,
            items: items(protection),
            on_pick: move |key: String| {
                let mut page = page;
                pick(&mut page.write(), &key);
            },
        }
    }
}

//! The composer's one protection control: none, or OpenPGP, or S/MIME, each to sign, encrypt, or
//! both.
//!
//! A message is protected one way or the other, never both — the two wrap the same content in
//! two envelopes, and one inside the other opens for nobody. So the page holds a single
//! [`Protection`], which cannot say "both", and one menu picks it; the draft's two fields are
//! written from it, one of them always `None` ([`Protection::openpgp`], [`Protection::smime`]).
//! There is no second row whose choice the first would have to undo.

use dioxus::prelude::*;
use mail_domain::{Draft, OpenPgp, Smime};

use super::super::menu::{Floating, MenuItem, Right, Tile};
use super::page::{Float, Page};
use super::seal::SealBar;
use ds::{Glyph, Icon, MenuKind, MountedRef};

/// What a protection does to the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Mode {
    Sign,
    Encrypt,
    SignAndEncrypt,
}

/// How the message is protected when it is sent.
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
        let mode = |sign, encrypt| match (sign, encrypt) {
            (true, true) => Some(Mode::SignAndEncrypt),
            (true, false) => Some(Mode::Sign),
            (false, true) => Some(Mode::Encrypt),
            (false, false) => None,
        };
        if let Some(mode) = mode(draft.openpgp.signs(), draft.openpgp.encrypts()) {
            Protection::OpenPgp(mode)
        } else if let Some(mode) = mode(draft.smime.signs(), draft.smime.encrypts()) {
            Protection::Smime(mode)
        } else {
            Protection::None
        }
    }

    /// The draft's OpenPGP field: `None` unless OpenPGP is the protection.
    pub(in crate::ui) fn openpgp(self) -> OpenPgp {
        match self {
            Protection::OpenPgp(Mode::Sign) => OpenPgp::Sign,
            Protection::OpenPgp(Mode::Encrypt) => OpenPgp::Encrypt,
            Protection::OpenPgp(Mode::SignAndEncrypt) => OpenPgp::SignAndEncrypt,
            Protection::None | Protection::Smime(_) => OpenPgp::None,
        }
    }

    /// The draft's S/MIME field: `None` unless S/MIME is the protection.
    pub(in crate::ui) fn smime(self) -> Smime {
        match self {
            Protection::Smime(Mode::Sign) => Smime::Sign,
            Protection::Smime(Mode::Encrypt) => Smime::Encrypt,
            Protection::Smime(Mode::SignAndEncrypt) => Smime::SignAndEncrypt,
            Protection::None | Protection::OpenPgp(_) => Smime::None,
        }
    }

    /// This protection with encryption taken off: a signature stays when one was asked for.
    pub(in crate::ui) fn without_encryption(self) -> Protection {
        match self {
            Protection::OpenPgp(Mode::Sign | Mode::SignAndEncrypt) => {
                Protection::OpenPgp(Mode::Sign)
            }
            Protection::Smime(Mode::Sign | Mode::SignAndEncrypt) => Protection::Smime(Mode::Sign),
            Protection::None | Protection::OpenPgp(Mode::Encrypt) => Protection::None,
            Protection::Smime(Mode::Encrypt) => Protection::None,
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
        "Sign and encrypt",
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
        "Sign and encrypt",
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
            key: (*key).to_owned(),
            tile: Tile::Icon(Icon::Key),
            name: (*name).to_owned(),
            help: Some((*help).to_owned()),
            right: Right::Check(*choice == protection),
            group: group.map(str::to_owned),
            marks: Vec::new(),
            title: Vec::new(),
            detail: Vec::new(),
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

/// The protection row: what the draft asks, and the menu to change it.
#[component]
pub(in crate::ui) fn ProtectionRow(page: Signal<Page>) -> Element {
    let protection = page.read().protection;
    let open = page.read().float == Float::Protection;
    let shown = label(protection);
    let name = "Protection";
    let mut value = use_signal(|| None::<MountedRef>);
    rsx! {
        div { class: "prop-row", "data-row": "protection",
            div { class: "k", Glyph { icon: Icon::Key, size: ds::IconSize::Compact }, "Protection" }
            div { class: "v",
                ds::Button {
                    variant: ds::ButtonVariant::Quiet,
                    label: shown.to_owned(),
                    aria_label: format!("{name}: {shown}"),
                    trailing: Some(ds::Trailing::Caret),
                    expanded: if open { ds::Expanded::Open } else { ds::Expanded::Closed },
                    mounted: move |event: MountedEvent| value.set(Some(MountedRef(event.data()))),
                    onclick: move |_: ds::Press| {
                        let next = if open { Float::Closed } else { Float::Protection };
                        page.write().float = next;
                    },
                }
                if open {
                    Floating {
                        kind: MenuKind::Dropdown,
                        anchor: value(),
                        title: "Protection".to_owned(),
                        items: items(protection),
                        on_pick: move |key: String| pick(&mut page.write(), &key),
                        on_close: move |_| {
                            if page.peek().float == Float::Protection {
                                page.write().float = Float::Closed;
                            }
                        },
                    }
                }
            }
        }
    }
}

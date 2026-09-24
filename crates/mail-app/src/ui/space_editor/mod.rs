//! The Space editor: a sheet over the frame, opened from the Space's name, "+" or the gear.
//!
//! Every change goes into the window's Spaces at once, so the frame's `Ds` root repaints with
//! it (cross-fading, as a switch does), Save writes `spaces.json`, and Esc puts the Space back
//! exactly as the sheet found it. The rules live on [`Draft`]; the colours all come from
//! quire's `ds::space` palette.

mod hue;
mod notify;
mod parts;

pub(in crate::ui) use parts::Seg;

use self::hue::HueField;
use self::notify::Notifications;
use self::parts::{Marks as MarksChoice, Presets, Readout, Stops};
use super::field::{Field, FieldKind};
use super::frame::keep;
use crate::space::Spaces;
use crate::space::edit::Draft;
use crate::view::{Motion, Shell, Theme};
use dioxus::prelude::*;
use ds::{CardAccent, Grain, Scheme};

/// Apply `edit` to the draft and put the result in the window's Spaces, which the frame's
/// root reads.
///
/// No write to disk: that is Save's. A change to a sheet that has closed does nothing. Every
/// change cross-fades the frame, a drag's steps included: quire's root has one fade, and
/// design/21-SPACES.md section 6 has the tint update live with it.
pub(super) fn change(
    mut editing: Signal<Option<Draft>>,
    mut spaces: Signal<Spaces>,
    edit: impl FnOnce(&mut Draft),
) {
    let (index, space) = {
        let mut guard = editing.write();
        let Some(draft) = guard.as_mut() else {
            return;
        };
        edit(draft);
        (draft.index, draft.space.clone())
    };
    if let Some(slot) = spaces.write().spaces.get_mut(index) {
        *slot = space;
    }
}

/// Esc: put the Space back as the sheet found it, and close.
pub(super) fn cancel(mut editing: Signal<Option<Draft>>, mut spaces: Signal<Spaces>) {
    let Some(draft) = editing.write().take() else {
        return;
    };
    let saved = draft.reverted();
    if let Some(slot) = spaces.write().spaces.get_mut(draft.index) {
        *slot = saved;
    }
    dioxus::document::eval("document.querySelector('.app')?.focus()");
}

/// Save: the draft is already on screen and in the Spaces; write them and close.
fn save(mut editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) {
    editing.set(None);
    keep(&spaces.read());
    dioxus::document::eval("document.querySelector('.app')?.focus()");
}

/// `--g`: `dots`' frame gradient in `scheme`, the one the window's root resolved to.
///
/// quire resolves System in Rust and says which scheme it is (`ds::use_env`), so a swatch
/// carries the one gradient it shows rather than both for the stylesheet to choose between.
pub(super) fn gradient_in(dots: &[ds::Dot], scheme: Scheme) -> String {
    format!("--g:{}", ds::gradient(&ds::derive(dots, scheme)))
}

/// What the Save button says it does.
const SAVE_TITLE: &str = "Save this Space and close";

/// The schemes the readout measures a Space in: its own, or both when the desktop decides.
fn measured_in(theme: Theme) -> &'static [(Scheme, &'static str)] {
    match theme {
        Theme::Light => &[(Scheme::Light, "")],
        Theme::Dark => &[(Scheme::Dark, "")],
        Theme::System => &[(Scheme::Light, "Light"), (Scheme::Dark, "Dark")],
    }
}

/// The sheet. Renders nothing while `editing` is `None`.
#[component]
pub(super) fn SpaceEditor(
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
) -> Element {
    let scheme = ds::use_env().scheme;
    let Some(draft) = editing.read().clone() else {
        return rsx! {};
    };
    let space = draft.space.clone();
    let sw = gradient_in(&space.look.dots, scheme);
    let grain = space.look.grain.0;
    let theme_now = space.look.theme;
    let motion_now = space.motion;
    let accent_now = space.look.card_accent;
    rsx! {
        div {
            class: "editor",
            role: "dialog",
            aria_label: "Space editor",
            h3 {
                span { class: "sw", style: "{sw}" }
                Field {
                    kind: FieldKind::Boxed,
                    value: space.name.clone(),
                    placeholder: "Name this Space".to_owned(),
                    extra: Some("ed-name".to_owned()),
                    on_input: move |value: String| change(editing, spaces, |draft| draft.space.name = value),
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
            }
            div {
                div { class: "ed-label", "Colour", span { class: "r", "drag a dot · arrows, Shift ×10" } }
                HueField { editing, spaces }
                Stops { editing, spaces }
            }
            div {
                div { class: "ed-label", "Grain", span { class: "r mono", "{grain}" } }
                Field {
                    kind: FieldKind::Range { min: 0, max: 100 },
                    value: grain.to_string(),
                    placeholder: "Grain".to_owned(),
                    extra: None,
                    on_input: move |value: String| {
                        if let Ok(grain) = value.parse::<u8>() {
                            change(editing, spaces, |draft| draft.space.look.grain = Grain(grain.min(100)));
                        }
                    },
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
            }
            div {
                div { class: "ed-label", "Appearance" }
                Seg {
                    label: "Theme".to_owned(),
                    options: [Theme::System, Theme::Light, Theme::Dark]
                        .iter()
                        .map(|theme| (theme.label().to_owned(), *theme == theme_now))
                        .collect::<Vec<_>>(),
                    on_pick: move |index: usize| {
                        let theme = [Theme::System, Theme::Light, Theme::Dark][index];
                        change(editing, spaces, |draft| draft.space.look.theme = theme);
                    },
                }
            }
            div {
                div { class: "ed-label", "Motion" }
                Seg {
                    label: "Motion".to_owned(),
                    options: Motion::ALL
                        .iter()
                        .map(|motion| (motion.label().to_owned(), *motion == motion_now))
                        .collect::<Vec<_>>(),
                    on_pick: move |index: usize| {
                        let motion = Motion::ALL[index];
                        change(editing, spaces, |draft| draft.space.motion = motion);
                    },
                }
            }
            div {
                div { class: "ed-label", "Accent inside the card" }
                Seg {
                    label: "Card accent".to_owned(),
                    options: vec![
                        ("A hint of the Space".to_owned(), accent_now == CardAccent::SpaceHue),
                        ("Postmark".to_owned(), accent_now == CardAccent::Postmark),
                    ],
                    on_pick: move |index: usize| {
                        let accent = if index == 0 { CardAccent::SpaceHue } else { CardAccent::Postmark };
                        change(editing, spaces, |draft| draft.space.look.card_accent = accent);
                    },
                }
            }
            MarksChoice { shell }
            Notifications {}
            div {
                div { class: "ed-label", "Accounts" }
                button {
                    class: "mini",
                    r#type: "button",
                    onclick: move |_| super::add_account::open(shell),
                    "Add account…"
                }
                p { class: "capnote", "A new account joins this Space when the Space shows only some accounts." }
            }
            div {
                div { class: "ed-label", "Contacts" }
                button {
                    class: "mini",
                    r#type: "button",
                    onclick: move |_| super::contacts::open(shell),
                    "Contacts…"
                }
                p { class: "capnote", "Who the composer suggests: import, export, rename, forget." }
            }
            div {
                div { class: "ed-label", "Rules" }
                button {
                    class: "mini",
                    r#type: "button",
                    onclick: move |_| super::rules::open(shell),
                    "Rules…"
                }
                p { class: "capnote", "What new mail sorts into, the vacation reply, and the server's copy." }
            }
            div {
                div { class: "ed-label", "Keys and certificates" }
                button {
                    class: "mini",
                    r#type: "button",
                    onclick: move |_| super::pgp::keys::open(shell),
                    "Keys and certificates…"
                }
                p { class: "capnote", "OpenPGP keys and S/MIME certificates, yours and your correspondents': make, import, export, trust, delete." }
            }
            div {
                div { class: "ed-label", "Presets" }
                Presets { editing, spaces }
            }
            div {
                div { class: "ed-label", "Measured, this Space" }
                for &(scheme, heading) in measured_in(theme_now) {
                    Readout { key: "{heading}", space: space.clone(), scheme, heading: heading.to_owned() }
                }
            }
            div { class: "ed-foot",
                button {
                    class: "mini",
                    r#type: "button",
                    onclick: move |_| cancel(editing, spaces),
                    "Cancel"
                    span { class: "k", "Esc" }
                }
                button {
                    class: "mini primary",
                    r#type: "button",
                    title: "{SAVE_TITLE}",
                    onclick: move |_| save(editing, spaces),
                    "Save"
                }
            }
        }
    }
}

#[cfg(test)]
mod notify_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;

//! The passphrase field: a key's passphrase, or an identity file's password, typed and handed on
//! once, to the call that needs it.
//!
//! What is typed is held in a [`Password`] kept by this component's hook — not a signal, so
//! nothing that subscribes to state can read it, and nothing that snapshots state can copy it.
//! Unlock moves it out, leaving an empty one behind, so it exists in one place at a time and is
//! zeroed wherever it is dropped. The field is [`FieldKind::Secret`], which never draws a value:
//! the markup never holds it.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

use super::super::field::{Field, FieldKind};
use super::{Busy, Tried};
use crate::password::Password;

/// The field and its button: `prompt` says what it is for, `act` is the button's words, `noun`
/// what the secret is called ("Passphrase" when not given). `on_unlock` is handed what was typed
/// and owns it from then on.
#[component]
pub(in crate::ui) fn Unlock(
    prompt: String,
    tried: Tried,
    act: String,
    noun: Option<String>,
    working: Busy,
    on_unlock: EventHandler<Password>,
) -> Element {
    let typed = use_hook(|| Rc::new(RefCell::new(Password::default())));
    let held = typed.clone();
    let give = move || {
        let passphrase = std::mem::take(&mut *typed.borrow_mut());
        if !passphrase.is_empty() {
            on_unlock.call(passphrase);
        }
    };
    let on_enter = give.clone();
    let noun = noun.unwrap_or_else(|| "Passphrase".to_owned());
    let label = format!("{noun}: {prompt}");
    rsx! {
        div { class: "unlock", role: "group", aria_label: "{prompt}",
            p { class: "say", "{prompt}" }
            if tried == Tried::Wrong {
                p { class: "why", role: "alert", "That passphrase did not unlock the key. Try again." }
            }
            div {
                class: "unlock-row",
                onkeydown: move |event: KeyboardEvent| {
                    if event.key().to_string() == "Enter" {
                        event.prevent_default();
                        on_enter();
                    }
                },
                Field {
                    kind: FieldKind::Secret,
                    value: String::new(),
                    placeholder: label,
                    extra: Some("unlock-field".to_owned()),
                    on_input: move |value: String| *held.borrow_mut() = Password::new(value),
                    on_focus: |_| {},
                    on_blur: |_| {},
                }
                button {
                    class: "mini primary",
                    r#type: "button",
                    aria_label: "{act}",
                    disabled: working == Busy::Working,
                    onclick: move |_| give(),
                    if working == Busy::Working { "Working…" } else { "{act}" }
                }
            }
        }
    }
}

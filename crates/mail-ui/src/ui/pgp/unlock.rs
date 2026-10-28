//! The passphrase field: a key's passphrase, or an identity file's password, typed and handed on
//! once, to the call that needs it.
//!
//! What is typed is held in a [`Password`] kept by this component's hook — not a signal, so
//! nothing that subscribes to state can read it, and nothing that snapshots state can copy it.
//! Unlock moves it out, leaving an empty one behind, so it exists in one place at a time and is
//! zeroed wherever it is dropped. The field is [`FieldKind::Secret`], which never draws a value:
//! the markup never holds it.

use ds::components::content::label::LabelRole;
use ds::components::controls::button_model::Answers;
use ds::components::fields::text_field_model::Invalid;
use ds::motion::detail::stamp::EventStamp;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

use super::super::press::{available, on_primary};
use super::{Busy, Tried};
use mail_client::password::Password;

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
    // Each passphrase handed on is an attempt, and the wrong one's refusal is stamped with the
    // count, so a second wrong one is announced again.
    let mut attempts = use_signal(|| 0u32);
    let validity = if tried == Tried::Wrong {
        Validity::Invalid(Invalid {
            message: WRONG.into(),
            stamp: EventStamp(attempts()),
        })
    } else {
        Validity::Valid
    };
    rsx! {
        div { class: "unlock", role: "group", aria_label: "{prompt}",
            p { class: "say", Label { text: prompt.clone(), role: LabelRole::Secondary } }
            Passphrase {
                prompt: prompt.clone(),
                act,
                noun,
                working,
                validity,
                on_unlock: move |passphrase| {
                    attempts += 1;
                    on_unlock.call(passphrase);
                },
            }
        }
    }
}

/// What is said when the passphrase typed did not open the key.
pub(in crate::ui) const WRONG: &str = "Wrong passphrase.";

/// The field and its button, nothing else: for a place that says the prompt itself (the
/// composer's banner, which says it as the banner's text). `prompt` names the field for
/// assistive technology; `validity` is the field's own refusal of what was last typed.
#[component]
pub(in crate::ui) fn Passphrase(
    prompt: String,
    act: String,
    noun: Option<String>,
    working: Busy,
    #[props(default)] validity: Validity,
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
        div {
            class: "unlock-row",
            onkeydown: move |event: KeyboardEvent| {
                if event.key().to_string() == "Enter" {
                    event.prevent_default();
                    on_enter();
                }
            },
            TextField {
                kind: FieldKind::Secure,
                label: label.clone(),
                value: String::new(),
                validity,
                placeholder: label,
                oninput: move |value: String| *held.borrow_mut() = Password::new(value),
                common: Common { extra_class: ExtraClass::parse("unlock-field").ok(), ..Common::default() },
            }
            Button {
                answers: Answers::Return,
                label: if working == Busy::Working { "Working…".to_owned() } else { act.to_string() },
                availability: available(working != Busy::Working),
                onclick: on_primary(give),
                common: Common { aria_label: Some(act.to_string()), ..Common::default() },
            }
        }
    }
}

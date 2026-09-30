//! The sender card's part in the book: whether this address is a contact, a name to give it,
//! and a way to forget it.
//!
//! The name is typed into the card itself. Every key typed there stops at the field, so a
//! letter is a letter and not the shortcut it would be over the list.

use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::controls::button_model::ButtonRole;
use ds::prelude::*;
use ds::root::common::Common;
use ds::root::pass_through::ExtraClass;
use ds::style::tokens::control_size::ControlSize;
use std::sync::Arc;

use dioxus::prelude::*;
use mail_store::{SqliteStore, Store};

use super::super::press::on_primary;
use super::book::{self, Standing};

/// What the part is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Doing {
    Showing,
    /// A name is being typed.
    Naming(String),
    /// A write failed, and this is why.
    Failed(String),
}

#[component]
pub(in crate::ui) fn ContactPart(email: String, name: String) -> Element {
    let mut changed = use_signal(|| 0u64);
    let mut doing = use_signal(|| Doing::Showing);
    let known = use_memo({
        let email = email.clone();
        move || {
            let _ = changed();
            let store = consume_context::<Arc<SqliteStore>>();
            store.contact(&email).ok().flatten()
        }
    });
    let standing = Standing::of(known().as_ref());
    let current = known()
        .and_then(|contact| contact.name)
        .unwrap_or_else(|| name.clone());
    let keep = {
        let email = email.clone();
        move || {
            let typed = match &*doing.peek() {
                Doing::Naming(typed) => typed.clone(),
                _ => return,
            };
            let store = consume_context::<Arc<SqliteStore>>();
            doing.set(match book::name(store.as_ref(), &email, &typed) {
                Ok(_) => Doing::Showing,
                Err(why) => Doing::Failed(why),
            });
            changed += 1;
        }
    };
    let mut keep_on_enter = keep.clone();
    let keep_on_click = keep;
    let mut forget = {
        let email = email.clone();
        move |_| {
            let store = consume_context::<Arc<SqliteStore>>();
            if let Err(why) = book::forget(store.as_ref(), &email) {
                doing.set(Doing::Failed(why));
            }
            changed += 1;
        }
    };
    rsx! {
        div { class: "contact",
            match doing() {
                Doing::Naming(typed) => rsx! {
                    div {
                        class: "naming",
                        onkeydown: move |event: KeyboardEvent| {
                            event.stop_propagation();
                            match event.key().to_string().as_str() {
                                "Enter" => {
                                    event.prevent_default();
                                    keep_on_enter();
                                }
                                "Escape" => doing.set(Doing::Showing),
                                _ => {}
                            }
                        },
                        TextField {
                            label: "Their name".to_owned(),
                            bezel: FieldBezel::Plain,
                            placeholder: "Their name".to_owned(),
                            value: typed,
                            oninput: move |value: String| doing.set(Doing::Naming(value)),
                            common: Common { extra_class: ExtraClass::parse("book-name").ok(), ..Common::default() },
                        }
                        Button {
                            label: "Save".to_owned(),
                            size: ControlSize::Small,
                            onclick: on_primary(keep_on_click),
                            common: Common { aria_label: Some(format!("Save the name for {email}")), ..Common::default() },
                        }
                    }
                },
                shown => rsx! {
                    div { class: "standing",
                        Label { text: standing.label().to_owned(), role: LabelRole::Secondary, style: LabelStyle::Footnote }
                        Button {
                            label: standing.name_action().to_owned(),
                            size: ControlSize::Small,
                            onclick: {
                                let current = current.clone();
                                on_primary(move || doing.set(Doing::Naming(current.clone())))
                            },
                            common: Common { aria_label: Some(format!("{}: {email}", standing.name_action())), ..Common::default() },
                        }
                        if standing != Standing::Unknown {
                            Button {
                                role: ButtonRole::Destructive,
                                label: "Forget",
                                size: ControlSize::Small,
                                onclick: on_primary(move || forget(())),
                                common: Common { aria_label: Some(format!("Forget {email}")), ..Common::default() },
                            }
                            Label { text: "Mail may teach it again".to_owned(), role: LabelRole::Tertiary, style: LabelStyle::Footnote }
                        }
                    }
                    if let Doing::Failed(why) = shown {
                        Label { text: why, role: LabelRole::Tertiary, style: LabelStyle::Footnote }
                    }
                },
            }
        }
    }
}

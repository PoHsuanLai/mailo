//! General › VIPs: the people whose mail matters most, a row each with its Remove, a field that
//! adds one by address, and the people pinned in a Space who are not VIPs yet, each with its Add.
//!
//! The list is the store's (`mail_core::vip`), not a key of `settings.toml`: `is:vip`, the VIPs
//! place and "Notify for: VIPs only" all read it there. Pinned people are a Space's shortcuts and
//! stay so; they are offered here, never made VIPs by being pinned.

use crate::ui::copy;
use crate::ui::press::on_primary;
use crate::ui::space::Pinned;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::vip::{Mark, Vip, Vips};
use mail_core::{SqliteStore, Store};
use std::sync::Arc;

/// A person a row names: their name with their address, or the address alone.
fn shown(name: Option<&str>, address: &str) -> String {
    match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!("{name} · {address}"),
        None => address.to_owned(),
    }
}

/// The people pinned in `spaces` who are not in `vips`, by address, once each.
pub(super) fn suggested<'a>(
    spaces: impl IntoIterator<Item = &'a Pinned>,
    vips: &Vips,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for pin in spaces {
        let Pinned::Person { name, email } = pin else {
            continue;
        };
        let email = email.trim().to_lowercase();
        if email.is_empty() || vips.includes(&email) || out.iter().any(|(_, seen)| *seen == email) {
            continue;
        }
        out.push((name.clone(), email));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Mark or unmark `address`, saying a failure in `failed`, and move `revision` so every window
/// reads the VIPs again.
fn set(
    address: &str,
    name: Option<&str>,
    mark: Mark,
    mut failed: Signal<Option<String>>,
    mut revision: Signal<u64>,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    match mail_core::vip::set(store.as_ref() as &dyn Store, address, name, mark) {
        Ok(()) => {
            failed.set(None);
            revision += 1;
        }
        Err(why) => failed.set(Some(crate::ui::remedy::told(why))),
    }
}

#[component]
pub(super) fn VipsPage(revision: Signal<u64>) -> Element {
    // Read again whenever a VIP is marked or unmarked, here or in another window.
    let _ = revision();
    let store = use_context::<Arc<SqliteStore>>();
    let listed: Vec<Vip> = match mail_core::vip::listed(store.as_ref()) {
        Ok(listed) => listed,
        Err(why) => {
            log::warn!("the VIPs could not be listed: {why}");
            Vec::new()
        }
    };
    let vips = Vips::new(listed.iter().map(|vip| vip.address.clone()));
    let pins: Vec<Pinned> = try_consume_context::<crate::ui::space::Handle>()
        .map(|handle| {
            handle
                .spaces()
                .read()
                .list()
                .iter()
                .flat_map(|space| space.payload.pins.clone())
                .collect()
        })
        // A window with no Spaces has nobody pinned to offer.
        .unwrap_or_default();
    let offered = suggested(&pins, &vips);
    let failed = use_signal(|| None::<String>);
    let mut typed = use_signal(String::new);
    let add_typed = move || {
        let address = typed.peek().trim().to_owned();
        if address.is_empty() {
            return;
        }
        set(&address, None, Mark::Vip, failed, revision);
        if failed.peek().is_none() {
            typed.set(String::new());
        }
    };
    let mut on_enter = add_typed;
    let none = listed.is_empty();
    rsx! {
        Form {
            FormSection {
                if none {
                    FieldRow { label: copy::empty::NO_VIPS }
                }
                for vip in listed {
                    FieldRow {
                        key: "{vip.address}",
                        label: shown(vip.name.as_deref(), &vip.address),
                        Button {
                            label: copy::controls::REMOVE,
                            common: Common {
                                aria_label: Some(copy::aria::remove_vip(&vip.address)),
                                ..Common::default()
                            },
                            onclick: {
                                let address = vip.address.clone();
                                on_primary(move || set(&address, None, Mark::NotVip, failed, revision))
                            },
                        }
                    }
                }
                FieldRow { label: copy::fields::VIP_ADDRESS,
                    div {
                        class: "vip-add",
                        onkeydown: move |event: KeyboardEvent| {
                            if event.key() == Key::Enter {
                                event.prevent_default();
                                on_enter();
                            }
                        },
                        TextField {
                            label: copy::fields::VIP_ADDRESS.to_owned(),
                            value: typed(),
                            placeholder: copy::fields::VIP_ADDRESS_EXAMPLE.to_owned(),
                            oninput: move |value: String| typed.set(value),
                        }
                        Button {
                            label: copy::controls::ADD,
                            onclick: on_primary(add_typed),
                        }
                    }
                }
            }
            if !offered.is_empty() {
                FormSection { title: Some(copy::headers::PINNED_IN_SPACES.to_owned()),
                    for (name, email) in offered {
                        FieldRow {
                            key: "{email}",
                            label: shown(Some(&name), &email),
                            Button {
                                label: copy::controls::ADD,
                                common: Common {
                                    aria_label: Some(copy::aria::add_vip(&email)),
                                    ..Common::default()
                                },
                                onclick: {
                                    let (name, email) = (name.clone(), email.clone());
                                    on_primary(move || set(&email, Some(&name), Mark::Vip, failed, revision))
                                },
                            }
                        }
                    }
                }
            }
        }
        if let Some(why) = failed() {
            crate::ui::common::Problem {
                title: copy::titles::NOT_SAVED,
                why,
                onclose: move |()| {
                    let mut failed = failed;
                    failed.set(None);
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_people_who_are_not_vips_are_offered_once_each() {
        let person = |name: &str, email: &str| Pinned::Person {
            name: name.to_owned(),
            email: email.to_owned(),
        };
        let pins = [
            person("Dana", "Dana@Example.test"),
            person("Bob", "bob@example.test"),
            Pinned::Search {
                name: "Bills".to_owned(),
                query: "from:bank".to_owned(),
            },
            person("Dana W", "dana@example.test"),
            person("Ada", "ada@example.test"),
        ];
        assert_eq!(
            suggested(&pins, &Vips::new(["bob@example.test"])),
            [
                ("Ada".to_owned(), "ada@example.test".to_owned()),
                ("Dana".to_owned(), "dana@example.test".to_owned()),
            ]
        );
        assert_eq!(shown(Some(" "), "a@b.test"), "a@b.test");
        assert_eq!(shown(Some("Ada"), "a@b.test"), "Ada · a@b.test");
    }
}

//! General › VIPs: the people whose mail matters most, a row each with its Remove, a field that
//! adds one by address, and the people pinned in a Space who are not VIPs yet, each with its Add.
//!
//! The list is the client's (`mail_core::vip`), not a key of `settings.toml`: `is:vip`, the VIPs
//! place and "Notify for: VIPs only" all read it there. Pinned people are a Space's shortcuts and
//! stay so; they are offered here, never made VIPs by being pinned.

use crate::ui::press::on_primary;
use crate::ui::space::Pinned;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::vip::{Mark, Vip, Vips};
use mail_ui_model::copy;

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

/// Mark or unmark `address` on a blocking thread, and hand `then` how it went: the failure in
/// the window's words. The VIPs are read again by every window when the store moves.
fn set(
    address: &str,
    name: Option<&str>,
    mark: Mark,
    then: impl FnOnce(Result<(), String>) + 'static,
) {
    let client = crate::ui::live::client();
    let (address, name) = (address.to_owned(), name.map(str::to_owned));
    spawn(async move {
        let done = tokio::task::spawn_blocking(move || {
            client
                .query()
                .set_vip(&address, name.as_deref(), mark)
                .map_err(crate::ui::remedy::told)
        })
        .await
        .unwrap_or_else(|why| Err(why.to_string()));
        then(done);
    });
}

/// A button's mark or unmark: a failure is said in the dialog.
fn set_with_dialog(
    address: &str,
    name: Option<&str>,
    mark: Mark,
    mut failed: Signal<Option<String>>,
) {
    set(address, name, mark, move |done| match done {
        Ok(()) => failed.set(None),
        Err(why) => failed.set(Some(why)),
    });
}

#[component]
pub(super) fn VipsPage() -> Element {
    // Read again whenever a VIP is marked or unmarked, here or in another window.
    let _ =
        crate::ui::live::moved_for(mail_client::Topics::CONTACTS | mail_client::Topics::SUMMARIES);
    let client = crate::ui::live::client();
    let listed: Vec<Vip> = match client.query().listed_vips() {
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
    // Why the address typed was not taken, under the field it was typed in.
    let mut refused = use_signal(|| None::<String>);
    let add_typed = move || {
        let address = typed.peek().trim().to_owned();
        if address.is_empty() {
            return;
        }
        set(&address, None, Mark::Vip, move |done| match done {
            Ok(()) => {
                refused.set(None);
                typed.set(String::new());
            }
            Err(why) => refused.set(Some(why)),
        });
    };
    let on_enter = add_typed;
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
                                on_primary(move || set_with_dialog(&address, None, Mark::NotVip, failed))
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
                            validity: crate::ui::views::refusal(refused.read().as_deref()),
                            oninput: move |value: String| {
                                refused.set(None);
                                typed.set(value);
                            },
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
                                    on_primary(move || set_with_dialog(&email, Some(&name), Mark::Vip, failed))
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

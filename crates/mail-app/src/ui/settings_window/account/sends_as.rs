//! Sending as, on an account's page: the name the account's own address goes out under, and the
//! other addresses it sends as, its aliases, each with its own name and Reply-To.
//!
//! The Full name field is kept when it is left or Return is pressed, as a Space's name is. An
//! alias is edited in fields of its own under the list, opened by its row or by Add Alias…, and
//! kept by Save, which is where a typed address is read: one that is not an address, or that the
//! account already sends as, is said under the fields and nothing is kept. Remove Alias takes it
//! off the account at once; a draft written as it goes back to the account's own address
//! (`mail_core::identity::remove_alias`), so nothing is lost and nothing asks first.
//!
//! The add sheet has no name field: its fields are porter's sign-in form, which names none, and a
//! browser sign-in has no form at all. So the name is given here, for a new account and an old
//! one alike.

use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::press::on_primary;
use dioxus::prelude::*;
use ds::components::content::label::Label;
use ds::components::controls::button_model::ButtonRole;
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::identity::Typed;
use mail_domain::{Identity, IdentityId, IsDefault};
use porter_core::AccountId;

/// The key of the Add Alias… row, which no identity's id can be.
const ADD: &str = " add-alias";

/// The element id of the row that opens `alias`'s fields.
pub(in crate::ui) fn alias_row_id(alias: IdentityId) -> String {
    format!("account-alias-{alias}")
}

/// The element id of the Add Alias… row.
pub(in crate::ui) const ADD_ALIAS_ROW: &str = "account-add-alias";

/// The alias whose fields are open: one being added (`id` none) or one being changed, as typed
/// so far, and what refused the last Save.
#[derive(Debug, Clone, PartialEq)]
struct Editing {
    id: Option<IdentityId>,
    typed: Typed,
    refused: Option<String>,
}

/// The account's identities, its own first; why not, when the client cannot say. Without a client
/// to ask there are none.
fn identities(account: &AccountId) -> Result<Vec<Identity>, String> {
    match crate::ui::live::try_client() {
        Some(client) => client
            .identities()
            .of_account(account.clone())
            .map_err(crate::ui::remedy::told),
        None => Ok(Vec::new()),
    }
}

/// Keep `typed` as the name of `account`'s own address, when it differs from what is kept. The
/// revision moves only once it is kept; a client that refuses is said in `failed`.
fn keep_name(account: &AccountId, typed: &str, mut failed: Signal<Option<(&'static str, String)>>) {
    let Some(client) = crate::ui::live::try_client() else {
        return;
    };
    let kept = match identities(account) {
        Ok(all) => all
            .into_iter()
            .find(|one| one.default == IsDefault::Default)
            .and_then(|own| own.from.name),
        Err(why) => {
            failed.set(Some((copy::titles::NOT_READ, why)));
            return;
        }
    };
    if kept.as_deref().unwrap_or("") == typed.trim() {
        return;
    }
    match client.identities().set_name(account.clone(), typed) {
        Ok(_) => {
            failed.set(None);
        }
        Err(why) => failed.set(Some((
            copy::titles::NOT_SAVED,
            crate::ui::remedy::told(why),
        ))),
    }
}

/// Save the alias being edited: added or changed, then the fields close; refused, they stay
/// with the reason under them.
fn save(account: &AccountId, mut editing: Signal<Option<Editing>>) {
    let Some(client) = crate::ui::live::try_client() else {
        return;
    };
    let Some(open) = editing.peek().clone() else {
        return;
    };
    let kept = match open.id {
        Some(id) => client.identities().edit(id, &open.typed).map(|_| ()),
        None => client
            .identities()
            .add_alias(account.clone(), &open.typed)
            .map(|_| ()),
    };
    match kept {
        Ok(()) => {
            editing.set(None);
        }
        Err(why) => {
            if let Some(open) = editing.write().as_mut() {
                open.refused = Some(crate::ui::remedy::told(why));
            }
        }
    }
}

/// Take the alias off its account, and close its fields.
fn remove(id: IdentityId, mut editing: Signal<Option<Editing>>) {
    let Some(client) = crate::ui::live::try_client() else {
        return;
    };
    match client.identities().remove_alias(id) {
        Ok(()) => {
            editing.set(None);
        }
        Err(why) => {
            if let Some(open) = editing.write().as_mut() {
                open.refused = Some(crate::ui::remedy::told(why));
            }
        }
    }
}

/// One of an alias's fields: `label`, holding what `field` reads of what is typed, writing what
/// is typed through `write`, and Return saving.
fn field(
    account: AccountId,
    label: &'static str,
    placeholder: &'static str,
    value: String,
    editing: Signal<Option<Editing>>,
    write: fn(&mut Typed, String),
) -> Element {
    let mut editing = editing;
    rsx! {
        FieldRow { label,
            TextField {
                label,
                value,
                placeholder,
                oninput: move |typed: String| {
                    if let Some(open) = editing.write().as_mut() {
                        write(&mut open.typed, typed);
                        open.refused = None;
                    }
                },
                onsubmit: move |_| save(&account, editing),
            }
        }
    }
}

/// The group: Full name, then the aliases and Add Alias…, then the open alias's fields.
#[component]
pub(super) fn SendsAs(account: AccountId) -> Element {
    // Read again when an identity is added, changed or removed.
    let _ = crate::ui::live::moved();
    let mut editing = use_signal(|| None::<Editing>);
    let mut failed = use_signal(|| None::<(&'static str, String)>);
    // An unreadable list is said, once, and drawn as an account with no name and no aliases; the
    // page's other groups are still worth showing.
    let mut unread_seen = use_signal(|| false);
    let (all, unread) = match identities(&account) {
        Ok(all) => (all, None),
        Err(why) => (Vec::new(), Some(why)),
    };
    let own = all.iter().find(|one| one.default == IsDefault::Default);
    let kept_name = own
        .and_then(|own| own.from.name.clone())
        // No name is an empty field.
        .unwrap_or_default();
    let mut name = use_signal(|| kept_name.clone());
    // Another window, or Save, moved the kept name on: the field follows it.
    use_effect(use_reactive!(|kept_name| name.set(kept_name)));
    let aliases: Vec<Identity> = all
        .iter()
        .filter(|one| one.default == IsDefault::Alternate)
        .cloned()
        .collect();
    let open = editing();
    let mut items: Vec<ListItem<String>> = aliases
        .iter()
        .map(|alias| {
            let shown = alias.from.email.clone();
            let detail = alias.from.display_name().map(TextLine::from);
            let opened = Editing {
                id: Some(alias.id),
                typed: Typed::of(alias),
                refused: None,
            };
            let on = open.as_ref().is_some_and(|open| open.id == Some(alias.id));
            ListItem::row(
                alias.id.to_string(),
                shown.clone(),
                rsx! {
                    Row {
                        title: shown.clone(),
                        detail,
                        size: RowSize::Settings,
                        accessory: if on { Accessory::Check(Check::On) } else { Accessory::Chevron },
                        onclick: Some(EventHandler::new(move |_| editing.set(Some(opened.clone())))),
                        common: Common {
                            id: Some(alias_row_id(alias.id)),
                            aria_label: Some(copy::aria::details_for(&shown)),
                            ..Common::default()
                        },
                    }
                },
            )
        })
        .collect();
    items.push(ListItem::row(
        ADD.to_owned(),
        copy::controls::ADD_ALIAS.to_owned(),
        rsx! {
            Row {
                leading: crate::ui::common::add_mark(),
                title: copy::controls::ADD_ALIAS,
                size: RowSize::Settings,
                onclick: Some(EventHandler::new(move |_| editing.set(Some(Editing {
                    id: None,
                    typed: Typed::default(),
                    refused: None,
                })))),
                common: Common {
                    id: Some(ADD_ALIAS_ROW.to_owned()),
                    aria_label: Some(copy::controls::ADD_ALIAS.to_owned()),
                    ..Common::default()
                },
            }
        },
    ));
    let for_name = account.clone();
    let for_submit = account.clone();
    rsx! {
        FormSection { title: Some(copy::headers::SENDING_AS.to_owned()),
            FieldRow { label: copy::fields::FULL_NAME,
                TextField {
                    label: copy::fields::FULL_NAME,
                    value: name(),
                    placeholder: copy::fields::FULL_NAME_EXAMPLE,
                    oninput: move |typed: String| name.set(typed),
                    onblur: move |()| keep_name(&for_name, &name.peek(), failed),
                    onsubmit: move |_| keep_name(&for_submit, &name.peek(), failed),
                }
            }
            List::<String> { label: copy::aria::ALIASES, items, style: ListStyle::Grouped }
        }
        if let Some(why) = unread.filter(|_| !unread_seen()) {
            Problem { title: copy::titles::NOT_READ, why, onclose: move |()| unread_seen.set(true) }
        }
        if let Some((title, why)) = failed() {
            Problem { title, why, onclose: move |()| failed.set(None) }
        }
        if let Some(open) = open {
            FormSection { title: Some(copy::headers::ALIAS.to_owned()),
                {field(account.clone(), copy::fields::NAME, copy::fields::FULL_NAME_EXAMPLE, open.typed.name.clone(), editing, |typed, text| typed.name = text)}
                {field(account.clone(), copy::fields::EMAIL, copy::fields::ALIAS_EXAMPLE, open.typed.email.clone(), editing, |typed, text| typed.email = text)}
                {field(account.clone(), copy::fields::REPLY_TO, copy::fields::REPLY_TO_EXAMPLE, open.typed.reply_to.clone(), editing, |typed, text| typed.reply_to = text)}
                if let Some(why) = open.refused.clone() {
                    FieldRow { label: copy::fields::STATUS,
                        Label { text: why }
                    }
                }
            }
            div { class: "alias-actions",
                Button {
                    label: copy::controls::SAVE,
                    common: Common { aria_label: Some(copy::controls::SAVE.to_owned()), ..Common::default() },
                    onclick: on_primary({
                        let account = account.clone();
                        move || save(&account, editing)
                    }),
                }
                if let Some(id) = open.id {
                    Button {
                        label: copy::controls::REMOVE_ALIAS,
                        role: ButtonRole::Destructive,
                        common: Common { aria_label: Some(copy::controls::REMOVE_ALIAS.to_owned()), ..Common::default() },
                        onclick: on_primary(move || remove(id, editing)),
                    }
                }
                Button {
                    label: copy::controls::CANCEL,
                    common: Common { aria_label: Some(copy::controls::CANCEL.to_owned()), ..Common::default() },
                    onclick: on_primary(move || editing.set(None)),
                }
            }
        }
    }
}

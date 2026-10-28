//! Settings › Contacts: the address book, a pane of two levels as Spaces is. Its root lists every
//! person's card, found by a search field over their names and addresses, then New Contact; then
//! every group, then New Group. A card's row pushes the card's page, a group's row the group's.
//!
//! A card's page holds what a card says about a person: their photo, name, email addresses,
//! telephone numbers, postal addresses, organisation and a note. It is a draft until Save (Add
//! Contact for a new card), which keeps it as `mail_core::contacts::people` makes an edit: onto
//! the card as it is, so what the page does not show stays on the card, and a synced card goes
//! back to its address book on the next sync. A field left empty at the end of a list is how a
//! person adds one more address or number. Delete Contact… asks first, and is there only for a
//! card made here: a synced card is its address book's to delete.
//!
//! A group's page is written as it is used: its name when the field is left or on Return, a
//! member added from the field under the members, one taken out by its row's Remove. Every
//! write moves the pane's `edits`, which the pages read, so what they list follows.

use dioxus::prelude::*;
use ds::components::content::avatar::{
    AvatarFace, AvatarShape, AvatarSize, AvatarTone, person_hue,
};
use ds::components::controls::button_model::ButtonRole;
use ds::components::fields::field_row::FieldRow;
use ds::components::fields::text_field_model::{FieldKind, FieldRows};
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::Client;
use mail_client::contacts::people::Details;
use mail_client::contacts::{CardId, GroupHome, GroupId, PersonCard};
use mail_pim::Postal;

use super::book;
use crate::ui::copy;
use crate::ui::motion::{Follow, tell};
use crate::ui::pick::{Ask, choose, file_name};
use crate::ui::press::on_primary;
use crate::ui::view::{ContactsPage, Shell};

/// The stack's element id: its back buttons are `contacts-pane-back-root` and `-detail`.
const STACK: &str = "contacts-pane";

/// The element id of the New Contact row, where the keyboard goes back to from a new card.
const NEW_ROW: &str = "contacts-pane-new";

/// The element id of the row that opens `page`, where the keyboard goes back to.
fn row_id(page: &ContactsPage) -> Option<String> {
    match page {
        ContactsPage::List => None,
        ContactsPage::Person(id) => Some(format!("contacts-pane-card-{}", key_of(&id.0))),
        ContactsPage::NewPerson => Some(NEW_ROW.to_owned()),
        ContactsPage::Group(id) => Some(format!("contacts-pane-group-{}", key_of(&id.0))),
    }
}

/// An id as an element id may hold it: letters and digits kept, anything else a dash.
fn key_of(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Push `page` over the list.
pub(in crate::ui) fn push(mut shell: Signal<Shell>, page: ContactsPage) {
    shell.write().contacts_pane = PanePath::new(ContactsPage::List).pushed(page);
}

/// Back to the list, as the back button, Escape and ⌘[ ask.
pub(in crate::ui) fn back(mut shell: Signal<Shell>) {
    let popped = shell.peek().contacts_pane.popped();
    shell.write().contacts_pane = popped;
}

fn client() -> Client {
    crate::ui::live::client()
}

/// The first character of `name`, uppercased, for an avatar.
fn initial(name: &str) -> char {
    name.chars()
        .next()
        .and_then(|c| c.to_uppercase().next())
        .unwrap_or('?')
}

/// A card's face: its photo when it has one written in, else its initial on its address's hue.
fn face(card: &PersonCard, size: AvatarSize) -> RowLeading {
    let address = card.addresses.first().cloned().unwrap_or_default();
    match card
        .addresses
        .first()
        .and_then(|a| book::photo(&client(), a))
    {
        Some(src) => RowLeading::Element(rsx! {
            img { class: "contact-photo", "data-size": "row", src: "{src}", alt: "" }
        }),
        None => RowLeading::Avatar(AvatarFace {
            initial: initial(&card.name),
            size,
            tone: AvatarTone::Person(person_hue(&address)),
            shape: AvatarShape::Round,
        }),
    }
}

/// The pane: the people and groups at its root, a card or a group over them.
#[component]
pub(in crate::ui) fn Contacts(shell: Signal<Shell>) -> Element {
    let edits = use_signal(|| 0u64);
    let path = shell.read().contacts_pane.clone();
    let title = use_callback(move |page: ContactsPage| match page {
        ContactsPage::List => copy::titles::CONTACTS.to_owned(),
        ContactsPage::Person(id) => book::card(&client(), &id)
            .map(|card| card.name)
            .unwrap_or_default(),
        ContactsPage::NewPerson => copy::controls::NEW_CONTACT.to_owned(),
        ContactsPage::Group(id) => book::group(&client(), &id)
            .map(|group| group.name)
            .unwrap_or_default(),
    });
    let page = use_callback(move |page: ContactsPage| match page {
        ContactsPage::List => rsx! { ContactList { shell, edits } },
        ContactsPage::Person(id) => rsx! { CardPage { key: "{id}", shell, edits, id: Some(id) } },
        ContactsPage::NewPerson => rsx! { CardPage { shell, edits, id: None } },
        ContactsPage::Group(id) => rsx! { GroupPage { key: "{id}", shell, edits, id } },
    });
    let opener = use_callback(|page: ContactsPage| row_id(&page));
    rsx! {
        PaneStack::<ContactsPage> {
            path,
            title,
            page,
            opener: Some(opener),
            on_back: move |()| back(shell),
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
    }
}

/// The root: the search field, every card found and New Contact, then every group and New Group.
#[component]
fn ContactList(shell: Signal<Shell>, edits: Signal<u64>) -> Element {
    let mut filter = use_signal(String::new);
    // Read, so a write elsewhere on the pane lists again.
    let _ = edits();
    let client = client();
    let cards = book::cards(&client, &filter());
    let people: Vec<ListItem<String>> = cards
        .iter()
        .map(|card| {
            let page = ContactsPage::Person(card.id.clone());
            let id = row_id(&page).unwrap_or_default();
            let name = card.name.clone();
            let address = card.addresses.first().cloned();
            ListItem::row(
                id.clone(),
                name.clone(),
                rsx! {
                    Row {
                        leading: face(card, AvatarSize::Size28),
                        title: name.clone(),
                        detail: address.filter(|a| *a != name).map(TextLine::from),
                        accessory: Accessory::Chevron,
                        size: RowSize::Settings,
                        onclick: move |_| push(shell, page.clone()),
                        common: Common {
                            id: Some(id),
                            aria_label: Some(copy::aria::details_for(&name)),
                            ..Common::default()
                        },
                    }
                },
            )
        })
        .collect();
    let new_contact = vec![ListItem::row(
        NEW_ROW.to_owned(),
        copy::controls::NEW_CONTACT.to_owned(),
        rsx! {
            Row {
                leading: crate::ui::common::add_mark(),
                title: copy::controls::NEW_CONTACT,
                size: RowSize::Settings,
                onclick: move |_| push(shell, ContactsPage::NewPerson),
                common: Common {
                    id: Some(NEW_ROW.to_owned()),
                    aria_label: Some(copy::aria::ADD_A_CONTACT.to_owned()),
                    ..Common::default()
                },
            }
        },
    )];
    let groups: Vec<ListItem<String>> = book::every_group(&client)
        .into_iter()
        .map(|group| {
            let page = ContactsPage::Group(group.id.clone());
            let id = row_id(&page).unwrap_or_default();
            let name = group.name.clone();
            ListItem::row(
                id.clone(),
                name.clone(),
                rsx! {
                    Row {
                        leading: RowLeading::Icon(Icon::Group),
                        title: name.clone(),
                        accessory: Accessory::Text(copy::status::members(group.members.len())),
                        size: RowSize::Settings,
                        onclick: move |_| push(shell, page.clone()),
                        common: Common {
                            id: Some(id),
                            aria_label: Some(copy::aria::details_for(&name)),
                            ..Common::default()
                        },
                    }
                },
            )
        })
        .collect();
    let new_group = vec![ListItem::row(
        "contacts-pane-new-group".to_owned(),
        copy::controls::NEW_GROUP.to_owned(),
        rsx! {
            Row {
                leading: crate::ui::common::add_mark(),
                title: copy::controls::NEW_GROUP,
                size: RowSize::Settings,
                onclick: move |_| make_group(shell, edits),
                common: Common {
                    id: Some("contacts-pane-new-group".to_owned()),
                    aria_label: Some(copy::aria::ADD_A_GROUP.to_owned()),
                    ..Common::default()
                },
            }
        },
    )];
    rsx! {
        Form {
            FormSection {
                TextField {
                    label: copy::fields::FIND_CONTACTS,
                    value: filter(),
                    placeholder: copy::fields::FIND_CONTACTS,
                    kind: FieldKind::Search,
                    oninput: move |typed: String| filter.set(typed),
                    common: Common { id: Some("contacts-pane-find".to_owned()), ..Common::default() },
                }
            }
            FormSection { title: Some(copy::headers::PEOPLE.to_owned()),
                if !people.is_empty() {
                    List::<String> { label: copy::headers::PEOPLE, items: people, style: ListStyle::Grouped }
                }
            }
            FormSection {
                List::<String> { label: copy::aria::ADD_A_CONTACT, items: new_contact, style: ListStyle::Grouped }
            }
            FormSection { title: Some(copy::headers::GROUPS.to_owned()),
                if !groups.is_empty() {
                    List::<String> { label: copy::headers::GROUPS, items: groups, style: ListStyle::Grouped }
                }
            }
            FormSection {
                List::<String> { label: copy::aria::ADD_A_GROUP, items: new_group, style: ListStyle::Grouped }
            }
        }
    }
}

/// New Group: a group of nobody, named as the row is, and its page pushed to name it and fill it.
fn make_group(shell: Signal<Shell>, mut edits: Signal<u64>) {
    match book::new_group(&client(), copy::controls::NEW_GROUP) {
        Ok(group) => {
            edits += 1;
            push(shell, ContactsPage::Group(group.id));
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}

/// Whether a card's page is asking before the card is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deleting {
    Showing,
    Asking,
}

/// A list of lines of a card — its addresses or its numbers — with one empty field after them
/// for one more. `at` is the line typed into, `typed` what it now says.
fn typed_into(lines: &mut Vec<String>, at: usize, typed: String) {
    if at < lines.len() {
        lines[at] = typed;
    } else {
        lines.push(typed);
    }
}

/// A card's page: its photo, name, addresses, numbers, postal addresses, organisation and note,
/// kept by Save; a new card's (`id` none) by Add Contact.
#[component]
fn CardPage(shell: Signal<Shell>, edits: Signal<u64>, id: Option<CardId>) -> Element {
    let mut draft = use_signal({
        let id = id.clone();
        move || match &id {
            Some(id) => book::details(&client(), id),
            None => Details::default(),
        }
    });
    let mut deleting = use_signal(|| Deleting::Showing);
    let held = id.as_ref().and_then(|id| book::card(&client(), id));
    let local = held
        .as_ref()
        .is_some_and(|card| card.home == GroupHome::Local);
    let shown = draft();
    let name = shown.name.clone();
    let emails: Vec<String> = shown
        .emails
        .iter()
        .cloned()
        .chain([String::new()])
        .collect();
    let phones: Vec<String> = shown
        .phones
        .iter()
        .cloned()
        .chain([String::new()])
        .collect();
    let save = {
        let id = id.clone();
        move || keep(shell, edits, id.as_ref(), &draft.peek())
    };
    let save_on_enter = save.clone();
    let photo = shown.photo.clone().filter(|p| p.starts_with("data:image/"));
    rsx! {
        Form {
            FormSection {
                FieldRow { label: copy::fields::PHOTO,
                    div { class: "contact-photo-row",
                        match photo {
                            Some(src) => rsx! {
                                img { class: "contact-photo", "data-size": "page", src: "{src}", alt: copy::aria::photo_of(&name) }
                            },
                            None => rsx! {
                                Avatar {
                                    initial: initial(&name),
                                    size: AvatarSize::Size48,
                                    tone: AvatarTone::Person(person_hue(shown.emails.first().map_or("", String::as_str))),
                                }
                            },
                        }
                        Button {
                            label: copy::controls::CHOOSE_PHOTO,
                            onclick: on_primary(move || choose_photo(draft)),
                        }
                        if shown.photo.is_some() {
                            Button {
                                label: copy::controls::REMOVE_PHOTO,
                                onclick: on_primary(move || draft.write().photo = None),
                            }
                        }
                    }
                }
                FieldRow { label: copy::fields::NAME,
                    TextField {
                        label: copy::fields::NAME,
                        value: name.clone(),
                        placeholder: copy::fields::NAME,
                        oninput: move |typed: String| draft.write().name = typed,
                        onsubmit: move |_| save_on_enter(),
                        common: Common { id: Some("contact-name".to_owned()), ..Common::default() },
                    }
                }
                for (at, email) in emails.into_iter().enumerate() {
                    FieldRow { key: "email-{at}", label: copy::fields::EMAIL,
                        TextField {
                            label: copy::fields::EMAIL,
                            value: email,
                            placeholder: copy::fields::ANOTHER_EMAIL,
                            oninput: move |typed: String| typed_into(&mut draft.write().emails, at, typed),
                            common: Common { id: Some(format!("contact-email-{at}")), ..Common::default() },
                        }
                    }
                }
                for (at, phone) in phones.into_iter().enumerate() {
                    FieldRow { key: "phone-{at}", label: copy::fields::PHONE,
                        TextField {
                            label: copy::fields::PHONE,
                            value: phone,
                            placeholder: copy::fields::ANOTHER_PHONE,
                            oninput: move |typed: String| typed_into(&mut draft.write().phones, at, typed),
                            common: Common { id: Some(format!("contact-phone-{at}")), ..Common::default() },
                        }
                    }
                }
                FieldRow { label: copy::fields::ORGANISATION,
                    TextField {
                        label: copy::fields::ORGANISATION,
                        value: shown.org.clone(),
                        oninput: move |typed: String| draft.write().org = typed,
                    }
                }
            }
            FormSection { title: Some(copy::headers::ADDRESSES.to_owned()),
                for (at, postal) in shown.postal.iter().cloned().enumerate() {
                    PostalFields { key: "postal-{at}", draft, at, postal }
                }
                div { class: "acct-actions",
                    Button {
                        label: copy::controls::ADD_ADDRESS,
                        onclick: on_primary(move || draft.write().postal.push(Postal::default())),
                    }
                }
            }
            FormSection {
                FieldRow { label: copy::fields::NOTE,
                    TextField {
                        label: copy::fields::NOTE,
                        value: shown.note.clone(),
                        kind: FieldKind::Multiline,
                        rows: FieldRows::Three,
                        oninput: move |typed: String| draft.write().note = typed,
                    }
                }
            }
            div { class: "acct-actions",
                Button {
                    label: if id.is_some() { copy::controls::SAVE } else { copy::controls::ADD_CONTACT },
                    onclick: on_primary(save),
                    common: Common { id: Some("contact-save".to_owned()), ..Common::default() },
                }
                if local {
                    Button {
                        label: copy::controls::DELETE_CONTACT,
                        role: ButtonRole::Destructive,
                        common: Common { aria_label: Some(copy::confirm::delete_contact(&name)), ..Common::default() },
                        onclick: on_primary(move || deleting.set(Deleting::Asking)),
                    }
                }
            }
        }
        if deleting() == Deleting::Asking {
            Alert {
                title: copy::confirm::delete_contact(&name),
                message: Some(TextLine::from(copy::confirm::DELETE_CONTACT_KEEPS)),
                style: AlertStyle::Critical,
                buttons: vec![
                    AlertButton::new(copy::controls::DELETE, AlertRole::Destructive, EventHandler::new({
                        let id = id.clone();
                        move |()| {
                            deleting.set(Deleting::Showing);
                            if let Some(id) = &id {
                                forget(shell, edits, id);
                            }
                        }
                    })),
                    AlertButton::new(
                        copy::controls::CANCEL,
                        AlertRole::Cancel,
                        EventHandler::new(move |()| deleting.set(Deleting::Showing)),
                    ),
                ],
            }
        }
    }
}

/// One postal address of the draft: its street, city, region, code and country.
#[component]
fn PostalFields(draft: Signal<Details>, at: usize, postal: Postal) -> Element {
    let mut draft = draft;
    let mut set = move |part: fn(&mut Postal) -> &mut String, typed: String| {
        if let Some(postal) = draft.write().postal.get_mut(at) {
            *part(postal) = typed;
        }
    };
    rsx! {
        FieldRow { label: copy::fields::STREET,
            TextField {
                label: copy::fields::STREET,
                value: postal.street.clone(),
                oninput: move |typed: String| set(|p| &mut p.street, typed),
            }
        }
        FieldRow { label: copy::fields::CITY,
            TextField {
                label: copy::fields::CITY,
                value: postal.locality.clone(),
                oninput: move |typed: String| set(|p| &mut p.locality, typed),
            }
        }
        FieldRow { label: copy::fields::REGION,
            TextField {
                label: copy::fields::REGION,
                value: postal.region.clone(),
                oninput: move |typed: String| set(|p| &mut p.region, typed),
            }
        }
        FieldRow { label: copy::fields::POSTAL_CODE,
            TextField {
                label: copy::fields::POSTAL_CODE,
                value: postal.code.clone(),
                oninput: move |typed: String| set(|p| &mut p.code, typed),
            }
        }
        FieldRow { label: copy::fields::COUNTRY,
            TextField {
                label: copy::fields::COUNTRY,
                value: postal.country.clone(),
                oninput: move |typed: String| set(|p| &mut p.country, typed),
            }
        }
    }
}

/// Ask for a picture and put it on the draft, or say why it cannot go there.
fn choose_photo(mut draft: Signal<Details>) {
    choose(Ask::Picture, None, move |paths| {
        spawn(async move {
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let name = file_name(&path);
            let read = tokio::task::spawn_blocking(move || std::fs::read(path)).await;
            match read {
                Ok(Ok(bytes)) => match book::photo_url(&name, &bytes) {
                    Ok(url) => draft.write().photo = Some(url),
                    Err(why) => tell(why, Follow::Nothing),
                },
                _ => tell(copy::errors::cannot_read(&name), Follow::Nothing),
            }
        });
    });
}

/// Keep the draft as `id`'s card, or as a new one, and say only a failure. A new card's page
/// goes back to the list, which shows it.
fn keep(shell: Signal<Shell>, mut edits: Signal<u64>, id: Option<&CardId>, draft: &Details) {
    match book::keep_card(&client(), id, draft) {
        Ok(_) => {
            edits += 1;
            if id.is_none() {
                back(shell);
            }
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}

/// Delete a card and go back to the list.
fn forget(shell: Signal<Shell>, mut edits: Signal<u64>, id: &CardId) {
    match book::forget_card(&client(), id) {
        Ok(()) => {
            edits += 1;
            back(shell);
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}

/// A group's page: its name, its members each with Remove, a field to add more, and Delete
/// Group… for a group made here.
#[component]
fn GroupPage(shell: Signal<Shell>, edits: Signal<u64>, id: GroupId) -> Element {
    // Read, so each write here draws the group again.
    let _ = edits();
    let mut adding = use_signal(String::new);
    let mut deleting = use_signal(|| Deleting::Showing);
    let mut named = use_signal({
        let id = id.clone();
        move || {
            book::group(&client(), &id)
                .map(|group| group.name)
                .unwrap_or_default()
        }
    });
    let client = client();
    let Some(group) = book::group(&client, &id) else {
        // Deleted from elsewhere while the page was shown.
        return rsx! {};
    };
    let local = group.home == GroupHome::Local;
    let name = group.name.clone();
    let members: Vec<ListItem<String>> = book::members(&client, &group)
        .into_iter()
        .map(|(uri, said)| {
            let remove = EventHandler::new({
                let (id, uri) = (id.clone(), uri.clone());
                move |()| {
                    write(edits, |client| book::remove_member(client, &id, &uri));
                }
            });
            ListItem::row(
                uri.clone(),
                said.clone(),
                rsx! {
                    Row {
                        title: said.clone(),
                        size: RowSize::Settings,
                        accessory: Accessory::Slot(rsx! {
                            Button {
                                label: copy::controls::REMOVE,
                                common: Common { aria_label: Some(copy::aria::remove_member(&said)), ..Common::default() },
                                onclick: on_primary(move || remove.call(())),
                            }
                        }),
                    }
                },
            )
        })
        .collect();
    let rename = {
        let id = id.clone();
        move || {
            let typed = named.peek().clone();
            if typed.trim() != name.trim() {
                write(edits, |client| book::rename_group(client, &id, &typed));
            }
        }
    };
    let rename_on_enter = rename.clone();
    let add = {
        let id = id.clone();
        move || {
            let typed = adding.peek().clone();
            if typed.trim().is_empty() {
                return;
            }
            if write(edits, |client| book::add_members(client, &id, &typed)) {
                adding.set(String::new());
            }
        }
    };
    let mut add_on_enter = add.clone();
    rsx! {
        Form {
            FormSection {
                FieldRow { label: copy::fields::NAME,
                    TextField {
                        label: copy::fields::GROUP_NAME,
                        value: named(),
                        placeholder: copy::fields::GROUP_NAME,
                        oninput: move |typed: String| named.set(typed),
                        onblur: move |()| rename(),
                        onsubmit: move |_| rename_on_enter(),
                        common: Common { id: Some("group-name".to_owned()), ..Common::default() },
                    }
                }
            }
            FormSection { title: Some(copy::headers::MEMBERS.to_owned()),
                if !members.is_empty() {
                    List::<String> { label: copy::headers::MEMBERS, items: members, style: ListStyle::Grouped }
                }
                FieldRow { label: copy::fields::EMAIL,
                    TextField {
                        label: copy::fields::ADD_MEMBERS,
                        value: adding(),
                        placeholder: copy::fields::ADD_MEMBERS,
                        oninput: move |typed: String| adding.set(typed),
                        onsubmit: move |_| add_on_enter(),
                        common: Common { id: Some("group-add".to_owned()), ..Common::default() },
                    }
                    Button {
                        label: copy::controls::ADD,
                        onclick: on_primary(add),
                        common: Common { id: Some("group-add-button".to_owned()), ..Common::default() },
                    }
                }
            }
            if local {
                div { class: "acct-actions",
                    Button {
                        label: copy::controls::DELETE_GROUP,
                        role: ButtonRole::Destructive,
                        common: Common { aria_label: Some(copy::confirm::delete_group(&group.name)), ..Common::default() },
                        onclick: on_primary(move || deleting.set(Deleting::Asking)),
                    }
                }
            }
        }
        if deleting() == Deleting::Asking {
            Alert {
                title: copy::confirm::delete_group(&group.name),
                message: Some(TextLine::from(copy::confirm::DELETE_GROUP_KEEPS)),
                style: AlertStyle::Critical,
                buttons: vec![
                    AlertButton::new(copy::controls::DELETE, AlertRole::Destructive, EventHandler::new({
                        let id = id.clone();
                        move |()| {
                            deleting.set(Deleting::Showing);
                            if write(edits, |client| book::forget_group(client, &id)) {
                                back(shell);
                            }
                        }
                    })),
                    AlertButton::new(
                        copy::controls::CANCEL,
                        AlertRole::Cancel,
                        EventHandler::new(move |()| deleting.set(Deleting::Showing)),
                    ),
                ],
            }
        }
    }
}

/// Make one write of the pane's, move `edits` when it is made, and say why when it is not.
/// Whether it was made.
fn write(mut edits: Signal<u64>, made: impl FnOnce(&Client) -> Result<(), String>) -> bool {
    match made(&client()) {
        Ok(()) => {
            edits += 1;
            true
        }
        Err(why) => {
            tell(why, Follow::Nothing);
            false
        }
    }
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;

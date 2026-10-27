//! A person, edited from their name in mail: the reader's sender name, right-clicked, offers Edit
//! Contact…, and this popover hangs from the name; and Add to Contacts, which makes the sender a
//! card, or Show Contact Card when one already lists them, which opens it in Settings. The popover
//! holds the one thing a person writes into the book by hand, the name; Return or Save keeps it
//! ([`book::name`]), and the entry is then the user's, so mail never renames it. Escape or a click
//! outside leaves the book as it was.

use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_client::Client;

use super::book;
use crate::ui::copy;
use crate::ui::menu::{MenuKey, anchor_at, menu_key};
use crate::ui::motion::{Follow, tell};
use crate::ui::press::on_primary;

/// The name the field starts with: the book's, else the one the mail gave.
pub(in crate::ui) fn starting_name(client: &Client, address: &str, in_mail: &str) -> String {
    client
        .stored()
        .contact(address)
        .ok()
        .flatten()
        .and_then(|contact| contact.name)
        .unwrap_or_else(|| in_mail.to_owned())
}

/// Keep `typed` as `address`'s name, saying only a failure.
fn keep(address: &str, typed: &str, on_close: EventHandler<()>) {
    let client = crate::ui::live::client();
    if let Err(why) = book::name(&client, address, typed) {
        tell(why, Follow::Nothing);
    }
    on_close.call(());
}

/// The editor for `address`, hung from `anchor`; `in_mail` is the name the message gave.
#[component]
pub(super) fn ContactEditor(
    address: String,
    in_mail: String,
    anchor: Option<MountedRef>,
    on_close: EventHandler<()>,
) -> Element {
    let mut typed = use_signal({
        let address = address.clone();
        move || {
            let client = crate::ui::live::client();
            starting_name(&client, &address, &in_mail)
        }
    });
    let on_enter = address.clone();
    rsx! {
        Popover {
            anchor: anchor_at(anchor),
            placement: Placement::new(Side::Bottom, Align::Start),
            gap: Px(4.0),
            arrow: Arrow::None,
            onclose: move |()| on_close.call(()),
            div {
                class: "contact-editor",
                onkeydown: move |event: KeyboardEvent| match menu_key(&event.key().to_string()) {
                    Some(MenuKey::Enter) => {
                        event.prevent_default();
                        keep(&on_enter, &typed.peek(), on_close);
                    }
                    Some(MenuKey::Escape) => {
                        event.stop_propagation();
                        on_close.call(());
                    }
                    _ => {}
                },
                Label { text: address.clone(), role: LabelRole::Secondary, style: LabelStyle::Footnote }
                TextField {
                    label: copy::fields::NAME.to_owned(),
                    placeholder: copy::fields::NAME.to_owned(),
                    value: typed(),
                    focus: FieldFocus::OnMount,
                    oninput: move |value: String| typed.set(value),
                }
                Button {
                    label: copy::controls::SAVE,
                    common: Common { aria_label: Some(copy::aria::save_contact(&address)), ..Common::default() },
                    onclick: on_primary({
                        let address = address.clone();
                        move || keep(&address, &typed.peek(), on_close)
                    }),
                }
            }
        }
    }
}

/// Where the sender's name is: at rest, its menu open at the pointer, or its editor open.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Asked {
    Resting,
    Menu(Rect),
    Editing,
}

/// The keys of the name menu's rows.
const EDIT: &str = "edit-contact";
const VIP: &str = "vip";
const ADD: &str = "add-contact";
const SHOW: &str = "show-card";

/// The name menu's second row: Show Contact Card when a card lists the sender, else Add to
/// Contacts.
fn card_row(address: &str) -> crate::ui::menu::MenuItem {
    // A window with no client (a lone component in a test) knows no cards.
    match crate::ui::live::try_client().and_then(|client| book::card_of(&client, address)) {
        Some(_) => {
            crate::ui::menu::MenuItem::icon(SHOW, Icon::Group, copy::menus::SHOW_CONTACT_CARD)
        }
        None => crate::ui::menu::MenuItem::icon(ADD, Icon::Plus, copy::menus::ADD_TO_CONTACTS),
    }
}

/// Add to Contacts, or Show Contact Card: the sender's card, made if there was none, opened in
/// Settings › Contacts. A new card is said in a toast.
fn show_card(address: &str, in_mail: &str) {
    let client = crate::ui::live::client();
    match book::add_sender(&client, address, in_mail) {
        Ok((card, said)) => {
            if let Some(said) = said {
                tell(said, Follow::Nothing);
            }
            crate::ui::settings_window::open_card(card);
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}

/// A sender's name in the reader's header, in the headline type: `shown`, which is their address
/// when the mail gave no name (`in_mail` empty), and the VIP mark after it when they are one. A
/// right click offers Edit Contact…, which opens [`ContactEditor`] hung from the name, and Add to
/// VIPs or Remove from VIPs where the reader says where a mark lands
/// ([`crate::ui::vip::Marking`]).
#[component]
pub(in crate::ui) fn SenderName(shown: String, in_mail: String, address: String) -> Element {
    let mut asked = use_signal(|| Asked::Resting);
    let mut at = use_signal(|| None::<MountedRef>);
    let marking = try_use_context::<crate::ui::vip::Marking>();
    let vips = marking.map(|marking| marking.shell.read().vips.clone());
    let vip = vips.as_ref().is_some_and(|vips| vips.includes(&address));
    let mut items = vec![crate::ui::menu::MenuItem::icon(
        EDIT,
        Icon::Pen,
        copy::menus::EDIT_CONTACT,
    )];
    items.push(card_row(&address));
    let markable =
        marking.is_some_and(|marking| crate::ui::vip::markable(&marking.shell.read(), &address));
    if let (Some(vips), true) = (&vips, markable) {
        items.push(crate::ui::menu::MenuItem::icon(
            VIP,
            crate::ui::vip::GLYPH,
            crate::ui::vip::menu_words(vips, &address),
        ));
    }
    let on_vip = {
        let (address, in_mail) = (address.clone(), in_mail.clone());
        move || {
            if let (Some(marking), Some(vips)) = (marking, &vips) {
                let name = Some(in_mail.as_str()).filter(|name| !name.trim().is_empty());
                let mark = mail_client::vip::toggled(vips, &address);
                crate::ui::vip::set(marking, &address, name, mark);
            }
        }
    };
    rsx! {
        span {
            class: "sender-name",
            oncontextmenu: move |event: MouseEvent| {
                event.prevent_default();
                event.stop_propagation();
                asked.set(Asked::Menu(crate::ui::row::point_rect(event.client_coordinates())));
            },
            onmounted: move |event: MountedEvent| at.set(Some(MountedRef(event.data()))),
            Label { text: shown, style: LabelStyle::Headline, common: crate::ui::common::classed("truncate") }
        }
        if vip {
            crate::ui::vip::VipMark {}
        }
        match asked() {
            Asked::Resting => rsx! {},
            Asked::Menu(placed) => rsx! {
                crate::ui::menu::Floating {
                    anchor: None,
                    placed: Some(placed),
                    title: String::new(),
                    items,
                    on_pick: {
                        let (address, in_mail) = (address.clone(), in_mail.clone());
                        move |key: String| {
                            if key == VIP {
                                asked.set(Asked::Resting);
                                on_vip();
                            } else if key == EDIT {
                                asked.set(Asked::Editing);
                            } else {
                                asked.set(Asked::Resting);
                                show_card(&address, &in_mail);
                            }
                        }
                    },
                    on_close: move |()| {
                        if matches!(*asked.peek(), Asked::Menu(_)) {
                            asked.set(Asked::Resting);
                        }
                    },
                }
            },
            Asked::Editing => rsx! {
                ContactEditor {
                    address: address.clone(),
                    in_mail: in_mail.clone(),
                    anchor: at(),
                    on_close: move |()| asked.set(Asked::Resting),
                }
            },
        }
    }
}

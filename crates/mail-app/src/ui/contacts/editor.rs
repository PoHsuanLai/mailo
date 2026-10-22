//! A person, edited from their name in mail: the reader's sender name, right-clicked, offers Edit
//! Contact…, and this popover hangs from the name. It holds the one thing a person writes into
//! the book by hand, the name; Return or Save keeps it ([`book::name`]), and the entry is then
//! the user's, so mail never renames it. Escape or a click outside leaves the book as it was.

use std::sync::Arc;

use dioxus::prelude::*;
use ds::base::geometry::placement::{Align, Side};
use ds::components::content::label::{LabelRole, LabelStyle};
use ds::components::overlays::popover::Arrow;
use ds::host::measure::MountedRef;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::{SqliteStore, Store};

use super::book;
use crate::ui::copy;
use crate::ui::menu::{MenuKey, anchor_at, menu_key};
use crate::ui::motion::{Follow, tell};
use crate::ui::press::on_primary;

/// The name the field starts with: the book's, else the one the mail gave.
pub(in crate::ui) fn starting_name(store: &dyn Store, address: &str, in_mail: &str) -> String {
    store
        .contact(address)
        .ok()
        .flatten()
        .and_then(|contact| contact.name)
        .unwrap_or_else(|| in_mail.to_owned())
}

/// Keep `typed` as `address`'s name, saying only a failure.
fn keep(address: &str, typed: &str, on_close: EventHandler<()>) {
    let store = consume_context::<Arc<SqliteStore>>();
    if let Err(why) = book::name(store.as_ref(), address, typed) {
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
            let store = consume_context::<Arc<SqliteStore>>();
            starting_name(store.as_ref(), &address, &in_mail)
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

/// The key of the name menu's one row.
const EDIT: &str = "edit-contact";

/// A sender's name in the reader's header, in the headline type: `shown`, which is their address
/// when the mail gave no name (`in_mail` empty). A right click offers Edit Contact…, which opens
/// [`ContactEditor`] hung from the name.
#[component]
pub(in crate::ui) fn SenderName(shown: String, in_mail: String, address: String) -> Element {
    let mut asked = use_signal(|| Asked::Resting);
    let mut at = use_signal(|| None::<MountedRef>);
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
        match asked() {
            Asked::Resting => rsx! {},
            Asked::Menu(placed) => rsx! {
                crate::ui::menu::Floating {
                    anchor: None,
                    placed: Some(placed),
                    title: String::new(),
                    items: vec![crate::ui::menu::MenuItem::icon(EDIT, Icon::Pen, copy::menus::EDIT_CONTACT)],
                    on_pick: move |_: String| asked.set(Asked::Editing),
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

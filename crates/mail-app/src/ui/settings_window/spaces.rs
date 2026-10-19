//! Spaces: a pane of two levels, as Accounts is. Its root lists each Space by its colour and its
//! name, with how many accounts it shows, a chevron row that pushes the Space's own page; then
//! New Space. A Space's page is what the Space's menu holds (Edit Space…, Appearance, Accounts ▸),
//! as rows: its name, its colour (quire's presets), its theme, the accounts it shows, and, last
//! and alone, Delete Space…, which asks first and is there only while another Space is.
//!
//! The Space's menu, a right click on the Space's name, stays the quick way in; this is where
//! the same things are found. The window edits the Spaces through its own quire handle
//! ([`super::root`]): a pick is written at once (`frame::keep`) and the other windows read the
//! file again. A name is changed as it is typed and written when the field is left, on Return,
//! or when the page is left. The editor's full colour field stays Edit Space…'s, in the menu.

use crate::ui::copy;
use crate::ui::space::{Handle, Mail, SpaceId, held_elsewhere};
use crate::ui::space_menu::{accounts, every_id, toggled};
use crate::ui::view::{Shell, SpacesPage};
use dioxus::prelude::*;
use ds::base::vocab::RowState;
use ds::components::app::space_editor::swatches::PresetSwatches;
use ds::components::controls::button_model::ButtonRole;
use ds::components::controls::choice::Choice;
use ds::components::controls::segmented::{SegmentedControl, Tracking};
use ds::components::fields::field_row::FieldRow;
use ds::components::lists::list::model::ListStyle;
use ds::components::lists::row::size::RowSize;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use ds::root::common::Common;
use ds::style::appearance::theme::Theme;
use ds::style::icon::family::PlateFamily;
use ds::style::scope::use_scope;
use ds::style::space::dot_paint::DotPaint;
use ds::style::space::look::SpaceLook;
use ds::style::space::palette::derive;
use mail_core::profile::ProfileId;
use porter_core::AccountId;

/// The key of the New Space row, which no Space's id can be.
const NEW: &str = " new";

/// The stack's element id: its back buttons are `spaces-pane-back-root` and `-detail`.
const STACK: &str = "spaces-pane";

/// The element id of the row that pushes `space`, where the keyboard goes back to.
fn row_id(space: SpaceId) -> String {
    format!("spaces-pane-open-{}", space.0)
}

/// Whether a Space's page is asking before the Space is deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Deleting {
    Showing,
    Asking,
}

/// The pane: the list at its root, a Space's page over it.
#[component]
pub(super) fn Spaces(shell: Signal<Shell>, handle: Handle) -> Element {
    let path = shell.read().spaces_pane.clone();
    let title = use_callback(move |page: SpacesPage| match page {
        SpacesPage::List => copy::titles::SPACES.to_owned(),
        SpacesPage::Space(space) => handle
            .spaces()
            .read()
            .get(space)
            .map(|one| one.name.clone())
            .unwrap_or_default(),
    });
    let page = use_callback(move |page: SpacesPage| match page {
        SpacesPage::List => rsx! { SpaceList { shell, handle } },
        SpacesPage::Space(space) => rsx! { SpacePage { shell, handle, space } },
    });
    let opener = use_callback(|page: SpacesPage| match page {
        SpacesPage::List => None,
        SpacesPage::Space(space) => Some(row_id(space)),
    });
    rsx! {
        PaneStack::<SpacesPage> {
            path,
            title,
            page,
            opener: Some(opener),
            on_back: move |()| back(shell, handle),
            common: Common { id: Some(STACK.to_owned()), ..Common::default() },
        }
    }
}

/// The root: every Space, then New Space.
#[component]
fn SpaceList(shell: Signal<Shell>, handle: Handle) -> Element {
    let scheme = use_scope().scheme;
    let held = handle.spaces();
    let spaces = held.read();
    let mut items: Vec<ListItem<String>> = Vec::new();
    for space in spaces.list() {
        let id = space.id;
        let paint = DotPaint::gradient(&derive(&space.look.dots, scheme).picked);
        let name = space.name.clone();
        items.push(ListItem::row(
            row_id(id),
            name.clone(),
            rsx! {
                Row {
                    leading: RowLeading::Element(rsx! {
                        span {
                            class: "ds-space-swatch",
                            "data-stops": paint.count(),
                            style: paint.style_attr(),
                        }
                    }),
                    title: name.clone(),
                    accessory: Accessory::Text(copy::status::accounts_shown(space.payload.accounts.len())),
                    size: RowSize::Settings,
                    onclick: move |_| push(shell, id),
                    common: Common {
                        id: Some(row_id(id)),
                        aria_label: Some(copy::aria::details_for(&name)),
                        ..Common::default()
                    },
                }
            },
        ));
    }
    items.push(ListItem::row(
        NEW.to_owned(),
        copy::controls::NEW_SPACE.to_owned(),
        rsx! {
            Row {
                leading: crate::ui::common::tile(Icon::Plus, PlateFamily::Blue),
                title: copy::controls::NEW_SPACE,
                size: RowSize::Settings,
                onclick: move |_| {
                    let id = handle.add(Mail::new_in(&ProfileId::default(), every_id()));
                    push(shell, id);
                },
                common: Common {
                    aria_label: Some(copy::controls::NEW_SPACE.to_owned()),
                    ..Common::default()
                },
            }
        },
    ));
    rsx! {
        Form {
            FormSection {
                List::<String> { label: copy::titles::SPACES, items, style: ListStyle::Grouped }
            }
        }
    }
}

/// A Space's page: its name, colour and theme, the accounts it shows, and Delete Space….
#[component]
fn SpacePage(shell: Signal<Shell>, handle: Handle, space: SpaceId) -> Element {
    let mut deleting = use_signal(|| Deleting::Showing);
    let held = handle.spaces();
    let spaces = held.read();
    let Some(one) = spaces.get(space) else {
        // Deleted from elsewhere while the page was shown.
        return rsx! {};
    };
    let name = one.name.clone();
    let look = one.look.clone();
    let shown = one.payload.accounts.clone();
    let several = spaces.count() > 1;
    let rows: Vec<ListItem<String>> = accounts()
        .into_iter()
        .map(|(account, label)| {
            let on = shown.contains(&account);
            // An account no other Space shows stays here: every account is in some Space.
            let stays = on && !held_elsewhere(&spaces, space, &account);
            account_row(handle, space, account, label, on, stays)
        })
        .collect();
    let theme = look.theme;
    rsx! {
        Form {
            FormSection {
                FieldRow { label: copy::fields::NAME,
                    TextField {
                        label: copy::fields::SPACE_NAME,
                        value: name.clone(),
                        placeholder: copy::fields::SPACE_NAME,
                        oninput: move |typed: String| handle.change(space, |one| one.name = typed),
                        onblur: move |()| handle.commit(),
                        onsubmit: move |_| handle.commit(),
                    }
                }
                FieldRow { label: copy::fields::COLOUR,
                    PresetSwatches {
                        look: look.clone(),
                        onchange: move |look: SpaceLook| handle.apply(space, |one| one.look = look),
                        common: Common { aria_label: Some(copy::fields::COLOUR.to_owned()), ..Common::default() },
                    }
                }
                FieldRow { label: copy::fields::APPEARANCE,
                    SegmentedControl::<Theme> {
                        label: copy::fields::APPEARANCE,
                        choices: Choice::pairs(Theme::ALL.iter().copied().map(|theme| (theme, theme.label().to_string())).collect::<Vec<_>>()),
                        tracking: Tracking::SelectOne(theme),
                        onchange: move |theme: Theme| handle.apply(space, |one| one.look.theme = theme),
                    }
                }
            }
            if !rows.is_empty() {
                FormSection { title: Some(copy::titles::ACCOUNTS.to_owned()),
                    List::<String> { label: copy::aria::ACCOUNTS_IN_SPACE, items: rows, style: ListStyle::Grouped }
                }
            }
            // Deleting is the page's last part, alone and outside any group, as Remove Account…
            // is: it cannot be undone, so it asks first. The last Space cannot go.
            if several {
                div { class: "acct-actions",
                    Button {
                        label: copy::controls::DELETE_SPACE,
                        role: ButtonRole::Destructive,
                        common: Common { aria_label: Some(copy::confirm::delete_space(&name)), ..Common::default() },
                        onclick: crate::ui::press::on_primary(move || deleting.set(Deleting::Asking)),
                    }
                }
            }
        }
        if deleting() == Deleting::Asking {
            Alert {
                title: copy::confirm::delete_space(&name),
                message: Some(TextLine::from(copy::confirm::DELETE_SPACE_KEEPS)),
                style: AlertStyle::Critical,
                buttons: vec![
                    AlertButton::new(copy::controls::DELETE, AlertRole::Destructive, EventHandler::new(move |()| {
                        deleting.set(Deleting::Showing);
                        delete(shell, handle, space);
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

/// One account's row on a Space's page: a switch for whether the Space shows it. An account no
/// other Space shows cannot be switched off, and says so.
fn account_row(
    handle: Handle,
    space: SpaceId,
    account: AccountId,
    label: String,
    on: bool,
    stays: bool,
) -> ListItem<String> {
    let value = if on { Check::On } else { Check::Off };
    let availability = if stays {
        Availability::Disabled
    } else {
        Availability::Enabled
    };
    let key = account.to_string();
    ListItem::row(
        key,
        label.clone(),
        rsx! {
            Row {
                title: label.clone(),
                detail: stays.then(|| TextLine::from(copy::tooltips::IN_NO_OTHER_SPACE)),
                size: RowSize::Settings,
                state: RowState { availability, ..RowState::default() },
                accessory: Accessory::Toggle {
                    value,
                    on_toggle: EventHandler::new(move |_| {
                        if !stays {
                            let account = account.clone();
                            handle.apply(space, |one| {
                                one.payload.accounts = toggled(&one.payload.accounts, account);
                            });
                        }
                    }),
                },
            }
        },
    )
}

/// Push `space`'s page over the list.
pub(super) fn push(mut shell: Signal<Shell>, space: SpaceId) {
    shell.write().spaces_pane = PanePath::new(SpacesPage::List).pushed(SpacesPage::Space(space));
}

/// Back one page, as the back button, Escape and ⌘[ ask: a name being typed is written first.
pub(super) fn back(mut shell: Signal<Shell>, handle: Handle) {
    handle.commit();
    let popped = shell.peek().spaces_pane.popped();
    shell.write().spaces_pane = popped;
}

/// Delete `space` and go back to the list. The first window forgets the Space's Today when it
/// reads the Spaces again (`app`).
fn delete(mut shell: Signal<Shell>, handle: Handle, space: SpaceId) {
    if handle.delete(space).is_ok() {
        shell.write().spaces_pane = PanePath::new(SpacesPage::List);
    }
}

#[cfg(test)]
#[path = "spaces_tests.rs"]
mod tests;

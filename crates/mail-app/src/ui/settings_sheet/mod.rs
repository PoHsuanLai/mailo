//! Settings: the window-wide choices, laid out as the desktop's Settings app lays out a program's
//! page (a sidebar of pages, each a column of titled groups of rows), and opened with Ctrl+, or
//! from the command menu.
//!
//! General draws mailo's settings schema (`crate::settings`) row by row ([`keys`]), the same
//! keys detent draws on mailo's page and writes to the same `mailo/settings.toml`; under them,
//! the sheets for contacts, rules, keys and the keyboard. Accounts lists each account, with its
//! own sheet (`account_settings`), which accounts keep all their mail here, and Add Account….
//! What belongs to one Space (its name, look and accounts) is the Space editor's.

mod accounts;
mod general;
mod keys;
mod offline;

pub(in crate::ui) use keys::with_value;

use crate::ui::common::in_card;
use crate::ui::press::SheetClose;
use crate::ui::view::{SettingsPage, Shell};
use dioxus::prelude::*;
use ds::components::chrome::sidebar::Sidebar;
use ds::components::chrome::sidebar_model::SidebarSection;
use ds::components::overlays::sheet_attach::Attach;
use ds::components::overlays::sheet_width::SheetWidth;
use ds::prelude::*;

/// What the sheet and its command are called.
pub(in crate::ui) const TITLE: &str = "Settings";

impl SettingsPage {
    const ALL: [SettingsPage; 2] = [SettingsPage::General, SettingsPage::Accounts];

    fn name(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Accounts => "Accounts",
        }
    }

    fn icon(self) -> Icon {
        match self {
            SettingsPage::General => Icon::Settings,
            SettingsPage::Accounts => Icon::Mail,
        }
    }
}

/// Open the sheet on General.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().settings = Some(SettingsPage::General);
}

/// Close the sheet and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().settings = None;
    crate::ui::host::Host::focus_app();
}

/// Whether another sheet opened from this one is in front of it: this one steps aside for it
/// and comes back when it closes.
fn covered(shell: &Shell) -> bool {
    shell.account_sheet.is_some()
        || shell.adding.is_some()
        || shell.contacts.is_some()
        || shell.rules.is_some()
        || shell.keys.is_some()
        || shell.keyboard.is_some()
}

/// The sheet. Mounted while `shell.settings` is `Some`.
#[component]
pub(in crate::ui) fn SettingsSheet(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let Some(page) = shell.read().settings else {
        return rsx! {};
    };
    if covered(&shell.read()) {
        return rsx! {};
    }
    let items: Vec<ListItem<SettingsPage>> = SettingsPage::ALL
        .into_iter()
        .map(|each| {
            let selection = if each == page {
                Selection::Selected
            } else {
                Selection::Unselected
            };
            ListItem::row(
                each,
                each.name(),
                rsx! {
                    Row {
                        title: TextLine::from(each.name()),
                        leading: RowLeading::Icon(each.icon()),
                        state: ds::base::vocab::RowState { selection, ..Default::default() },
                        common: ds::root::common::Common {
                            aria_label: Some(each.name().to_owned()),
                            ..Default::default()
                        },
                        onclick: move |_| shell.write().settings = Some(each),
                    }
                },
            )
        })
        .collect();
    rsx! {
        Sheet {
            label: TITLE,
            attach: Attach::Window,
            width: SheetWidth::Wide,
            common: in_card(),
            onclose: move |()| close(shell),
            // The scroller is the sheet's own child and the grid of the sidebar and the page, as
            // the Space editor's scroller is its own grid; Done stays under it.
            div { class: "settings-scroll settings",
                Sidebar::<SettingsPage> {
                    label: TITLE,
                    sections: vec![SidebarSection::List(items)],
                    cursor: Some(page),
                    onselect: move |next: SettingsPage| shell.write().settings = Some(next),
                }
                div { class: "settings-page", "data-page": page.name(),
                    match page {
                        SettingsPage::General => rsx! { general::General { shell } },
                        SettingsPage::Accounts => rsx! { accounts::Accounts { shell, revision } },
                    }
                }
            }
            div { class: "settings-foot",
                SheetClose { label: "Done", on_close: move |()| close(shell) }
            }
        }
    }
}

#[cfg(test)]
mod tests;

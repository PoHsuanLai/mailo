//! Settings: a window of its own, as the desktop's Settings app is and as a Mac app's settings
//! are, opened with ⌘, the gear in the sidebar's foot or the command menu, and raised if it is
//! already open. Laid out as detent lays out a program's page: a sidebar of pages, each a column
//! of titled groups of rows.
//!
//! General draws mailo's settings schema (`crate::settings`) row by row ([`keys`]), the same
//! keys detent draws on mailo's page and writes to the same `mailo/settings.toml`; under them,
//! the sheets for contacts, rules, keys and the keyboard. Accounts lists each account, with its
//! own sheet (`account_settings`), which accounts keep all their mail here, and Add Account….
//! What belongs to one Space (its name, look and accounts) is the Space editor's.
//!
//! The window is quire's (`ds_blitz::open_window`), with the root contexts every window of the
//! app is given. It keeps its own `Shell` for the sheets it opens, and tells the other windows
//! what it changed through the shared revision (`ui::revisions`): the main window reads the
//! settings, the keymap and the Spaces again when the revision moves, so a switch here is
//! followed there without a watch on the file. The call goes through [`SettingsWindows`] when
//! the window was given one, which is how a test sees what would open: quire's harness has no
//! event loop to open a window on.

mod accounts;
mod general;
mod keys;
mod offline;
mod root;

pub(in crate::ui) use keys::with_value;
pub use root::settings_root;

use crate::ui::view::{SettingsPage, Shell};
use dioxus::prelude::*;
use ds::components::chrome::sidebar::Sidebar;
use ds::components::chrome::sidebar_model::SidebarSection;
use ds::prelude::*;
use ds_blitz::{WindowHandle, WindowLife, WindowSpec};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// What the window and its command are called.
pub(in crate::ui) const TITLE: &str = "Settings";

/// The size the window opens at, in logical pixels.
const SIZE: (u32, u32) = (780, 620);

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

/// Whatever opens the Settings window: quire's event loop in the launched window, a recorder in
/// a test.
pub trait OpenSettings: Send + Sync + 'static {
    /// Open the Settings window, or raise it when it is open.
    fn open(&self);
}

/// Where ⌘, goes, as a root context. Without one it is quire's `open_window`.
#[derive(Clone)]
pub struct SettingsWindows(pub Arc<dyn OpenSettings>);

impl std::fmt::Debug for SettingsWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SettingsWindows")
    }
}

/// The Settings window this window opened, kept so that opening it again raises it.
#[derive(Clone, Default)]
pub(in crate::ui) struct SettingsOpened(Rc<RefCell<Option<WindowHandle>>>);

/// Keep the Settings window this one opens. Called once, from `App`.
pub(in crate::ui) fn use_settings_opened() {
    use_context_provider(SettingsOpened::default);
}

/// Open Settings, or raise it.
pub(in crate::ui) fn open() {
    match try_consume_context::<SettingsWindows>() {
        Some(windows) => windows.0.open(),
        None => quire(),
    }
}

/// Whether a window opened earlier is one to raise rather than open again.
fn raise(life: Option<WindowLife>) -> bool {
    matches!(life, Some(WindowLife::Opening | WindowLife::Open))
}

/// Ask quire's event loop for the window, or raise the one already open.
fn quire() {
    let opened = try_consume_context::<SettingsOpened>().unwrap_or_default();
    let existing = opened.0.borrow().clone();
    if let Some(handle) = existing.filter(|handle| raise(Some(handle.life()))) {
        handle.focus();
        return;
    }
    match ds_blitz::open_window(
        WindowSpec::new(TITLE, SIZE.0, SIZE.1),
        root::settings_window,
    ) {
        Ok(handle) => {
            *opened.0.borrow_mut() = Some(handle);
        }
        Err(why) => crate::ui::motion::tell(
            format!("Could not open Settings: {why}"),
            crate::ui::motion::Follow::Nothing,
        ),
    }
}

/// The window's content: the sidebar of pages beside the page shown, which scrolls.
#[component]
pub(in crate::ui) fn SettingsView(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let page = shell.read().settings.unwrap_or_default();
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
        div { class: "settings",
            Sidebar::<SettingsPage> {
                label: TITLE,
                sections: vec![SidebarSection::List(items)],
                cursor: Some(page),
                onselect: move |next: SettingsPage| shell.write().settings = Some(next),
            }
            div { class: "settings-scroll settings-page", "data-page": page.name(),
                match page {
                    SettingsPage::General => rsx! { general::General { shell } },
                    SettingsPage::Accounts => rsx! { accounts::Accounts { shell, revision } },
                }
            }
        }
    }
}

#[cfg(test)]
mod raise_tests {
    use super::raise;
    use ds_blitz::WindowLife;

    #[test]
    fn a_window_still_there_is_raised_and_a_closed_one_opened_again() {
        const CASES: &[(Option<WindowLife>, bool)] = &[
            (None, false),
            (Some(WindowLife::Opening), true),
            (Some(WindowLife::Open), true),
            (Some(WindowLife::Closed), false),
        ];
        for (life, raised) in CASES {
            assert_eq!(raise(*life), *raised, "{life:?}");
        }
    }
}

#[cfg(test)]
mod tests;

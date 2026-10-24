//! Settings: a window of its own, as the desktop's Settings app is and as a Mac app's settings
//! are, opened with ⌘, the gear in the sidebar's foot or the command menu, and raised if it is
//! already open. Laid out as detent lays out a program's page: a sidebar of pages, each a column
//! of titled groups of rows.
//!
//! Five pages: General, Accounts, Spaces, Rules and Keyboard. General draws mailo's settings schema
//! (`crate::settings`) as two groups of rows ([`general`], each row a key's, drawn by [`keys`]),
//! the same keys detent draws on mailo's page and writes to the same `mailo/settings.toml`.
//! Accounts lists each account, whose row pushes its own page into the pane ([`account`]): its
//! mail kept here, its encryption keys, and its removal. Rules and Keyboard are drawn by their
//! modules (`rules`, `keyboard`). A page that has pages of its own pushes them, as System
//! Settings does, with quire's `PaneStack`; a sheet is only for making something or asking
//! before something goes. Spaces lists each Space, whose row pushes its page: its name, colour,
//! theme and accounts, and its deletion ([`spaces`]); the Space's menu, a right click on the
//! Space's name, is the quick way to the same, and its Edit Space… opens the Space's page here
//! ([`open_space`]). A person is edited from their name in mail, and contacts
//! come in and go out through ⌘K, so Contacts is not a page here.
//!
//! Every way into one of those pages from the main window (⌘K, the composer's bar) opens the
//! window on that page, or turns the open window to it ([`open_at`]), and the composer's way to
//! an account's keys opens it on that account's page ([`open_account`]): where it is asked to be
//! is put in [`SettingsAsked`], which every window shares, and the Settings window follows it.
//!
//! The window is quire's (`ds_blitz::open_window`), with the root contexts every window of the
//! app is given. It keeps its own `Shell` for its pages and the sheets it opens, and tells the other windows
//! what it changed through the shared revision (`ui::revisions`): the main window reads the
//! settings, the keymap and the Spaces again when the revision moves, so a switch here is
//! followed there without a watch on the file. The call goes through [`SettingsWindows`] when
//! the window was given one, which is how a test sees what would open: quire's harness has no
//! event loop to open a window on.

mod account;
mod accounts;
mod general;
mod keys;
mod offline;
mod push_relay;
mod root;
mod rows;
mod spaces;

pub(in crate::ui) use account::Seams as AccountSeams;
pub(in crate::ui) use keys::with_value;
pub use root::settings_root;

use crate::ui::copy;
use crate::ui::space::SpaceId;
use crate::ui::view::{
    AccountStep, AccountsPage, AccountsPane, RulesAt, SettingsPage, Shell, SpacesPage, VacationAt,
};
use dioxus::prelude::*;
use ds::components::chrome::sidebar::Sidebar;
use ds::components::chrome::sidebar_model::SidebarSection;
use ds::prelude::*;
use ds_blitz::{WindowHandle, WindowSpec};
use porter_core::AccountId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::watch;

/// What the window and its command are called.
pub(in crate::ui) const TITLE: &str = copy::titles::SETTINGS;

/// The size the window opens at, in logical pixels.
const SIZE: (u32, u32) = (780, 620);

impl SettingsPage {
    pub(in crate::ui) const ALL: [SettingsPage; 6] = [
        SettingsPage::General,
        SettingsPage::Accounts,
        SettingsPage::Spaces,
        SettingsPage::Rules,
        SettingsPage::Vacation,
        SettingsPage::Keyboard,
    ];

    /// The page's name in the sidebar, which is also the row's accessible name.
    pub(in crate::ui) fn name(self) -> &'static str {
        match self {
            SettingsPage::General => copy::titles::GENERAL,
            SettingsPage::Accounts => copy::titles::ACCOUNTS,
            SettingsPage::Spaces => copy::titles::SPACES,
            SettingsPage::Rules => copy::titles::RULES,
            SettingsPage::Vacation => copy::titles::VACATION_REPLY,
            SettingsPage::Keyboard => copy::titles::KEYBOARD,
        }
    }

    fn icon(self) -> Icon {
        match self {
            SettingsPage::General => Icon::Settings,
            SettingsPage::Accounts => Icon::Mail,
            SettingsPage::Spaces => Icon::Grid,
            SettingsPage::Rules => Icon::FolderInput,
            SettingsPage::Vacation => Icon::Sun,
            SettingsPage::Keyboard => Icon::Keyboard,
        }
    }
}

/// Where the Settings window is asked to be: a page, one account's page pushed over Accounts, or
/// one Space's pushed over Spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAt {
    Page(SettingsPage),
    Account(AccountId),
    Space(SpaceId),
}

impl SettingsAt {
    /// The page of the sidebar it is on.
    pub(in crate::ui) fn page(&self) -> SettingsPage {
        match self {
            SettingsAt::Page(page) => *page,
            SettingsAt::Account(_) => SettingsPage::Accounts,
            SettingsAt::Space(_) => SettingsPage::Spaces,
        }
    }

    /// The Accounts pane as it stands there: the list, or the account's page over it.
    pub(in crate::ui) fn pane(&self) -> AccountsPane {
        match self {
            SettingsAt::Page(_) | SettingsAt::Space(_) => AccountsPane::default(),
            SettingsAt::Account(account) => AccountsPane {
                path: PanePath::new(AccountsPage::List)
                    .pushed(AccountsPage::Account(account.clone())),
                step: AccountStep::Showing,
            },
        }
    }
}

impl SettingsAt {
    /// The Spaces pane as it stands there: the list, or the Space's page over it.
    pub(in crate::ui) fn spaces_pane(&self) -> PanePath<SpacesPage> {
        let list = PanePath::new(SpacesPage::List);
        match self {
            SettingsAt::Space(space) => list.pushed(SpacesPage::Space(*space)),
            SettingsAt::Page(_) | SettingsAt::Account(_) => list,
        }
    }
}

impl Default for SettingsAt {
    fn default() -> Self {
        SettingsAt::Page(SettingsPage::default())
    }
}

/// Whatever opens the Settings window: quire's event loop in the launched window, a recorder in
/// a test.
pub trait OpenSettings: Send + Sync + 'static {
    /// Open the Settings window, or raise it when it is open, turned to `at` when it is asked
    /// for and where it is otherwise.
    fn open(&self, at: Option<SettingsAt>);
}

/// Where the Settings window is next, as a root context every window shares: the main window puts
/// where it asks for here, a Settings window opening starts there, and an open one turns to it.
/// Without one (a test's single window) the window opens on General.
#[derive(Clone)]
pub struct SettingsAsked(Arc<watch::Sender<SettingsAt>>);

impl Default for SettingsAsked {
    fn default() -> Self {
        SettingsAsked(Arc::new(watch::channel(SettingsAt::default()).0))
    }
}

impl std::fmt::Debug for SettingsAsked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SettingsAsked")
            .field(&*self.0.borrow())
            .finish()
    }
}

impl SettingsAsked {
    /// Where it was asked to be last.
    pub(in crate::ui) fn at(&self) -> SettingsAt {
        self.0.borrow().clone()
    }

    /// Ask for `at`. Sent even when it is where it was asked to be last: the window may have been
    /// turned elsewhere since.
    pub(in crate::ui) fn ask(&self, at: SettingsAt) {
        self.0.send_replace(at);
    }

    /// What a Settings window hears each ask through.
    pub(in crate::ui) fn heard(&self) -> watch::Receiver<SettingsAt> {
        self.0.subscribe()
    }
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

/// Open Settings, or raise it on the page it shows.
pub(in crate::ui) fn open() {
    ask(None);
}

/// Open Settings on `page`, or raise it and turn it to `page`.
pub(in crate::ui) fn open_at(page: SettingsPage) {
    ask(Some(SettingsAt::Page(page)));
}

/// Open Settings on `account`'s page, pushed over Accounts, or raise it and turn it there.
pub(in crate::ui) fn open_account(account: AccountId) {
    ask(Some(SettingsAt::Account(account)));
}

/// Open Settings on `space`'s page, pushed over Spaces, or raise it and turn it there: the
/// Space's menu's Edit Space….
pub(in crate::ui) fn open_space(space: SpaceId) {
    ask(Some(SettingsAt::Space(space)));
}

fn ask(at: Option<SettingsAt>) {
    match try_consume_context::<SettingsWindows>() {
        Some(windows) => windows.0.open(at),
        None => {
            if let (Some(at), Some(asked)) = (at, try_consume_context::<SettingsAsked>()) {
                asked.ask(at);
            }
            quire();
        }
    }
}

/// Show `page` in this Settings window, Accounts and Rules at their lists as System Settings shows
/// a pane chosen in its sidebar.
pub(in crate::ui) fn go(mut shell: Signal<Shell>, page: SettingsPage) {
    let mut write = shell.write();
    write.settings = Some(page);
    if write.accounts_pane.step != AccountStep::Removing {
        write.accounts_pane = AccountsPane::default();
    }
    write.rules.path = PanePath::new(RulesAt::List);
    write.spaces_pane = PanePath::new(SpacesPage::List);
    write.vacation_pane = PanePath::new(VacationAt::List);
}

/// Turn this Settings window to `at`: a page, an account's page over Accounts, or a Space's
/// over Spaces.
pub(in crate::ui) fn go_to(shell: Signal<Shell>, at: SettingsAt) {
    match at {
        SettingsAt::Page(page) => go(shell, page),
        SettingsAt::Account(account) => {
            go(shell, SettingsPage::Accounts);
            accounts::push(shell, account);
        }
        SettingsAt::Space(space) => {
            go(shell, SettingsPage::Spaces);
            spaces::push(shell, space);
        }
    }
}

/// Ask quire's event loop for the window, or raise the one already open.
fn quire() {
    let opened = try_consume_context::<SettingsOpened>().unwrap_or_default();
    let existing = opened.0.borrow().clone();
    if let Some(handle) = existing.filter(|handle| crate::ui::window::raise(Some(handle.life()))) {
        handle.focus();
        return;
    }
    match ds_blitz::open_window(
        WindowSpec::new(TITLE, ds_blitz::WindowSize::new(SIZE.0, SIZE.1)),
        root::settings_window,
    ) {
        Ok(handle) => {
            *opened.0.borrow_mut() = Some(handle);
        }
        Err(why) => {
            log::warn!("the Settings window would not open: {why}");
            crate::ui::motion::tell(
                crate::ui::copy::toasts::would_not_open(TITLE),
                crate::ui::motion::Follow::Nothing,
            );
        }
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
                        onclick: move |_| go(shell, each),
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
                onselect: move |next: SettingsPage| go(shell, next),
            }
            div { class: "settings-scroll settings-page", "data-page": page.name(),
                match page {
                    SettingsPage::General => rsx! { general::General {} },
                    SettingsPage::Accounts => rsx! { accounts::Accounts { shell, revision } },
                    SettingsPage::Spaces => match try_consume_context::<crate::ui::space::Handle>() {
                        Some(handle) => rsx! { spaces::Spaces { shell, handle } },
                        None => rsx! {},
                    },
                    SettingsPage::Rules => rsx! { crate::ui::rules::RulesPage { shell, revision } },
                    SettingsPage::Vacation => rsx! { crate::ui::rules::VacationPage { shell } },
                    SettingsPage::Keyboard => rsx! { crate::ui::keyboard::KeyboardPage { shell } },
                }
            }
        }
    }
}

#[cfg(test)]
pub(in crate::ui) mod tests;

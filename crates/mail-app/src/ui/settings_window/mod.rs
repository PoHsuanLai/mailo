//! Settings: a window of its own, as the desktop's Settings app is and as a Mac app's settings
//! are, opened with ⌘, the gear in the sidebar's foot or the command menu, and raised if it is
//! already open. Laid out as detent lays out a program's page: a sidebar of pages, each a column
//! of titled groups of rows.
//!
//! General draws mailo's settings schema (`crate::settings`) row by row ([`keys`]), the same
//! keys detent draws on mailo's page and writes to the same `mailo/settings.toml`. Accounts
//! lists each account, whose row pushes its own page into the pane ([`account`]), which
//! accounts keep all their mail here, and Add Account…. Contacts, Rules, Keys and certificates
//! and Keyboard are pages of their own, drawn by their modules (`contacts`, `rules`,
//! `pgp::keys`, `keyboard`) as rows in titled groups, as System Settings draws a pane; a sheet is
//! only for making something or asking before something goes. What belongs to one Space (its
//! name, look and accounts) is the Space's menu's, a right click on the Space.
//!
//! Every way into one of those pages from the main window (⌘K, the composer's bar) opens the
//! window on that page, or turns the open window to it ([`open_at`]), and the Connection Doctor's
//! gear opens it on an account's page ([`open_account`]): where it is asked to be is put in
//! [`SettingsAsked`], which every window shares, and the Settings window follows it.
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
mod root;

pub(in crate::ui) use keys::with_value;
pub use root::settings_root;

use crate::ui::view::{AccountStep, AccountsPage, AccountsPane, SettingsPage, Shell};
use dioxus::prelude::*;
use ds::components::chrome::sidebar::Sidebar;
use ds::components::chrome::sidebar_model::SidebarSection;
use ds::prelude::*;
use ds_blitz::{WindowHandle, WindowLife, WindowSpec};
use porter_core::AccountId;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::watch;

/// What the window and its command are called.
pub(in crate::ui) const TITLE: &str = "Settings";

/// The size the window opens at, in logical pixels.
const SIZE: (u32, u32) = (780, 620);

impl SettingsPage {
    pub(in crate::ui) const ALL: [SettingsPage; 6] = [
        SettingsPage::General,
        SettingsPage::Accounts,
        SettingsPage::Contacts,
        SettingsPage::Rules,
        SettingsPage::Keys,
        SettingsPage::Keyboard,
    ];

    /// The page's name in the sidebar, which is also the row's accessible name.
    pub(in crate::ui) fn name(self) -> &'static str {
        match self {
            SettingsPage::General => "General",
            SettingsPage::Accounts => "Accounts",
            SettingsPage::Contacts => "Contacts",
            SettingsPage::Rules => "Rules",
            SettingsPage::Keys => crate::ui::pgp::keys::TITLE,
            SettingsPage::Keyboard => "Keyboard",
        }
    }

    fn icon(self) -> Icon {
        match self {
            SettingsPage::General => Icon::Settings,
            SettingsPage::Accounts => Icon::Mail,
            SettingsPage::Contacts => Icon::Group,
            SettingsPage::Rules => Icon::FolderInput,
            SettingsPage::Keys => Icon::Key,
            SettingsPage::Keyboard => Icon::Keyboard,
        }
    }
}

/// Where the Settings window is asked to be: a page, or one account's page pushed over Accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAt {
    Page(SettingsPage),
    Account(AccountId),
}

impl SettingsAt {
    /// The page of the sidebar it is on.
    pub(in crate::ui) fn page(&self) -> SettingsPage {
        match self {
            SettingsAt::Page(page) => *page,
            SettingsAt::Account(_) => SettingsPage::Accounts,
        }
    }

    /// The Accounts pane as it stands there: the list, or the account's page over it.
    pub(in crate::ui) fn pane(&self) -> AccountsPane {
        match self {
            SettingsAt::Page(_) => AccountsPane::default(),
            SettingsAt::Account(account) => AccountsPane {
                path: PanePath::new(AccountsPage::List)
                    .pushed(AccountsPage::Account(account.clone())),
                step: AccountStep::Showing,
            },
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

/// Show `page` in this Settings window, Accounts at its list as System Settings shows a pane
/// chosen in its sidebar. A key the Keyboard page was waiting for is waited for no longer: the
/// page that took every key is gone.
pub(in crate::ui) fn go(mut shell: Signal<Shell>, page: SettingsPage) {
    let mut write = shell.write();
    write.settings = Some(page);
    write.keyboard.listening = None;
    if write.accounts_pane.step != AccountStep::Removing {
        write.accounts_pane = AccountsPane::default();
    }
}

/// Turn this Settings window to `at`: a page, or an account's page over Accounts.
pub(in crate::ui) fn go_to(shell: Signal<Shell>, at: SettingsAt) {
    match at {
        SettingsAt::Page(page) => go(shell, page),
        SettingsAt::Account(account) => {
            go(shell, SettingsPage::Accounts);
            accounts::push(shell, account);
        }
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
        WindowSpec::new(TITLE, ds_blitz::WindowSize::new(SIZE.0, SIZE.1)),
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
                    SettingsPage::Contacts => rsx! { crate::ui::contacts::ContactsPage { shell } },
                    SettingsPage::Rules => rsx! { crate::ui::rules::RulesPage { shell, revision } },
                    SettingsPage::Keys => rsx! { crate::ui::pgp::keys::KeysPage {} },
                    SettingsPage::Keyboard => rsx! { crate::ui::keyboard::KeyboardPage { shell } },
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
pub(in crate::ui) mod tests;

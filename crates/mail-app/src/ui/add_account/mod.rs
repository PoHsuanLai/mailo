//! Adding an account from the window: "Add account…" from the search bar, the "+" after the
//! account tiles, Settings, and the Connection Doctor's sign-in again.
//!
//! The add-account window is a window of its own, as Settings is: a separate top-level window,
//! raised when it is already open, that draws porter's accounts sheet with quire's
//! `ds-shell::accounts` parts. The sheet is porter's: its machine
//! (`porter_core::sheet`) says what to draw and asks a provider's sign-in what to do next, and
//! porter's account service runs them in this process (`host`). What is mailo's is the sign-in
//! itself, which looks an address up, signs in in a browser where the address wants one and adds
//! the account to mailo's store (`provider`), and the one mapping between the sheet's values and
//! quire's parts (`map`). `window` is the window, [`Ask`] and [`AddAccountWindows`] the way it is
//! opened.
//!
//! **Who draws** ([`Route`]). With the desktop's accountd linked (step E6, `mail_runtime::link`)
//! mailo asks it to run the sheet and sill draws it (`Accounts::add_account`), and the window is
//! not opened at all. Otherwise the window is the host, on every platform, as before.

mod host;
mod map;
mod provider;
mod size;
mod window;

pub use provider::{Request, Seams};
pub use window::{Browse, Fit, Opened, Wiring, add_account_root};

use std::cell::RefCell;
use std::sync::Arc;

use dioxus::prelude::*;
use ds_blitz::{WindowHandle, WindowSpec};
use mail_store::SqliteStore;

/// What the window and its command are called.
pub(in crate::ui) const TITLE: &str = "Add Account";

/// What the window opens with: the address of an account being signed in again, already typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ask {
    pub address: Option<String>,
}

/// Whatever opens the add-account window: quire's event loop in the launched window, a recorder
/// in a test.
pub trait OpenAddAccount: Send + Sync + 'static {
    /// Open the window for `ask`, or raise it when it is open.
    fn open(&self, ask: Ask);
}

/// Where "Add account…" goes, as a root context. Without one it is quire's `open_window_with`.
#[derive(Clone)]
pub struct AddAccountWindows(pub Arc<dyn OpenAddAccount>);

impl std::fmt::Debug for AddAccountWindows {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AddAccountWindows")
    }
}

/// Who draws the add-account sheet.
#[derive(Debug, Clone)]
enum Route {
    /// This process hosts porter's account service and draws the sheet in a window of its own.
    OwnWindow,
    /// The desktop's accountd runs the sheet and its shell draws it (`Accounts::add_account`, and
    /// `Accounts::reauthenticate` for an account being signed in again): mailo opens no window.
    Accountd(Arc<dyn mail_runtime::Accountd>),
}

/// Who draws the sheet, given the link this process chose (`mail_runtime::link`).
///
/// With accountd linked every sheet is accountd's, a new account or one signed in again: accountd
/// holds every account, and mailo opens no window and keeps no sign-in of its own. Without
/// accountd it is the window's.
fn route_of(link: &mail_runtime::Link) -> Route {
    match link.accountd() {
        Some(accountd) => Route::Accountd(accountd.clone()),
        None => Route::OwnWindow,
    }
}

/// Who draws the sheet now, from the link chosen at start.
fn route() -> Route {
    route_of(&mail_runtime::link::current())
}

/// Open the add-account window, or raise it. Call it from an event handler.
pub(in crate::ui) fn open() {
    request(Ask::default());
}

/// Open the window with `address` already typed: signing an account in again. Call it from an
/// event handler.
pub(in crate::ui) fn open_for(address: String) {
    request(Ask {
        address: Some(address),
    });
}

/// Let Mail use an account of the desktop's again, when the grant on it was withdrawn: accountd's
/// chooser and consent sheet, not a sign-in. Without accountd there are no grants, and this is
/// Add Account. Call it from an event handler.
pub(in crate::ui) fn allow_again() {
    if try_consume_context::<AddAccountWindows>().is_none()
        && let Route::Accountd(accountd) = route()
    {
        let store = consume_context::<Arc<SqliteStore>>();
        through_accountd(accountd, store, |store, accountd| {
            crate::accountd::allow(store, accountd)
        });
        return;
    }
    request(Ask::default());
}

fn request(ask: Ask) {
    match try_consume_context::<AddAccountWindows>() {
        Some(windows) => windows.0.open(ask),
        None => {
            let store = consume_context::<Arc<SqliteStore>>();
            match route() {
                Route::OwnWindow => quire(ask),
                Route::Accountd(accountd) => {
                    through_accountd(accountd, store, move |store, accountd| {
                        crate::accountd::add(store, accountd, ask.address.as_deref())
                    })
                }
            }
        }
    }
}

/// Run accountd's sheet (`sheet`: an add, a sign-in again, an allow again) on a thread of its
/// own, and tell every window when it ends.
///
/// The sheet is the desktop's (sill draws it, over everything), and the call waits for the person,
/// so it is not made on the window's thread. What the person added is read into the store, and the
/// shared revision moves, so every window draws it. A sheet closed is nothing to say; a failure is
/// said on stderr and in the notice of the window that asked, which hears it back on its own
/// thread.
fn through_accountd(
    accountd: Arc<dyn mail_runtime::Accountd>,
    store: Arc<SqliteStore>,
    sheet: impl FnOnce(
        &SqliteStore,
        &Arc<dyn mail_runtime::Accountd>,
    ) -> Result<crate::accountd::Added, String>
    + Send
    + 'static,
) {
    let revisions = try_consume_context::<crate::ui::revisions::Revisions>();
    let motion = crate::ui::motion::motion();
    let (failed, heard) = tokio::sync::oneshot::channel::<String>();
    let spawned = std::thread::Builder::new()
        .name("mailo-add-account".to_owned())
        .spawn(move || {
            match sheet(&store, &accountd) {
                Ok(crate::accountd::Added::Account(read)) => {
                    if let Some(said) = crate::accountd::said(&read) {
                        eprintln!("mailo: {said}");
                    }
                }
                Ok(crate::accountd::Added::Nothing) => {}
                Err(why) => {
                    eprintln!("mailo: Add Account: {why}");
                    let _ = failed.send(format!("Could not add the account: {why}"));
                }
            }
            if let Some(revisions) = revisions {
                revisions.bump();
            }
        });
    match spawned {
        // Nothing comes back when it worked or the sheet was closed: the sender is dropped.
        // At the root: the button that asked may be gone (a Settings window closed) by the time
        // the person is done with the sheet.
        Ok(_) => {
            dioxus::core::spawn_forever(async move {
                if let Ok(text) = heard.await {
                    crate::ui::motion::tell_through(motion, text);
                }
            });
        }
        Err(why) => crate::ui::motion::tell(
            format!("Could not open the Add Account sheet: {why}"),
            crate::ui::motion::Follow::Nothing,
        ),
    }
}

thread_local! {
    /// The window this app has open, kept so that opening it again raises it. Windows share the
    /// event loop's thread, so every window finds the same one.
    static OPEN: RefCell<Option<WindowHandle>> = const { RefCell::new(None) };
}

/// Ask quire's event loop for the window, or raise the one already open.
fn quire(ask: Ask) {
    let existing = OPEN.with(|open| open.borrow().clone());
    if let Some(handle) = existing.filter(|handle| crate::ui::window::raise(Some(handle.life()))) {
        handle.focus();
        return;
    }
    let store = consume_context::<Arc<SqliteStore>>();
    let props = window::AddAccountWindowProps {
        wiring: window::Wiring::real(store),
        prefill: ask.address,
    };
    // It opens at the size of the list, its first step, fitted to the screen (`size`).
    let screen = try_consume_context::<ds_blitz::AppHandle>().and_then(|app| app.screen_extent());
    let spec = WindowSpec::new(TITLE, size::opening(screen));
    match ds_blitz::open_window_with(spec, window::AddAccountWindow, props) {
        Ok(handle) => OPEN.with(|open| *open.borrow_mut() = Some(handle)),
        Err(why) => crate::ui::motion::tell(
            format!("Could not open the Add Account window: {why}"),
            crate::ui::motion::Follow::Nothing,
        ),
    }
}

#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod provider_tests;
#[cfg(test)]
mod route_tests;
#[cfg(test)]
mod size_tests;

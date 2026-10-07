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
//! **Who draws.** Today the window is the only host, on every platform ([`Route`]). On Linux with
//! the desktop's accountd (step E6) mailo asks it to run the sheet and sill draws it
//! (`Accounts::add_account`), and the window is not opened at all: that is one more [`Route`],
//! chosen in [`route`], and nothing else here changes.

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
use ds_blitz::{WindowHandle, WindowLife, WindowSpec};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    /// This process hosts porter's account service and draws the sheet in a window of its own.
    OwnWindow,
}

/// Who draws the sheet now: the one choice E6 adds a case to (`Accounts::connect` finding
/// accountd, whose sheet sill draws).
fn route() -> Route {
    Route::OwnWindow
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

fn request(ask: Ask) {
    match try_consume_context::<AddAccountWindows>() {
        Some(windows) => windows.0.open(ask),
        None => match route() {
            Route::OwnWindow => quire(ask),
        },
    }
}

thread_local! {
    /// The window this app has open, kept so that opening it again raises it. Windows share the
    /// event loop's thread, so every window finds the same one.
    static OPEN: RefCell<Option<WindowHandle>> = const { RefCell::new(None) };
}

/// Whether a window opened earlier is one to raise rather than open again.
fn raise(life: Option<WindowLife>) -> bool {
    matches!(life, Some(WindowLife::Opening | WindowLife::Open))
}

/// Ask quire's event loop for the window, or raise the one already open.
fn quire(ask: Ask) {
    let existing = OPEN.with(|open| open.borrow().clone());
    if let Some(handle) = existing.filter(|handle| raise(Some(handle.life()))) {
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
mod size_tests;

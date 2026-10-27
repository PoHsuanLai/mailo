//! Adding an account from the window: "Add account…" from the search bar, the Space's menu ›
//! Accounts, Settings, and the dialog's sign-in again.
//!
//! The add-account window is a window of its own, as Settings is: a separate top-level window,
//! raised when it is already open, that draws porter's accounts sheet with quire's
//! `ds-shell::accounts` parts. The sheet is porter's: its machine
//! (`porter_core::sheet`) says what to draw and asks a provider's sign-in what to do next, and
//! porter's account service runs them in this process (`host`). What is mailo's is the sign-in
//! itself, which looks an address up, signs in in a browser where the address wants one and adds
//! the account to mailo's client (`provider`), and the one mapping between the sheet's values and
//! quire's parts (`map`). `window` is the window, [`Ask`] and [`AddAccountWindows`] the way it is
//! opened.
//!
//! **Straight to the browser** (`direct`). A provider whose accounts sign in with an OAuth redirect
//! needs no window: the Space menu's Add Account and Settings' Add Account… are a menu that
//! names each such provider and signs in with it in the browser, as a sign-in button on the web
//! does, and the dialog signs such an account in again the same way. Other Account…,
//! and every provider that needs a form, is the window.
//!
//! The window is the host on every platform: mailo keeps its own accounts and credentials. Signing
//! in through the desktop's accountd (a "Continue as" offer) is planned, not here yet.

mod direct;
mod host;
mod map;
mod provider;
mod size;
mod window;

pub(in crate::ui) use direct::{
    AddMenu, AddPick, DirectSignInWatch, Retry, add_from, add_menu, again, offered_redirects,
    picked, retry,
};
pub use direct::{DirectSignIns, SignInDirectly};
pub use provider::{AddError, AddFault, Request, Seams};
pub use window::{Browse, Opened, Wiring, add_account_root};

use std::cell::RefCell;
use std::sync::Arc;

use dioxus::prelude::*;
use ds_blitz::{WindowHandle, WindowSpec};

use crate::ui::space::SpaceId;

/// What the window and its command are called.
pub(in crate::ui) const TITLE: &str = "Add Account";

/// What the window opens with: the address of an account being signed in again, already typed,
/// and which Space an account added there joins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ask {
    pub address: Option<String>,
    pub join: Join,
}

/// Which Space an account added joins. Accounts and Spaces are apart: an account is in a Space
/// only because someone put it there, so one added from a Space (its menu, its sidebar, the
/// window with nothing in it) joins that Space, and one added from Settings › Accounts, or signed
/// in again, joins none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Join {
    /// The Space on screen when the account is added.
    #[default]
    OnScreen,
    /// This Space, whose menu it was added from.
    Into(SpaceId),
    /// No Space.
    Nowhere,
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

/// Open the add-account window, or raise it. Call it from an event handler.
pub(in crate::ui) fn open() {
    request(Ask::default());
}

/// Open the window with `address` already typed: signing an account in again. Call it from an
/// event handler.
pub(in crate::ui) fn open_for(address: String) {
    request(Ask {
        address: Some(address),
        join: Join::Nowhere,
    });
}

/// What a failure that asks to allow Mail again opens: Add Account, the one way an account comes
/// back. Call it from an event handler.
pub(in crate::ui) fn allow_again() {
    request(Ask::default());
}

fn request(ask: Ask) {
    match try_consume_context::<AddAccountWindows>() {
        Some(windows) => windows.0.open(ask),
        None => quire(ask),
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
    // Reached only in the launched program: a test's window hands `AddAccountWindows`. The OAuth
    // clients are the ones `main` gave the edge, as every other sign-in's are.
    let client = crate::ui::live::client();
    let props = window::AddAccountWindowProps {
        wiring: window::Wiring::real(client, crate::edge::clients()),
        prefill: ask.address,
        join: ask.join,
    };
    // It opens at the size of the list, its first step, fitted to the screen (`size`).
    let screen = try_consume_context::<ds_blitz::AppHandle>().and_then(|app| app.screen_extent());
    let spec = WindowSpec::new(TITLE, size::opening(screen));
    match ds_blitz::open_window_with(spec, window::AddAccountWindow, props) {
        Ok(handle) => OPEN.with(|open| *open.borrow_mut() = Some(handle)),
        Err(why) => {
            log::warn!("the Add Account window would not open: {why}");
            crate::ui::motion::tell(
                crate::ui::copy::toasts::would_not_open(TITLE),
                crate::ui::motion::Follow::Nothing,
            );
        }
    }
}

#[cfg(test)]
mod direct_tests;
#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod provider_tests;
#[cfg(test)]
mod size_tests;

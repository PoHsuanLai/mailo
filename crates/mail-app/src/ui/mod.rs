//! The Dioxus shell.
//!
//! Thin on purpose: every decision lives in [`crate::ui::view`], which is tested without a window.
//! What is here is layout, event wiring, and the one thing a UI can get dangerously wrong —
//! rendering a stranger's HTML.

pub mod appearance;
pub mod bin;
pub mod copy;
pub mod emoji;
pub mod handoff;
pub mod keymap;
pub mod saved;
pub mod selection;
pub mod space;
pub mod spelling;
pub mod tidy;
pub mod today;
pub mod view;

mod actions;
mod add_account;
mod app;
mod brand;
mod checks;
mod clock;
mod command;
mod common;
mod compose;
pub(crate) use compose::discard_draft;
mod contacts;
mod data;
mod debounce;
pub mod default_mail;
mod destroy;
mod downloads;
mod erase;
mod fetching;
mod files;
mod folder_open;
mod follow_up;
mod frame;
mod history;
mod host;
mod hover;
mod intent;
mod invite;
mod keyboard;
mod launch;
mod launcher_count;
mod list;
mod list_query;
mod list_search;
mod marked;
mod menu;
mod menus;
mod motion;
mod move_to;
mod no_account;
mod ops;
mod original;
mod page;
mod pgp;
mod pick;
mod picks;
mod prefs;
mod press;
mod print;
mod provider_chip;
pub mod reading;
mod reauth;
mod receipt;
mod remedy;
mod revisions;
mod row;
mod rules;
mod server_search;
mod settings_window;
mod sidebar;
mod space_menu;
mod start;
mod style;
mod switch;
mod text;
mod toolbar;
mod unsubscribe;
mod view_groups;
mod views;
mod window;
mod window_core;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod shell_tests;
#[cfg(test)]
mod tips_tests;

pub(crate) use fetching::on_write;
pub use launch::run;

/// The window on Blitz, for a test to drive through `ds_harness::Harness`.
pub mod native {
    pub use super::add_account::{
        AddAccountWindows, AddError, AddFault, Ask as AddAccountAsk, Browse as AddAccountBrowse,
        DirectSignIns, OpenAddAccount, Opened as AddAccountOpened, Request as AddRequest,
        Seams as AddAccountSeams, SignInDirectly, Wiring as AddAccountWiring, add_account_root,
    };
    pub use super::brand::BrandCache;
    pub use super::clock::WallClock;
    pub use super::compose::Dictionaries;
    pub use super::downloads::{NotOpened as FileNotOpened, Opener as FileOpener};
    pub use super::files::SaveDir;
    pub use super::follow_up::Notices;
    pub use super::launch::DesktopSettings;
    pub use super::launch::gate::{Found, found as damaged_files};
    pub use super::launch::native::{APP_ID, contexts, live_root, root};
    pub use super::original::{Browse, Consent, Fetch, FetchImage, Got, Original};
    pub use super::pick::{Ask as DialogAsk, Dialogs};
    pub use super::print::Printer;
    pub use super::provider_chip::read as read_icons;
    pub use super::reading::OriginalFrame;
    pub use super::revisions::{Configured, Revisions};
    pub use super::server_search::{Search, ServerSearcher};
    pub use super::settings_window::{
        OpenSettings, SettingsAsked, SettingsAt, SettingsWindows, settings_root,
    };
    pub use super::window::{Ask, MessageOpen, OpenWindow, Windows, message_root};
    pub use crate::handoff::{ActivationToken, Request, Requests};
}

pub use start::{Start, mailto_of, open_thread, start_mailto, start_of};

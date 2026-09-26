//! Opening the window.
//!
//! The window reads six values as root contexts: the store, the look, the Spaces, the provider
//! icons, where it opens, and the directories it writes (when there are any). `native.rs` hands
//! them over to quire's Blitz window.

use super::app::App;
use crate::appearance::WindowDirs;
use crate::provider::icon::Loaded;
use crate::space::Spaces;
use crate::view::Appearance;
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

pub(super) mod native;

/// What the window is opened with.
pub(super) struct Opening {
    pub store: Arc<SqliteStore>,
    pub look: Appearance,
    pub spaces: Spaces,
    pub dirs: Option<WindowDirs>,
    pub start: super::Start,
    pub icons: Loaded,
}

/// Launch the shell, already wearing `look` and `spaces`, open where `start` says.
pub fn run(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Spaces,
    dirs: Option<WindowDirs>,
    start: super::Start,
) {
    let icons = crate::appearance::cache_dir()
        .map(|dir| Loaded::read(&dir.join("providers")))
        .unwrap_or_default();
    let opening = Opening {
        store,
        look,
        spaces,
        dirs,
        start,
        icons,
    };
    native::run(opening);
}

/// The launched window's root: the window's settings, then [`Shell`].
///
/// The settings are quire's: `appearance.toml` and the desktop's preferences, both watched, as
/// one signal `App`'s root reads (`ds_settings::use_environment`). `main` imported
/// `appearance.json` into the TOML file before the window opened. Only the launched window
/// watches; a test renders `App` (or [`Shell`]) without this and never touches the real config
/// directory.
#[component]
fn ShellRoot() -> Element {
    let environment = ds_settings::use_environment(ds_settings::AppName::MAILO);
    use_context_provider(|| environment);
    rsx! { Shell {} }
}

/// Holds the icon cache in a signal so a refresh can replace it, and names the window's host.
///
/// The files were read once, before the first frame. The signal is what a later refresh
/// writes; the chips subscribe to it. The host is what the window's focus, scroll and clipboard
/// asks go to (`ui/host`): Blitz's.
#[component]
fn Shell() -> Element {
    let loaded = try_consume_context::<Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    super::host::use_window_host();
    rsx! { App {} }
}

//! Opening the window: what both frontends share, and each one's own launch.
//!
//! The window reads six values as root contexts: the store, the look, the Spaces, the provider
//! icons, where it opens, and the directories it writes (when there are any). Both frontends hand
//! over the same six; only how they are handed over differs (`webview.rs`, `native.rs`).

use super::app::App;
use crate::appearance::WindowDirs;
use crate::provider::icon::Loaded;
use crate::space::Spaces;
use crate::view::Appearance;
use dioxus::prelude::*;
use mail_store::SqliteStore;
use std::sync::Arc;

#[cfg(feature = "native")]
pub(super) mod native;
#[cfg(feature = "webview")]
pub(super) mod webview;

/// What the window is opened with, whichever frontend opens it.
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
    #[cfg(feature = "webview")]
    webview::run(opening);
    #[cfg(feature = "native")]
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
/// asks go to (`ui/host`): on `native`, Blitz's; on the webview, none is provided, and an ask is
/// the page's script as it always was.
#[component]
fn Shell() -> Element {
    let loaded = try_consume_context::<Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    super::host::use_window_host();
    rsx! { App {} }
}

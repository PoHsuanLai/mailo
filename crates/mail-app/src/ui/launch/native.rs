//! The `native` frontend: quire's `ds-native`, Blitz drawn with wgpu.
//!
//! The six values the window reads reach it as root contexts through
//! `AppConfig::with_context`, the same call a test makes through `HarnessConfig`
//! ([`contexts`]). Nothing is passed through a global.
//!
//! What the webview's head carried does not exist here: `ds-native` registers the fonts itself,
//! holds and places the focus (`ds_native::focus`), and runs no script, so there is no keep-focus
//! script, no "nothing mounted" note and no debug probe.

use super::{Opening, Shell, ShellRoot};
use crate::appearance::WindowDirs;
use crate::space::Spaces;
use crate::view::Appearance;
use dioxus::prelude::*;
use ds_native::{AppConfig, AppId, NetPolicy, RootContexts};
use mail_store::SqliteStore;
use std::sync::Arc;

/// The desktop entry's name (`packaging/mailo.desktop`), which notifications name too: the
/// Wayland `app_id` and X11 `WM_CLASS`, so the desktop matches the window to its entry.
pub(in crate::ui) const APP_ID: &str = "mailo";

/// Open the window on Blitz.
///
/// Network: [`NetPolicy::Local`], `data:` and nothing else reaching a sub-document. The reader's
/// remote images therefore stay blocked on this frontend whatever the consent says, until mailo
/// has a handler that honours it.
pub(super) fn run(opening: Opening) {
    let Opening {
        store,
        look,
        spaces,
        dirs,
        start,
        icons,
    } = opening;
    let config = AppConfig::new("mailo", 1200, 800)
        .with_app_id(AppId(APP_ID.to_owned()))
        .with_net(NetPolicy::Local)
        .with_contexts(contexts(store, look, spaces, dirs, start))
        .with_context(icons);
    ds_native::launch(ShellRoot, config);
}

/// The root contexts the window reads, for `AppConfig` or `HarnessConfig::with_contexts`: the
/// store, the look, the Spaces, where the window opens, and the directories it writes, when there
/// are any. Without directories the window keeps its choices in memory and writes no file.
///
/// The provider icons are the one value left out: a test has no cache to read them from, and
/// the window then draws each provider's letter.
pub fn contexts(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Spaces,
    dirs: Option<WindowDirs>,
    start: crate::ui::Start,
) -> RootContexts {
    let contexts = RootContexts::new()
        .with(store)
        .with(look)
        .with(spaces)
        .with(start);
    match dirs {
        Some(dirs) => contexts.with(dirs),
        None => contexts,
    }
}

/// The window as a test drives it: everything the launched window draws, but not its watch on
/// the settings directory, so a test never reads or watches the real `~/.config`. Hand it to
/// `ds_native::Harness` with [`contexts`].
pub fn root() -> Element {
    rsx! { Shell {} }
}

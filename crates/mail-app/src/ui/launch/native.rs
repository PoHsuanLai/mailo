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
use crate::ui::original::Original;
use crate::view::Appearance;
use dioxus::prelude::*;
use ds_native::{AppConfig, AppId, RootContexts};
use mail_store::SqliteStore;
use std::sync::Arc;

/// The desktop entry's name (`packaging/mailo.desktop`), which notifications name too: the
/// Wayland `app_id` and X11 `WM_CLASS`, so the desktop matches the window to its entry.
pub(in crate::ui) const APP_ID: &str = "mailo";

/// Open the window on Blitz.
///
/// The reader's Original frames are sealed documents whose network and links are mailo's
/// ([`Original::window`], `ui/original`): a frame gets inline `data:`, and a remote image only
/// when the reader has consented to that thread's images and the frame is that message's. The
/// window's own document keeps its `file:` and `data:` and is refused everything else, so the
/// Reader view's consented images are fetched by mailo on "Show images" and drawn as `data:`
/// (`reading/remote.rs`). A link clicked in a frame opens in the browser.
pub(super) fn run(opening: Opening) {
    let Opening {
        store,
        look,
        spaces,
        dirs,
        start,
        icons,
    } = opening;
    let original = Original::window();
    let config = AppConfig::new("mailo", 1200, 800)
        .with_app_id(AppId(APP_ID.to_owned()))
        .with_net(original.net())
        .with_frame_links(original.links())
        .with_contexts(contexts(store, look, spaces, dirs, start))
        .with_context(original.consent())
        .with_context(original.pill())
        .with_context(original.images())
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
/// `ds_native::Harness` with [`contexts`], and with an [`Original`]'s `harness` for frames held
/// as the window holds them.
pub fn root() -> Element {
    rsx! { Shell {} }
}

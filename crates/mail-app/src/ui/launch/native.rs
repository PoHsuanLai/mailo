//! The window: quire's `ds-blitz`, Blitz drawn with wgpu. The only frontend.
//!
//! The seven values the window reads reach it as root contexts through
//! `AppConfig::with_context`, the same call a test makes through `HarnessConfig`
//! ([`contexts`]). Nothing is passed through a global.
//!
//! `ds-blitz` registers the fonts itself, holds and places the focus (`ds_blitz::focus`), and
//! runs no script, so there is no keep-focus script, no "nothing mounted" note and no debug probe.

use super::{Opening, Shell, ShellRoot};
use crate::ui::appearance::WindowDirs;
use crate::ui::original::Original;
use crate::ui::space::Spaces;
use crate::ui::view::Appearance;
use dioxus::prelude::*;
use ds_blitz::{AppConfig, AppId, RootContexts};
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
        brand,
    } = opening;
    // Read once, as the window opens, like the watch reads it.
    let notices = dirs
        .as_ref()
        .filter(|dirs| {
            crate::ui::prefs::root_of(dirs)
                .map(|root| crate::settings::load(&root).notifications.new_mail)
                == Some(crate::settings::NewMail::On)
        })
        .map(|_| {
            crate::ui::follow_up::Notices(std::sync::Arc::new(
                mail_core::notify::desktop::Desktop::connect(),
            ))
        });
    let original = Original::window();
    // One revision for every window, so a conversation open in a window of its own follows what
    // the main window does to it, and the other way round (`ui/revisions`). It is also what the
    // follower of accountd's changes moves: an account added or removed there is every window's
    // to draw.
    let revisions = crate::ui::revisions::Revisions::new();
    let following = revisions.clone();
    crate::accountd::follow(&store, &mail_runtime::link::current(), move |_| {
        following.bump();
    });
    let config = AppConfig::new("mailo", ds_blitz::WindowSize::new(1200, 800))
        .with_app_id(AppId(APP_ID.to_owned()))
        .with_net(original.net())
        .with_frame_links(original.links())
        .with_contexts(contexts(store, look, spaces, dirs, start))
        .with_context(original.consent())
        .with_context(original.pill())
        .with_context(original.images())
        .with_context(icons)
        .with_context(revisions)
        // And one for the configuration files a window writes (key bindings, Spaces).
        .with_context(crate::ui::revisions::Configured::default())
        // And the page the Settings window shows next, which ⌘K and the composer turn it to.
        .with_context(crate::ui::settings_window::SettingsAsked::default());
    let config = match brand {
        Some(brand) => config.with_context(brand),
        None => config,
    };
    // The unread count on the dock or the Dash. Only the launched window has one; a test hands
    // its own recorder or none. With a `mailo watch` running (the session's sync daemon) the
    // watch is the one voice: it counts every account, and it keeps the count when this window
    // closes, which a dock would otherwise forget along with this window's connection.
    let config = match crate::ui::launcher::platform() {
        Some(launcher) if !mail_core::ipc::watching::running() => config.with_context(launcher),
        _ => config,
    };
    // Another program's ask to open a conversation here (a banner's click, `mailo open`).
    let config = match crate::ui::handoff::serve() {
        Some(requests) => config.with_context(requests),
        None => config,
    };
    // A reminder that comes back is said on the desktop too, while notifications are on: the
    // setting `mailo watch` reads. A test's window has none (`contexts`), so no test raises one.
    let config = match notices {
        Some(notices) => config.with_context(notices),
        None => config,
    };
    ds_blitz::launch(ShellRoot, config);
}

/// The root contexts the window reads, for `AppConfig` or `HarnessConfig::with_contexts`: the
/// store, the look, the Spaces, where the window opens, and the directories it writes, when there
/// are any. Without directories the window keeps its choices in memory and writes no file.
///
/// The provider icons and the brand logo cache are left out: a test has no cache to read them
/// from, and the window then draws each provider's letter and each sender's initial. A test that
/// wants logos adds a `BrandCache` of its own.
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

/// The launched window's root, with its watch on the desktop's settings, for a test that changes
/// them: hand it a [`super::DesktopSettings`] naming a scratch directory and fixed preferences
/// beside [`contexts`], so it never reads or watches the real `~/.config` or the session bus.
pub fn live_root() -> Element {
    rsx! { super::ShellRoot {} }
}

/// The window as a test drives it: everything the launched window draws, but not its watch on
/// the settings directory, so a test never reads or watches the real `~/.config`. Hand it to
/// `ds_blitz::Harness` with [`contexts`], and with an [`Original`]'s `harness` for frames held
/// as the window holds them.
pub fn root() -> Element {
    rsx! { Shell {} }
}

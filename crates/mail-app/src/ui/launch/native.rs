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
use mail_core::SqliteStore;
use std::sync::Arc;

/// The desktop entry's name (`packaging/org.quire.Mail.desktop`), which notifications name too:
/// the Wayland `app_id` and X11 `WM_CLASS`, so the desktop matches the window to its entry. It is
/// the intents app ([`crate::intents::APP`]) too, so the scope a launcher starts the window in names
/// the app the desktop's intent router knows.
pub const APP_ID: &str = crate::intents::APP;

/// The main window's title, which the system's window lists show.
const TITLE: &str = "mailo";

/// The main window's size as it opens, the first time and each time it is opened again.
fn size() -> ds_blitz::WindowSize {
    ds_blitz::WindowSize::new(1200, 800)
}

/// Open the window on Blitz.
///
/// The reader's Original frames are sealed documents whose network and links are mailo's
/// ([`Original::window`], `ui/original`): a frame gets inline `data:`, and a remote image only
/// when the reader has consented to that thread's images and the frame is that message's. The
/// window's own document keeps its `file:` and `data:` and is refused everything else, so the
/// Reader view's consented images are fetched by mailo on "Show images" and drawn as `data:`
/// (`reading/remote.rs`). A link clicked in a frame opens in the browser.
pub(super) fn run(opening: Opening) -> Result<(), ds_blitz::LaunchError> {
    let Opening {
        store,
        look,
        spaces,
        dirs,
        start,
        icons,
        brand,
        found,
    } = opening;
    // What earlier runs' Open left in the temporary folder, cleared off the thread that draws.
    std::thread::spawn(crate::ui::reading::sweep_left_open);
    // Where the settings are, for what the quit removes.
    let quit_root = dirs
        .as_ref()
        .and_then(crate::ui::prefs::root_of)
        .or_else(crate::settings::person_root);
    // The desktop launcher's copy of the recent conversations and the address book, kept while
    // the window runs, on threads of its own (`intents::index`).
    #[cfg(not(any(target_os = "macos", windows)))]
    crate::intents::index::start(store.clone());
    // Whether they are on is asked as each is said (`follow_up::speaking`), so Settings' switch
    // takes effect without a restart.
    let notices = dirs.as_ref().map(|dirs| {
        let interest = crate::ui::prefs::root_of(dirs)
            .map(|root| crate::settings::load(&root).notifications.interest())
            .unwrap_or_default();
        crate::ui::follow_up::Notices(std::sync::Arc::new(
            mail_core::notify::desktop::Desktop::connect_in_window().with_interest(interest),
        ))
    });
    // And the login item's switch, read as the window opens.
    let while_quit = dirs
        .as_ref()
        .and_then(crate::ui::prefs::root_of)
        .map(|root| crate::settings::load(&root).sync.while_quit);
    let original = Original::window();
    // How the window reaches the world for a sign-in, for keys and for a removal's keyring: the
    // real ones, here and only here. A test's root (`contexts`) has none of them, and a window
    // without one reaches nothing (`add_account::DirectSignIns`, `pgp::seams`, Settings'
    // account page): no browser, no issuer, no keyring, no dialog.
    let sign_ins =
        crate::ui::add_account::DirectSignIns::real(store.clone(), crate::edge::clients());
    let keys = crate::ui::pgp::Seams::real();
    let removals = crate::ui::settings_window::AccountSeams::real();
    // One revision for every window, so a conversation open in a window of its own follows what
    // the main window does to it, and the other way round (`ui/revisions`).
    let revisions = crate::ui::revisions::Revisions::new();
    // The keys the person chose, as the windows open: quire lays them over the system's keymap
    // for every window, so they are the keymap's own chords. A change made in Settings reaches
    // the running windows through `Keys::set_overrides` (`frame::use_followed_configuration`).
    let chosen = dirs
        .as_ref()
        .map(|dirs| crate::ui::keymap::load(&dirs.config).overrides());
    // On macOS the app goes on with no window open, checking for mail, until it is quit, and is
    // the one that does whether a window is open or not (`crate::background`): its scheduler
    // starts here, before the window, so the window finds it.
    let stays = crate::background::stays_open(crate::background::Os::HERE);
    if stays {
        crate::background::start(store.clone(), crate::edge::profile().id().clone());
    }
    // How the app opens its main window again, from a thread that is not the window's.
    let app = ds_blitz::AppHandle::new();
    let config = AppConfig::new(TITLE, size())
        .with_last_window(crate::background::last_window(crate::background::Os::HERE))
        .with_handle(app.clone())
        .with_app_id(AppId(APP_ID.to_owned()))
        .with_net(original.net())
        .with_frame_links(original.links())
        // ⌘+, ⌘− and ⌘0 are the reader's text size (`reading.text_zoom`), as in Mail, kept in
        // the settings: blitz-shell would otherwise zoom the whole window on them before the
        // window hears them, and forget it at the next launch. Its Alt+D, Alt+H and Alt+T
        // overlays go with them, which leaves those keys to the window too.
        .with_shell_keys(ds_blitz::ShellKeys::App)
        .with_contexts(window_contexts(
            store.clone(),
            look,
            Some(spaces),
            dirs,
            start,
        ))
        .with_context(original.consent())
        .with_context(original.pill())
        .with_context(original.menus())
        .with_context(original.clicked_images())
        .with_context(original.images())
        .with_context(sign_ins)
        .with_context(keys)
        .with_context(removals)
        .with_context(icons)
        .with_context(found)
        .with_context(revisions)
        // And one for the configuration files a window writes (key bindings, Spaces).
        .with_context(crate::ui::revisions::Configured::default())
        // And the page the Settings window shows next, which ⌘K and the composer turn it to.
        .with_context(crate::ui::settings_window::SettingsAsked::default())
        // And who needs to sign in again, which only the first window knows and Settings shows.
        .with_context(crate::ui::fetching::Needing::default());
    let config = match chosen {
        Some(chosen) => config.with_keymap_overrides(chosen),
        None => config,
    };
    let config = match brand {
        Some(brand) => config.with_context(brand),
        None => config,
    };
    // The unread count on the dock or the Dash. Only the launched window has one; a test hands
    // its own recorder or none. With a `mailo watch` running (the session's sync daemon) the
    // watch is the one voice: it counts every account, and it keeps the count when this window
    // closes, which a dock would otherwise forget along with this window's connection. On macOS
    // the Dock tile is the app's, and the app is the one voice, on a thread of its own, whether
    // a window is open or not and whoever watches: no other process can badge it.
    let config = match crate::launcher::platform() {
        Some(launcher) if stays => {
            let kept = crate::session::keep_the_badge(
                store.clone(),
                store.file().map(Into::into),
                launcher.0,
                crate::session::EVERY,
            );
            if let Err(e) = kept {
                eprintln!("the Dock's unread count is off: {e}");
            }
            config
        }
        Some(launcher) if !mail_core::ipc::watching::running(crate::edge::profile().id()) => {
            config.with_context(launcher)
        }
        _ => config,
    };
    // "Check for mail when quit": where the system keeps `mailo watch` to start at login. Only
    // the launched app has one; a test's window has none, so no test registers a login item.
    // Handed the switch as it stands once now, so an item made by a mailo since moved names the
    // program where it is.
    let config = match crate::login_item::platform() {
        Some(items) => {
            if while_quit == Some(crate::settings::WhileQuit::On) {
                let kept = items.clone();
                // Off the thread that draws: `launchctl` and SMAppService take their time.
                let started = std::thread::Builder::new()
                    .name("mailo-login-item".to_owned())
                    .spawn(move || {
                        if let Err(why) =
                            crate::login_item::apply(&*kept.0, crate::settings::WhileQuit::On)
                        {
                            log::warn!("mail is not checked at login: {why}");
                        }
                    });
                if let Err(why) = started {
                    log::warn!("mail is not checked at login: {why}");
                }
            }
            config.with_context(items)
        }
        None => config,
    };
    // Which application opens email links, and the system's question to make it mailo. Only an
    // installed mailo.app has one; a test's window has none, so no test changes the real handler.
    let config = match crate::mail_handler::platform() {
        Some(handlers) => config.with_context(handlers),
        None => config,
    };
    // Another program's ask to open a conversation here (a banner's click, `mailo open`). On
    // macOS one that comes with no main window open opens one again, as the Dock icon does.
    let config = match crate::handoff::serve(crate::edge::profile().id()) {
        Some(requests) => {
            #[cfg(target_os = "macos")]
            requests.open_with(move || {
                app.open_window(
                    ds_blitz::WindowSpec::new(TITLE, size()),
                    super::ReopenedRoot,
                )
                .is_ok()
            });
            config.with_context(requests)
        }
        None => config,
    };
    // A reminder that comes back is said on the desktop too, while notifications are on: the
    // setting `mailo watch` reads. A test's window has none (`contexts`), so no test raises one.
    let config = match notices {
        Some(notices) => config.with_context(notices),
        None => config,
    };
    let ran = ds_blitz::launch(ShellRoot, config);
    // The last window closed: the copies Open wrote go now, unedited, when the setting says so,
    // as it says by then.
    if let Some(root) = quit_root {
        crate::ui::downloads::unedited::remove_on_quit(
            crate::settings::load(&root).attachments.remove_unedited,
        );
    }
    ran
}

/// The root contexts the window reads, for `AppConfig` or `HarnessConfig::with_contexts`: the
/// store, the look, the Spaces, where the window opens, and the directories it writes, when there
/// are any. Without directories the window keeps its choices in memory and writes no file; `None`
/// for the Spaces is a first run over the store's accounts, as a window with nothing stored has.
///
/// The provider icons and the brand logo cache are left out: a test has no cache to read them
/// from, and the window then draws each provider's letter and each sender's initial. A test that
/// wants logos adds a `BrandCache` of its own.
///
/// A window built from these never syncs: each account's pass ends at once having fetched
/// nothing, so a test's window reaches no server and no keyring, and an account of its fixture
/// is never found wanting a sign-in it was never going to get. The launched window passes for
/// real.
pub fn contexts(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Option<Spaces>,
    dirs: Option<WindowDirs>,
    start: crate::ui::Start,
) -> RootContexts {
    window_contexts(store, look, spaces, dirs, start).with(crate::ui::fetching::Passer::idle())
}

/// [`contexts`] as the launched window has them: its passes go to the servers.
fn window_contexts(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Option<Spaces>,
    dirs: Option<WindowDirs>,
    start: crate::ui::Start,
) -> RootContexts {
    let looks = crate::ui::reading::looks();
    let mut contexts = RootContexts::new()
        .with(store)
        .with(look)
        .with(start)
        // The one cache of rendered bodies every reader and the warming share, and the one set
        // of per-message lookups they and the opened bodies come from.
        .with(crate::ui::reading::frames(looks.clone()))
        .with(looks);
    if let Some(spaces) = spaces {
        contexts = contexts.with(spaces);
    }
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

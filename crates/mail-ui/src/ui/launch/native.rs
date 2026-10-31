//! The window: quire's `ds-blitz`, Blitz drawn with wgpu. The only frontend.
//!
//! The seven values the window reads reach it as root contexts through
//! `AppConfig::with_context`, the same call a test makes through `HarnessConfig`
//! ([`contexts`]). Nothing is passed through a global.
//!
//! `ds-blitz` registers the fonts itself, holds and places the focus (`ds_blitz::focus`), and
//! runs no script, so there is no keep-focus script, no "nothing mounted" note and no debug probe.

use super::{Opening, Shell, ShellRoot};
use dioxus::prelude::*;
use ds_blitz::{AppConfig, AppId, RootContexts};
use mail_ui_kit::appearance::WindowDirs;
use mail_ui_kit::original::Original;
use mail_ui_kit::space::Spaces;
use mail_ui_kit::view::Appearance;

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
        program,
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
        .and_then(mail_ui_kit::prefs::root_of)
        .or_else(mail_ui_model::settings::person_root);
    // Whether they are on is asked as each is said (`follow_up::speaking`), so Settings' switch
    // takes effect without a restart.
    // Which mailboxes, which sound and which accounts are muted are read as well, for the same
    // reason (`settings::Notices`).
    let notices = dirs.as_ref().map(|dirs| {
        let desktop = mail_os::desktop::connect_in_window();
        crate::ui::follow_up::Notices(std::sync::Arc::new(mail_ui_model::settings::Notices::new(
            Box::new(move |notification, sound| desktop.show_with(notification, sound)),
            mail_ui_kit::prefs::root_of(dirs),
        )))
    });
    // And the login item's switch, read as the window opens.
    let while_quit = dirs
        .as_ref()
        .and_then(mail_ui_kit::prefs::root_of)
        .map(|root| mail_ui_model::settings::load(&root).sync.while_quit);
    // The one client every window reads mail through, reaching the servers as the program does.
    let super::Program {
        client,
        profile,
        resident,
    } = program;
    let original = Original::window(client.net().runtime());
    // How the window reaches the world for a sign-in, for keys and for a removal's keyring: the
    // real ones, here and only here. A test's root (`contexts`) has none of them, and a window
    // without one reaches nothing (`add_account::DirectSignIns`, `pgp::seams`, Settings'
    // account page): no browser, no issuer, no keyring, no dialog.
    let sign_ins =
        crate::ui::add_account::DirectSignIns::real(client.clone(), client.net().oauth_clients());
    let keys = crate::ui::pgp::Seams::real(&client);
    let removals = crate::ui::settings_window::AccountSeams::real(&client);
    // The keys the person chose, as the windows open: quire lays them over the system's keymap
    // for every window, so they are the keymap's own chords. A change made in Settings reaches
    // the running windows through `Keys::set_overrides` (`frame::use_followed_configuration`).
    let chosen = dirs
        .as_ref()
        .map(|dirs| mail_ui_kit::keymap::load(&dirs.config).overrides());
    // How the app opens its main window again, from a thread that is not the window's.
    let app = ds_blitz::AppHandle::new();
    let stays = resident.stays;
    let config = AppConfig::new(TITLE, size())
        .with_last_window(resident.last_window())
        .with_handle(app.clone())
        .with_app_id(AppId(crate::APP_ID.to_owned()))
        .with_net(original.net())
        .with_frame_links(original.links())
        // ⌘+, ⌘− and ⌘0 are the reader's text size (`reading.text_zoom`), as in Mail, kept in
        // the settings: blitz-shell would otherwise zoom the whole window on them before the
        // window hears them, and forget it at the next launch. Its Alt+D, Alt+H and Alt+T
        // overlays go with them, which leaves those keys to the window too.
        .with_shell_keys(ds_blitz::ShellKeys::App)
        .with_contexts(window_contexts(
            client.clone(),
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
        // The count of the configuration files a window writes (key bindings, Spaces).
        .with_context(mail_ui_kit::configured::Configured::default())
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
    let config = match mail_os::launcher::platform() {
        Some(launcher) if stays => {
            if let Some(keep) = &resident.keep_badge {
                keep(launcher.0);
            }
            config
        }
        Some(launcher) if !mail_client::ipc::watching::running(&profile) => {
            config.with_context(launcher)
        }
        _ => config,
    };
    // "Check for mail when quit": where the system keeps `mailo watch` to start at login. Only
    // the launched app has one; a test's window has none, so no test registers a login item.
    // Handed the switch as it stands once now, so an item made by a mailo since moved names the
    // program where it is.
    let config = match mail_os::login_item::platform() {
        Some(items) => {
            if while_quit == Some(mail_ui_model::settings::WhileQuit::On) {
                let kept = items.clone();
                // Off the thread that draws: `launchctl` and SMAppService take their time.
                let started = std::thread::Builder::new()
                    .name("mailo-login-item".to_owned())
                    .spawn(move || {
                        if let Err(why) = mail_os::login_item::apply(&*kept.0, true) {
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
    let config = match mail_os::mail_handler::platform() {
        Some(handlers) => config.with_context(handlers),
        None => config,
    };
    // Another program's ask to open a conversation here (a banner's click, `mailo open`). On
    // macOS one that comes with no main window open opens one again, as the Dock icon does.
    let config = match crate::handoff::serve(&profile) {
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
    clean_up_at_quit(quit_root.as_ref());
    ran
}

/// What the quit removes: the copies Open wrote, unedited, when the setting under `root` says so.
/// No settings to ask, nothing removed.
fn clean_up_at_quit(root: Option<&ds_settings::ConfigRoot>) {
    if let Some(root) = root {
        crate::ui::downloads::unedited::remove_on_quit(
            mail_ui_model::settings::load(root)
                .attachments
                .remove_unedited,
        );
    }
}

#[cfg(test)]
mod quit_tests {
    use super::clean_up_at_quit;
    use crate::ui::downloads::unedited;
    use mail_ui_model::settings::RemoveUnedited;

    /// The quit asks the settings under its root, and removes the recorded copies only under When
    /// Mailo Quits; with no settings to ask it removes nothing.
    #[test]
    fn the_quit_removes_unedited_copies_only_when_the_setting_says_so() {
        let _alone = unedited::RUN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let scratch = tempfile::tempdir().expect("a scratch folder");
        let root = scratch.path().join("mailo-open-test");
        std::fs::create_dir_all(&root).expect("Open's folder");
        let config = scratch.path().join("config");
        std::fs::create_dir_all(&config).expect("the config folder");
        let settings = mail_ui_model::settings::root_for(&config);
        let copy = root.join("plans.txt");
        std::fs::write(&copy, b"plans").expect("a copy");
        unedited::record(&root, &copy, mail_domain::MessageId::generate());

        clean_up_at_quit(None);
        assert!(copy.exists(), "removed with no settings to ask");
        for setting in [RemoveUnedited::Never, RemoveUnedited::AfterMessageIsDeleted] {
            mail_ui_model::settings::change(&settings, |all| {
                all.attachments.remove_unedited = setting
            })
            .expect("the setting is written");
            clean_up_at_quit(Some(&settings));
            assert!(copy.exists(), "removed at quit under {setting:?}");
        }
        mail_ui_model::settings::change(&settings, |all| {
            all.attachments.remove_unedited = RemoveUnedited::WhenMailoQuits;
        })
        .expect("the setting is written");
        clean_up_at_quit(Some(&settings));
        assert!(!copy.exists(), "kept at quit under When Mailo Quits");
    }
}

/// The root contexts the window reads, for `AppConfig` or `HarnessConfig::with_contexts`: the
/// client, the look, the Spaces, where the window opens, and the directories it writes, when there
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
    client: mail_client::Client,
    look: Appearance,
    spaces: Option<Spaces>,
    dirs: Option<WindowDirs>,
    start: crate::ui::ask::Open,
) -> RootContexts {
    contexts_passing(
        client,
        look,
        spaces,
        dirs,
        start,
        crate::ui::fetching::Passer::idle().0,
    )
}

/// [`contexts`] with `pass` run for each account's pass in place of the idle one: a test that
/// needs the first fetch to be in a state it can hold (still running, say) lends a pass that
/// stays there, where the idle one ends at once and the window moves on between two looks.
pub fn contexts_passing(
    client: mail_client::Client,
    look: Appearance,
    spaces: Option<Spaces>,
    dirs: Option<WindowDirs>,
    start: crate::ui::ask::Open,
    pass: mail_client::fetch::Pass,
) -> RootContexts {
    window_contexts(client, look, spaces, dirs, start).with(crate::ui::fetching::Passer(pass))
}

/// [`contexts`] as the launched window has them: its passes go to the servers.
fn window_contexts(
    client: mail_client::Client,
    look: Appearance,
    spaces: Option<Spaces>,
    dirs: Option<WindowDirs>,
    start: crate::ui::ask::Open,
) -> RootContexts {
    let looks = crate::ui::reading::looks();
    // The accounts and saved views read before the first window asks, so its first frame has its
    // places: kept for as long as the windows are, by their root.
    let primed = Primed {
        _accounts: client.live(mail_client::query::Accounts),
        _views: client.live(mail_client::query::Views),
    };
    // Waited for, here and now, for a moment at most: a window opened before they are read would
    // draw its first frame without its places. A read that fails or is slow is not waited on
    // further; the window shows what it has and the places arrive with the next frame.
    primed._accounts.wait_for(PRIMED_WITHIN);
    primed._views.wait_for(PRIMED_WITHIN);
    let mut contexts = RootContexts::new()
        .with(client)
        .with(primed)
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

/// How long the launch waits for the first reads of the accounts and the saved views.
const PRIMED_WITHIN: std::time::Duration = std::time::Duration::from_secs(2);

/// Values the client reads before a window asks for them, held so they stay read.
#[derive(Clone, Debug)]
struct Primed {
    _accounts: mail_client::Live<Vec<mail_client::query::Account>>,
    _views: mail_client::Live<Vec<mail_domain::View>>,
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

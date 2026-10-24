//! Opening the window.
//!
//! The window reads seven values as root contexts: the store, the look, the Spaces, the provider
//! icons, where it opens, the directories it writes (when there are any), and where brand logos
//! are cached (when there is a cache directory). `native.rs` hands them over to quire's Blitz
//! window.

use super::app::App;
use crate::ui::appearance::WindowDirs;
use crate::ui::space::Spaces;
use crate::ui::view::Appearance;
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds::style::kit::{StyleHomes, Styles, UserStyle};
use ds_settings::{AppName, ConfigRoot, Store, SystemPrefsSource, use_environment};
use mail_core::SqliteStore;
use mail_core::provider::icon::Loaded;
use std::sync::Arc;

pub(super) mod gate;
pub(super) mod native;

/// What the window is opened with.
pub(super) struct Opening {
    pub store: Arc<SqliteStore>,
    pub look: Appearance,
    pub spaces: Spaces,
    pub dirs: Option<WindowDirs>,
    pub start: super::Start,
    pub icons: Loaded,
    /// Where brand logos are cached, when there is a cache directory.
    pub brand: Option<super::brand::BrandCache>,
    /// The settings files that are there and cannot be read, asked about before the window opens.
    pub found: gate::Found,
}

/// Launch the shell, already wearing `look` and `spaces`, open where `start` says. Returns when
/// the app ends: when the window closes, except on macOS, where mailo.app goes on until it is
/// quit (`crate::background`); or at once when it could not open (no runtime, no event loop).
pub fn run(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Spaces,
    dirs: Option<WindowDirs>,
    start: super::Start,
) -> Result<(), ds_blitz::LaunchError> {
    let cache = mail_core::config::cache_dir();
    let icons = cache
        .as_ref()
        .map(|dir| super::provider_chip::read(&dir.join("providers")))
        .unwrap_or_default();
    let brand = cache.map(|dir| super::brand::BrandCache(dir.join("bimi")));
    let found = gate::Found(dirs.as_ref().map(gate::found).unwrap_or_default());
    let opening = Opening {
        store,
        look,
        spaces,
        dirs,
        start,
        icons,
        brand,
        found,
    };
    native::run(opening)
}

/// Where the launched window reads the desktop's appearance from: the person's own
/// (`ConfigRoot::Xdg`, the settings portal), or, in a test, a scratch directory and fixed
/// preferences. A root context; a window without one reads the person's.
#[derive(Debug, Clone)]
pub struct DesktopSettings {
    pub root: ConfigRoot,
    pub prefs: SystemPrefsSource,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        DesktopSettings {
            root: ConfigRoot::Xdg,
            prefs: SystemPrefsSource::Portal,
        }
    }
}

impl DesktopSettings {
    /// The settings in force for this window: its context, else the person's.
    pub(in crate::ui) fn current() -> Self {
        try_consume_context::<DesktopSettings>().unwrap_or_default()
    }

    /// The desktop's one store, `quire/` in the config root, which the shell and detent read
    /// and write too. mailo keeps no appearance file of its own.
    pub(in crate::ui) fn store(&self) -> Store {
        Store::new(self.root.clone(), AppName::QUIRE)
    }
}

/// The launched window's root: the window's settings, then [`Shell`].
///
/// The settings are the desktop's, quire's (`quire/appearance.toml`, the same store the shell
/// and detent use) and the desktop's preferences, both watched, as one signal `App`'s root
/// reads (`ds_settings::use_environment`), so a theme, accent or motion change restyles this
/// window while it runs; the person's own `style.css`, watched the same way and drawn after
/// quire's and mailo's sheets unless "Use my styles" is off (CONSUMING.md section 12); and mailo's own settings
/// (`crate::settings`), watched so a change made in the desktop's Settings app reaches the window. `main` migrated mailo's old appearance
/// file into the desktop's before the window opened (`appearance::adopt`). Only the launched
/// window watches; a test renders `App` (or [`Shell`]) without this and never touches the real
/// config directory.
#[component]
pub(super) fn ShellRoot() -> Element {
    let settings = DesktopSettings::current();
    let store = settings.store();
    let spawner: Arc<dyn Spawner> = Arc::new(ds_blitz::TokioSpawner::current());
    let environment = use_environment(store.clone(), settings.prefs.clone(), Arc::clone(&spawner));
    use_context_provider(|| environment);
    // The person's stylesheet, the desktop's `quire/style.css`, allowed to name files in its own
    // folder. mailo reads no `style.css` of its own: `appearance::adopt` brought the one it had
    // into the desktop's folder, where it is now, and drawing the old copy too would apply it
    // twice.
    let mut styles = use_signal({
        let store = store.clone();
        move || Styles {
            desktop: store.load::<UserStyle>().value,
            homes: StyleHomes {
                desktop: store.dir(),
                ..StyleHomes::default()
            },
            ..Styles::default()
        }
    });
    use_future(move || {
        let store = store.clone();
        let spawner = Arc::clone(&spawner);
        async move {
            let mut watch = store.watch::<UserStyle>(&*spawner);
            while let Some(loaded) = watch.changed().await {
                styles.write().desktop = loaded.value;
            }
        }
    });
    use_context_provider(|| ReadSignal::new(styles));
    // mailo's own settings (`mailo/settings.toml`), watched the same way: detent writes the same
    // file, and the window follows it.
    super::prefs::use_window_settings();
    rsx! { gate::Gate {} }
}

/// The main window opened again, after the last one closed while mailo.app ran on (the Dock icon
/// clicked, a banner's conversation, a `mailto:` link: `crate::background`). [`ShellRoot`], but
/// at the inbox, whatever the launch was opened on, and past the damaged settings' question, which
/// the first window asked: what it carries in from outside comes through the hand-off's queue.
#[component]
pub(super) fn ReopenedRoot() -> Element {
    use_context_provider(|| super::Start::Inbox);
    use_context_provider(gate::Found::default);
    rsx! { ShellRoot {} }
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
    crate::ui::provider_chip::use_fetch_missing(icons);
    super::host::use_window_host();
    rsx! { App {} }
}

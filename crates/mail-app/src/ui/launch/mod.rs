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
use ds_settings::{AppName, ConfigRoot, Store, SystemPrefsSource, UserStyle, use_environment};
use mail_core::provider::icon::Loaded;
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
    /// Where brand logos are cached, when there is a cache directory.
    pub brand: Option<super::brand::BrandCache>,
}

/// Launch the shell, already wearing `look` and `spaces`, open where `start` says.
pub fn run(
    store: Arc<SqliteStore>,
    look: Appearance,
    spaces: Spaces,
    dirs: Option<WindowDirs>,
    start: super::Start,
) {
    let cache = mail_core::config::cache_dir();
    let icons = cache
        .as_ref()
        .map(|dir| Loaded::read(&dir.join("providers")))
        .unwrap_or_default();
    let brand = cache.map(|dir| super::brand::BrandCache(dir.join("bimi")));
    let opening = Opening {
        store,
        look,
        spaces,
        dirs,
        start,
        icons,
        brand,
    };
    native::run(opening);
}

/// The launched window's root: the window's settings, then [`Shell`].
///
/// The settings are quire's: `appearance.toml` and the desktop's preferences, both watched, as
/// one signal `App`'s root reads (`ds_settings::use_environment`), and the person's own
/// `style.css`, watched the same way and drawn after quire's and mailo's sheets (CONSUMING.md
/// section 12). `main` imported `appearance.json` into the TOML file before the window opened.
/// Only the launched window watches; a test renders `App` (or [`Shell`]) without this and never
/// touches the real config directory.
#[component]
fn ShellRoot() -> Element {
    let store = Store::new(ConfigRoot::Xdg, AppName::MAILO);
    let spawner: Arc<dyn Spawner> = Arc::new(ds_blitz::TokioSpawner::current());
    let environment = use_environment(
        store.clone(),
        SystemPrefsSource::Portal,
        Arc::clone(&spawner),
    );
    use_context_provider(|| environment);
    let mut user_style = use_signal({
        let store = store.clone();
        move || store.load::<UserStyle>().value
    });
    use_future(move || {
        let store = store.clone();
        let spawner = Arc::clone(&spawner);
        async move {
            let mut watch = store.watch::<UserStyle>(&*spawner);
            while let Some(loaded) = watch.changed().await {
                user_style.set(loaded.value);
            }
        }
    });
    use_context_provider(|| ReadSignal::new(user_style));
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

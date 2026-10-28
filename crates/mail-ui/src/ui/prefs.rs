//! The window's half of `crate::settings`: `mailo/settings.toml` in one signal, which everything
//! that follows a setting reads, and the one way the window changes it.
//!
//! The launched window loads it and watches the file (`launch::ShellRoot`), so a change made in
//! the desktop's Settings app reaches the open window. A window drawn without that (a test)
//! loads it from the config directory it was handed, or keeps the defaults in memory when it was
//! handed none.

use crate::settings::{self, MailSettings};
use crate::ui::appearance::WindowDirs;
use dioxus::prelude::*;
use ds_settings::ConfigRoot;

/// Where the window's settings are written: `None` keeps them in memory only.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct PrefsRoot(pub Option<ConfigRoot>);

/// The config root the window's config directory stands for (`settings::root_for`).
pub(in crate::ui) fn root_of(dirs: &WindowDirs) -> Option<ConfigRoot> {
    Some(settings::root_for(&dirs.config))
}

/// Provide the settings to everything under the caller: the launched window's, when its root
/// already provided them, else loaded from `dirs`.
pub(in crate::ui) fn use_prefs(dirs: Option<&WindowDirs>) -> Signal<MailSettings> {
    let given = try_consume_context::<Signal<MailSettings>>();
    let root =
        try_consume_context::<PrefsRoot>().unwrap_or_else(|| PrefsRoot(dirs.and_then(root_of)));
    let loaded = use_hook({
        let root = root.clone();
        move || {
            given.unwrap_or_else(|| {
                Signal::new(root.0.as_ref().map(settings::load).unwrap_or_default())
            })
        }
    });
    use_context_provider(|| root);
    use_context_provider(|| loaded)
}

/// The config root a window keeps its settings under: its config directory's
/// (`settings::root_for`), else the person's (`settings::person_root`), so every window, the
/// command line and `watch` read one `settings.toml`. `None` where there is neither.
fn window_root() -> Option<ConfigRoot> {
    try_consume_context::<WindowDirs>()
        .as_ref()
        .and_then(root_of)
        .or_else(settings::person_root)
}

/// What every window's root does first: load the settings, provide them, and follow
/// `settings.toml` as it changes, so a change made in another window, the desktop's Settings app
/// or `mailo notify` reaches it. A window with nowhere to keep them holds the defaults.
pub(in crate::ui) fn use_window_settings() {
    let root = use_hook(window_root);
    let mut values = use_signal({
        let root = root.clone();
        move || root.as_ref().map(settings::load).unwrap_or_default()
    });
    use_context_provider({
        let root = root.clone();
        move || PrefsRoot(root)
    });
    use_context_provider(|| values);
    // The window's images are fetched the way `privacy.*` says, and follow it as it changes.
    // The relay's key comes from the keyring, not the settings file, and is read again when the
    // file says it changed (`privacy.relay_key_changed`). Off the thread that draws: a keyring
    // answers when it likes.
    // `None` until the keyring has answered, then the key or its absence.
    let mut relay_key = use_signal(|| None::<Option<String>>);
    let _reading = use_resource(move || {
        let _changed = values.read().privacy.relay_key_changed;
        async move {
            let Some(client) = crate::ui::live::try_client() else {
                return;
            };
            let read = tokio::task::spawn_blocking(move || client.net().relay_key())
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
            relay_key.set(Some(read));
        }
    });
    // Why images cannot be fetched the way `privacy.*` says, if they cannot: what the Privacy page
    // and the reader tell the person ([`route_problem`]). Not a missing key before the keyring
    // has answered.
    let problem = use_memo(move || {
        let answered: Option<Option<String>> = relay_key();
        let known = answered.is_some();
        values
            .read()
            .privacy
            .route(answered.as_ref().and_then(|key| key.as_deref()))
            .err()
            .filter(|problem| known || *problem != crate::settings::RouteProblem::RelayKey)
    });
    use_context_provider(|| problem);
    // A `settings.toml` whose privacy settings cannot be read is the defaults, which would fetch
    // straight whatever was chosen: then nothing is fetched (`settings::privacy_in_doubt`).
    let watched = root.clone();
    use_effect(move || {
        let privacy = values.read().privacy.clone();
        let answered: Option<Option<String>> = relay_key();
        let doubt = watched.as_ref().is_some_and(settings::privacy_in_doubt);
        crate::ui::original::follow_privacy(
            (!doubt).then_some(&privacy),
            answered.as_ref().and_then(|key| key.as_deref()),
        );
    });
    use_future(move || {
        let root = root.clone();
        let spawner: std::sync::Arc<dyn ds::base::spawner::Spawner> =
            std::sync::Arc::new(ds_blitz::TokioSpawner::current());
        async move {
            let Some(root) = root else { return };
            let mut watch = settings::store(root).watch::<MailSettings>(&*spawner);
            while let Some(loaded) = watch.changed().await {
                if *values.peek() != loaded.value {
                    values.set(loaded.value);
                }
            }
        }
    });
}

/// Why images cannot be fetched the way `privacy.*` says, if they cannot: the relay's or the
/// proxy's address is not one to use, or the relay has no key. `None` while they can, and where
/// no window provided it.
pub(in crate::ui) fn route_problem() -> Option<settings::RouteProblem> {
    try_consume_context::<Memo<Option<settings::RouteProblem>>>().and_then(|problem| problem())
}

/// The settings in force, as a signal to read in a render or a resource: the window root's.
/// A component drawn under no window root (a unit test's lone component) reads them from its
/// config directory once, so it never shows defaults over what the file says.
pub(in crate::ui) fn use_settings() -> Signal<MailSettings> {
    use_hook(|| {
        try_consume_context::<Signal<MailSettings>>().unwrap_or_else(|| {
            Signal::new(
                window_root()
                    .as_ref()
                    .map(settings::load)
                    .unwrap_or_default(),
            )
        })
    })
}

/// The settings in force, or the defaults where nothing provided them.
pub(in crate::ui) fn current() -> MailSettings {
    try_consume_context::<Signal<MailSettings>>()
        .map(|values| values.read().clone())
        .unwrap_or_default()
}

/// Read `settings.toml` again into the window's settings, when another window said it wrote it
/// (`configured::Configured`). Unchanged settings are left alone, so nothing that reads them runs
/// again.
pub(in crate::ui) fn reread() {
    let (Some(mut values), Some(PrefsRoot(Some(root)))) = (
        try_consume_context::<Signal<MailSettings>>(),
        try_consume_context::<PrefsRoot>(),
    ) else {
        return;
    };
    let loaded = settings::load(&root);
    if *values.peek() != loaded {
        values.set(loaded);
    }
}

/// Change the settings with `edit`: written to `settings.toml` where the window has somewhere
/// to write it, and shown at once either way. A write that fails changes nothing on screen.
pub(in crate::ui) fn change(edit: impl FnOnce(&mut MailSettings)) -> Result<(), String> {
    let Some(mut values) = try_consume_context::<Signal<MailSettings>>() else {
        return Err(crate::ui::copy::errors::NO_SETTINGS_TO_CHANGE.to_owned());
    };
    let next = match try_consume_context::<PrefsRoot>().and_then(|root| root.0) {
        Some(root) => {
            let next = settings::change(&root, edit)?;
            // The other windows read it again now, rather than when their watch next settles
            // (and where a watch cannot run at all): the configuration counter, not the store's,
            // so no window's mail queries run again for a setting.
            crate::ui::configured::told_configuration();
            next
        }
        None => {
            let mut next = values.peek().clone();
            edit(&mut next);
            next
        }
    };
    values.set(next);
    Ok(())
}

/// Which request of a series is the newest: a write that lands after a newer one was asked for
/// must not show its (older) settings over the newer.
#[derive(Debug, Default)]
pub(in crate::ui) struct Latest(std::sync::atomic::AtomicU64);

impl Latest {
    /// No request yet.
    pub(in crate::ui) const fn new() -> Self {
        Self(std::sync::atomic::AtomicU64::new(0))
    }

    /// A request has come; its number.
    pub(in crate::ui) fn press(&self) -> u64 {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1
    }

    /// Whether no request has come since request number `n`.
    pub(in crate::ui) fn is_last(&self, n: u64) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) == n
    }
}

/// The requests [`change_later`] has been given, in the order they were asked for.
static REQUESTS: Latest = Latest::new();

/// [`change`] with the file written off the drawing thread: the window's contexts are taken now,
/// the write runs on a blocking thread, and the other windows are told when it has landed. A write
/// that fails changes nothing on screen.
///
/// The settings it read back are shown only if no newer request has been asked for meanwhile: a
/// write that lands late holds an older value than what the window already shows (and what the
/// newer request will write), and showing it would put the old value back for a moment or for good.
pub(in crate::ui) fn change_later(
    edit: impl FnOnce(&mut MailSettings) + Send + 'static,
) -> impl std::future::Future<Output = Result<(), String>> + 'static {
    let values = try_consume_context::<Signal<MailSettings>>();
    let root = try_consume_context::<PrefsRoot>().and_then(|root| root.0);
    let mine = REQUESTS.press();
    async move {
        let Some(mut values) = values else {
            return Err(crate::ui::copy::errors::NO_SETTINGS_TO_CHANGE.to_owned());
        };
        let next = match root {
            Some(root) => {
                let next = tokio::task::spawn_blocking(move || settings::change(&root, edit))
                    .await
                    .map_err(|e| e.to_string())??;
                crate::ui::configured::told_configuration();
                next
            }
            None => {
                let mut next = values.peek().clone();
                edit(&mut next);
                next
            }
        };
        if REQUESTS.is_last(mine) {
            values.set(next);
        }
        Ok(())
    }
}

/// Change one key by its schema path (`compose.spelling`) to `value`, as the Settings sheet's
/// rows hand it back. A value the settings cannot hold changes nothing.
///
/// The key is set on the settings as `change` loads them from `settings.toml`, not on the
/// window's copy, so an edit made meanwhile elsewhere (detent, `mailo notify`) is kept.
pub(in crate::ui) fn change_key(path: &str, value: toml::Value) -> Result<(), String> {
    let mut refused = None;
    change(|settings| match with_key(settings, path, value) {
        Ok(next) => *settings = next,
        Err(why) => refused = Some(why),
    })?;
    refused.map_or(Ok(()), Err)
}

/// `settings` with the key at `path` set to `value`, or why it cannot hold it.
fn with_key(
    settings: &MailSettings,
    path: &str,
    value: toml::Value,
) -> Result<MailSettings, String> {
    let values = toml::Value::try_from(settings).map_err(|e| e.to_string())?;
    crate::ui::settings_window::with_value(values, path, value)
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::Latest;
    use crate::settings::{self, BrandLogos, MailSettings, NewMail, Spelling};
    use crate::ui::appearance::WindowDirs;
    use dioxus::prelude::*;

    /// A config directory whose `settings.toml` has brand logos on.
    fn dirs_with_logos_on() -> (tempfile::TempDir, WindowDirs) {
        let dir = tempfile::tempdir().unwrap();
        let dirs = WindowDirs {
            config: dir.path().join("config"),
            state: dir.path().join("state"),
        };
        settings::change(&settings::root_for(&dirs.config), |s| {
            s.reading.brand_logos = BrandLogos::On;
        })
        .unwrap();
        (dir, dirs)
    }

    #[component]
    fn ReadsLogos() -> Element {
        let settings = super::use_settings();
        let on = settings.read().reading.brand_logos == BrandLogos::On;
        rsx! { p { "{on}" } }
    }

    /// A component drawn under no window root (as a conversation's own window drew brand logos
    /// before it had one) reads the file, not the defaults.
    #[test]
    fn use_settings_reads_the_file_where_no_root_provided_them() {
        let (_dir, dirs) = dirs_with_logos_on();
        let mut dom = VirtualDom::new(ReadsLogos).with_root_context(dirs);
        dom.rebuild_in_place();
        assert_eq!(dioxus_ssr::render(&dom), "<p>true</p>");
    }

    #[component]
    fn ChangesSpelling() -> Element {
        let _ = super::use_prefs(try_consume_context::<WindowDirs>().as_ref());
        use_hook(|| {
            // Another program turns notifications off after this window loaded its copy.
            let dirs = consume_context::<WindowDirs>();
            settings::change(&settings::root_for(&dirs.config), |s| {
                s.notifications.new_mail = NewMail::Off;
            })
            .unwrap();
            super::change_key("compose.spelling", toml::Value::String("off".to_owned())).unwrap();
        });
        rsx! {}
    }

    /// A key set from the Settings window lands on what `settings.toml` says now, so an edit made
    /// meanwhile by another program is kept.
    #[test]
    fn change_key_keeps_an_edit_made_elsewhere_meanwhile() {
        let (_dir, dirs) = dirs_with_logos_on();
        let root = settings::root_for(&dirs.config);
        let mut dom = VirtualDom::new(ChangesSpelling).with_root_context(dirs);
        dom.rebuild_in_place();
        let stored: MailSettings = settings::load(&root);
        assert_eq!(stored.compose.spelling, Spelling::Off);
        assert_eq!(stored.notifications.new_mail, NewMail::Off);
        assert_eq!(stored.reading.brand_logos, BrandLogos::On);
    }

    /// A write that lands after a newer request was asked for is not the latest, so it does not
    /// show its older settings over the newer ones.
    #[test]
    fn a_stale_write_is_not_the_latest_request() {
        let requests = Latest::default();
        let first = requests.press();
        assert!(requests.is_last(first));
        let (second, third) = (requests.press(), requests.press());
        assert!(!requests.is_last(first) && !requests.is_last(second));
        assert!(requests.is_last(third));
    }
}

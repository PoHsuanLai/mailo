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

/// The settings in force, or the defaults where nothing provided them.
pub(in crate::ui) fn current() -> MailSettings {
    try_consume_context::<Signal<MailSettings>>()
        .map(|values| values.read().clone())
        .unwrap_or_default()
}

/// Change the settings with `edit`: written to `settings.toml` where the window has somewhere
/// to write it, and shown at once either way. A write that fails changes nothing on screen.
pub(in crate::ui) fn change(edit: impl FnOnce(&mut MailSettings)) -> Result<(), String> {
    let Some(mut values) = try_consume_context::<Signal<MailSettings>>() else {
        return Err("The window has no settings to change.".to_owned());
    };
    let next = match try_consume_context::<PrefsRoot>().and_then(|root| root.0) {
        Some(root) => settings::change(&root, edit)?,
        None => {
            let mut next = values.peek().clone();
            edit(&mut next);
            next
        }
    };
    values.set(next);
    Ok(())
}

/// Change one key by its schema path (`compose.spelling`) to `value`, as the Settings sheet's
/// rows hand it back. A value the settings cannot hold changes nothing.
pub(in crate::ui) fn change_key(path: &str, value: toml::Value) -> Result<(), String> {
    let values = toml::Value::try_from(current()).map_err(|e| e.to_string())?;
    let next: MailSettings = crate::ui::settings_sheet::with_value(values, path, value)
        .try_into()
        .map_err(|e: toml::de::Error| e.to_string())?;
    change(|settings| *settings = next)
}

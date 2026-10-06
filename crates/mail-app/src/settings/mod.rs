//! mailo's own settings, the window-wide ones: one file, `mailo/settings.toml`, and the schema
//! that describes it (quire design/22 section 9).
//!
//! Each table is a struct deriving `SettingsSchema`, so the desktop's Settings app (detent) draws
//! a Mail page from the schema `mailo --write-schema <dir>` writes, and writes the same
//! file mailo reads; mailo's own Settings sheet draws the same keys. `ds_settings::Store` reads
//! the file leniently (a bad value costs only its key), writes it atomically and watches it, so a
//! change made in detent reaches an open window.
//!
//! What is not a key here stays where it was: the Spaces (`spaces.json`), the keyboard
//! (`keyboard.json`), which accounts are kept offline (`offline.json`, per account), and the
//! look, which is the desktop's (`quire/appearance.toml`).
//!
//! Before this file existed each switch had a file of its own. The first load with no
//! `settings.toml` reads them ([`legacy`]) and writes this file once; they are left where they
//! are and never written again.

mod legacy;
mod words;

pub use words::{BrandLogos, NewMail, ProviderMarks, ServerSearch, Spelling};

use ds_settings::schema::{AppId, FilePath, Page, Schema, SettingsSchema};
use ds_settings::{AppName, ConfigRoot, FileName, Format, SettingsDoc, Store};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The directory under the config root, and the schema's app id.
pub const APP: AppName = AppName("mailo");

/// The settings file, relative to the config root, as the schema names it.
const FILE: &str = "mailo/settings.toml";

/// The page mailo's keys are on in the Settings app: mailo's own.
fn page() -> Page {
    Page::App(APP.0.to_owned())
}

/// `window.*`: how the window draws mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "window", page = page())]
pub struct WindowSettings {
    #[settings(
        label = "Provider marks",
        help = "Each account's provider on its chips and tiles: the provider's icon, or a letter.",
        section = "Mail list"
    )]
    pub provider_marks: ProviderMarks,
}

/// `notifications.*`: what is said on the desktop.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "notifications", page = page())]
pub struct NotificationSettings {
    #[settings(
        label = "New mail",
        help = "A notification for each new message in an inbox, raised by mailo watch.",
        section = "Notifications"
    )]
    pub new_mail: NewMail,
}

/// `compose.*`: writing mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "compose", page = page())]
pub struct ComposeSettings {
    #[settings(
        label = "Check spelling",
        help = "Misspelt words are underlined as you write, with the system's dictionaries.",
        section = "Writing"
    )]
    pub spelling: Spelling,
}

/// `reading.*`: reading mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "reading", page = page())]
pub struct ReadingSettings {
    #[settings(
        label = "Brand logos",
        help = "A sender's verified logo (BIMI), fetched only for mail whose sender passed DMARC.",
        section = "Reading"
    )]
    pub brand_logos: BrandLogos,
}

/// `search.*`: searching mail.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, ds_settings::SettingsSchema)]
#[serde(default)]
#[settings(file = "mailo/settings.toml", domain = "search", page = page())]
pub struct SearchSettings {
    #[settings(
        label = "Search the server automatically",
        help = "A search also asks each account's server, without pressing Search on the server.",
        section = "Search"
    )]
    pub server_automatically: ServerSearch,
}

/// `mailo/settings.toml`, whole.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MailSettings {
    pub window: WindowSettings,
    pub notifications: NotificationSettings,
    pub compose: ComposeSettings,
    pub reading: ReadingSettings,
    pub search: SearchSettings,
}

impl SettingsDoc for MailSettings {
    const FILE: FileName = FileName("settings.toml");
    const FORMAT: Format = Format::Toml;
}

/// The one `mailo.settings.toml` the package installs: every table's keys under app `mailo`.
pub fn schema() -> Schema {
    let key = [
        WindowSettings::schema(),
        NotificationSettings::schema(),
        ComposeSettings::schema(),
        ReadingSettings::schema(),
        SearchSettings::schema(),
    ]
    .into_iter()
    .flat_map(|schema| schema.key)
    .collect();
    Schema {
        app: AppId(APP.0.to_owned()),
        file: FilePath(FILE.to_owned()),
        version: 1,
        key,
    }
}

/// The config root a mailo config directory stands for: `~/.config/mailo` is `~/.config`, the
/// root it is mailo's directory in. Any other directory (a test's scratch, a dev script's) is a
/// root of its own, so `settings.toml` lands in its `mailo/` and never beside it.
pub fn root_for(config_dir: &Path) -> ConfigRoot {
    match (config_dir.file_name(), config_dir.parent()) {
        (Some(name), Some(parent)) if name == APP.0 => ConfigRoot::Scratch(parent.to_path_buf()),
        _ => ConfigRoot::Scratch(config_dir.to_path_buf()),
    }
}

/// The person's own config root: the one `mail_core::config::config_dir()` is mailo's
/// directory in, as the window's is (`~/.config` on Linux, Application Support on macOS, the
/// roaming folder on Windows), so the command line, `watch` and every window read and write
/// one `settings.toml`. `None` where the machine has no config directory.
pub fn person_root() -> Option<ConfigRoot> {
    mail_core::config::config_dir().map(|dir| root_for(&dir))
}

/// The store `settings.toml` lives in, under `root`.
pub fn store(root: ConfigRoot) -> Store {
    Store::new(root, APP)
}

/// The settings in force under `root`. With no `settings.toml` yet, what the old per-switch files
/// said, written once as `settings.toml`; with neither, the defaults. A file that cannot be read
/// or written is the defaults: these are preferences, and a window opens without them.
pub fn load(root: &ConfigRoot) -> MailSettings {
    let store = store(root.clone());
    if store
        .path::<MailSettings>()
        .is_some_and(|path| path.exists())
    {
        return store.load::<MailSettings>().value;
    }
    let Some(dir) = store.dir() else {
        return MailSettings::default();
    };
    let carried = legacy::read(&dir);
    if carried != MailSettings::default() {
        let _ = store.save(&carried);
    }
    carried
}

/// Change the settings under `root` with `edit` and write them, returning what was written.
pub fn change(
    root: &ConfigRoot,
    edit: impl FnOnce(&mut MailSettings),
) -> Result<MailSettings, String> {
    let mut settings = load(root);
    edit(&mut settings);
    store(root.clone())
        .save(&settings)
        .map_err(|e| e.to_string())?;
    Ok(settings)
}

/// `mailo --write-schema <dir>`: write `mailo.settings.toml` into `dir`.
pub fn write_schema(dir: &Path) -> Result<String, String> {
    let path = schema().write_to(dir).map_err(|e| e.to_string())?;
    Ok(format!("wrote {}\n", path.display()))
}

#[cfg(test)]
mod tests;

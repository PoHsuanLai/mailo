//! Where mailo keeps its files on this machine: one rule per platform, in one place.
//!
//! On Linux and the BSDs it is the XDG base directory rule, spelled out here rather than taken
//! from a crate, so the directories an existing installation already uses cannot move: the
//! variable when it is set, else the fixed path under `$HOME`. On macOS and Windows it is the
//! folders the system names, through `dirs`:
//!
//! | place  | Linux                        | macOS                          | Windows                 |
//! |--------|------------------------------|--------------------------------|-------------------------|
//! | config | `$XDG_CONFIG_HOME` or `~/.config`      | `~/Library/Application Support` | `%APPDATA%` (roaming)   |
//! | data   | `$XDG_DATA_HOME` or `~/.local/share`   | `~/Library/Application Support` | `%LOCALAPPDATA%`        |
//! | state  | `$XDG_STATE_HOME` or `~/.local/state`  | `~/Library/Application Support` | `%LOCALAPPDATA%`        |
//! | cache  | `$XDG_CACHE_HOME` or `~/.cache`        | `~/Library/Caches`              | `%LOCALAPPDATA%`        |
//!
//! each with `mailo` under it. The mail database is data, and on Windows data is the local
//! folder, not the roaming one: a mail store copied to a domain server at every sign-out is not
//! something anybody asked for. macOS and Windows have no separate place for state, and put it
//! with local data, as their own applications do. No two places hold a file of the same name, so
//! sharing a folder is safe.
//!
//! quire resolves its own `appearance.toml` by the XDG rule on every platform
//! (`ds_settings::config_dir`); `mail-app`'s `appearance` module says what follows from that.

use std::ffi::OsString;
use std::path::PathBuf;

/// The kinds of file mailo keeps, by how long they matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Preferences and the OAuth registry: what the user chose.
    Config,
    /// The mail database and its blobs.
    Data,
    /// What the window remembers between runs that is neither a choice nor mail.
    State,
    /// What can be fetched again: provider icons.
    Cache,
}

/// The directory `place` names for this user, with `mailo` at its end; `None` when this user has
/// no home to put it under.
pub fn dir(place: Place) -> Option<PathBuf> {
    base(place).map(|base| base.join(APP))
}

/// The user's home directory.
pub fn home() -> Option<PathBuf> {
    #[cfg(not(any(target_os = "macos", windows)))]
    return std::env::var_os("HOME").map(PathBuf::from);
    #[cfg(any(target_os = "macos", windows))]
    return dirs::home_dir();
}

/// The downloads folder the desktop names, if it names one: `$XDG_DOWNLOAD_DIR` on Linux, the
/// system's Downloads folder on macOS and Windows. The caller decides what to do without one.
pub fn downloads_named() -> Option<OsString> {
    #[cfg(not(any(target_os = "macos", windows)))]
    return std::env::var_os("XDG_DOWNLOAD_DIR");
    #[cfg(any(target_os = "macos", windows))]
    return dirs::download_dir().map(PathBuf::into_os_string);
}

const APP: &str = "mailo";

#[cfg(not(any(target_os = "macos", windows)))]
fn base(place: Place) -> Option<PathBuf> {
    xdg(place, |name| std::env::var_os(name))
}

#[cfg(any(target_os = "macos", windows))]
fn base(place: Place) -> Option<PathBuf> {
    match place {
        Place::Config => dirs::config_dir(),
        Place::Data | Place::State => dirs::data_local_dir(),
        Place::Cache => dirs::cache_dir(),
    }
}

/// The XDG base directory rule over an environment given as a function, so it can be tested
/// without setting a variable (which is `unsafe` since the 2024 edition, and the workspace
/// forbids `unsafe`). Compiled everywhere, so the rule is tested everywhere.
#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
fn xdg(place: Place, var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let (name, under_home) = match place {
        Place::Config => ("XDG_CONFIG_HOME", ".config"),
        Place::Data => ("XDG_DATA_HOME", ".local/share"),
        Place::State => ("XDG_STATE_HOME", ".local/state"),
        Place::Cache => ("XDG_CACHE_HOME", ".cache"),
    };
    var(name)
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(under_home)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(set: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn the_xdg_rule_is_the_variable_else_the_path_under_home() {
        type Env = &'static [(&'static str, &'static str)];
        let cases: &[(Place, Env, Option<&str>)] = &[
            (Place::Config, &[("HOME", "/h")], Some("/h/.config")),
            (Place::Data, &[("HOME", "/h")], Some("/h/.local/share")),
            (Place::State, &[("HOME", "/h")], Some("/h/.local/state")),
            (Place::Cache, &[("HOME", "/h")], Some("/h/.cache")),
            (
                Place::Data,
                &[("HOME", "/h"), ("XDG_DATA_HOME", "/d")],
                Some("/d"),
            ),
            (Place::Config, &[("XDG_CONFIG_HOME", "/c")], Some("/c")),
            // Another place's variable is not this one's.
            (
                Place::Cache,
                &[("HOME", "/h"), ("XDG_CONFIG_HOME", "/c")],
                Some("/h/.cache"),
            ),
            (Place::State, &[], None),
        ];
        for (place, set, expect) in cases {
            assert_eq!(
                xdg(*place, env(set)),
                expect.map(PathBuf::from),
                "{place:?} with {set:?}"
            );
        }
    }

    #[test]
    fn every_place_is_mailos_own() {
        for place in [Place::Config, Place::Data, Place::State, Place::Cache] {
            if let Some(dir) = dir(place) {
                assert_eq!(
                    dir.file_name(),
                    Some(std::ffi::OsStr::new(APP)),
                    "{place:?}"
                );
            }
        }
    }
}

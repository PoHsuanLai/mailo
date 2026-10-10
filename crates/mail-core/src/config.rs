//! Where mailo keeps what is not mail, and how it reads and writes it.
//!
//! Config, state and cache directories, and the JSON files the switches (notifications,
//! offline mode, brand logos, server search, ...) are kept in. Config, not mail: it lives
//! under `$XDG_CONFIG_HOME/mailo/` (`~/.config/mailo/` when unset) on Linux and in the system's
//! per-user folder on macOS and Windows (`mail_runtime::places`), never in the mail database.
//! A cosmetic choice must not be a write to the file that holds someone's mail, or be able to
//! fail a migration.

use crate::error::CoreError;
use mail_runtime::places::{self, Place};
use std::path::{Path, PathBuf};

/// Read `file_name` from `dir`.
///
/// A missing file, or one that is not this type's JSON, is `T::default`: a
/// damaged preference must not stop the window opening.
pub fn read_json<T>(dir: &Path, file_name: &str) -> T
where
    T: serde::de::DeserializeOwned + Default,
{
    let Ok(bytes) = std::fs::read(dir.join(file_name)) else {
        return T::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write `value` as JSON to `dir/file_name`, creating `dir` if needed.
///
/// The bytes land in a temporary file in `dir` and are renamed into place, so a
/// crash mid-write cannot leave a half-written file for the next launch.
pub fn write_json(
    dir: &Path,
    file_name: &str,
    value: &impl serde::Serialize,
) -> Result<(), CoreError> {
    let mut body = serde_json::to_string(value)?;
    body.push('\n');
    write_text(dir, file_name, &body)
}

/// Write `body` to `dir/file_name` by a temporary file and a rename, creating `dir` if needed.
pub fn write_text(dir: &Path, file_name: &str, body: &str) -> Result<(), CoreError> {
    std::fs::create_dir_all(dir).map_err(CoreError::at(dir))?;
    let path = dir.join(file_name);
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, body).map_err(CoreError::at(&tmp))?;
    std::fs::rename(&tmp, &path).map_err(CoreError::at(&path))
}

/// mailo's config directory: `$XDG_CONFIG_HOME/mailo`, else `$HOME/.config/mailo`, on Linux;
/// the system's per-user folder on macOS and Windows (`mail_runtime::places`); else None.
pub fn config_dir() -> Option<PathBuf> {
    places::dir(Place::Config)
}

/// `$XDG_STATE_HOME/mailo`, else `$HOME/.local/state/mailo`, on Linux; the local
/// application data folder on macOS and Windows; else None.
///
/// Today lives here, not beside appearance: it is a list of what was opened,
/// and it expires, so it is state rather than a preference.
pub fn state_dir() -> Option<PathBuf> {
    places::dir(Place::State)
}

/// `$XDG_CACHE_HOME/mailo`, else `$HOME/.cache/mailo`, on Linux; the system's cache folder
/// on macOS and Windows; else None.
///
/// Provider icons live here, under `providers/`. They are not config and not the
/// mail database: a missing cache is the letter on the chip, not a lost account.
pub fn cache_dir() -> Option<PathBuf> {
    places::dir(Place::Cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directories_are_mailos() {
        // The XDG resolution itself moved to `ds_settings::dirs`, with its tests; what is
        // left to hold here is that mailo asks for its own name.
        for dir in [config_dir(), state_dir(), cache_dir()]
            .into_iter()
            .flatten()
        {
            assert_eq!(
                dir.file_name().and_then(|n| n.to_str()),
                Some("mailo"),
                "{dir:?}"
            );
        }
    }
}

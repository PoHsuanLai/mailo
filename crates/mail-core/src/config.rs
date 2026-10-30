//! Where mailo keeps what is not mail, and how it reads and writes it.
//!
//! Config, state and cache directories, and the JSON files the switches (notifications,
//! offline mode, brand logos, server search, ...) are kept in. Config, not mail: it lives
//! under `$XDG_CONFIG_HOME/mailo/` (`~/.config/mailo/` when unset) on Linux and in the system's
//! per-user folder on macOS and Windows (`mail_secrets::places`), never in the mail database.
//! A cosmetic choice must not be a write to the file that holds someone's mail, or be able to
//! fail a migration.

use crate::error::CoreError;
use mail_secrets::places::{self, Place};
use std::path::{Path, PathBuf};

/// Read `file_name` from `dir`.
///
/// A missing file, or one that is not this type's JSON, is `T::default`: a
/// damaged preference must not stop the window opening.
pub fn read_json<T>(dir: &Path, file_name: &str) -> T
where
    T: serde::de::DeserializeOwned + Default,
{
    let path = dir.join(file_name);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        // No file is the ordinary first run, and not worth a line.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            log::warn!("{}: {e}; using the defaults", path.display());
            return T::default();
        }
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        log::warn!("{}: {e}; using the defaults", path.display());
        T::default()
    })
}

/// Why `dir/file_name` cannot be read as a `T`: `None` when it is missing or reads. A file that
/// is there and cannot be used is the person's, so a caller asks before putting the defaults in
/// its place, and never writes over it.
pub fn json_damage<T: serde::de::DeserializeOwned>(dir: &Path, file_name: &str) -> Option<String> {
    let bytes = match std::fs::read(dir.join(file_name)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => return Some(e.to_string()),
    };
    serde_json::from_slice::<T>(&bytes)
        .err()
        .map(|e| e.to_string())
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
    mail_store::write_atomic(&path, body.as_bytes(), Some(0o600)).map_err(CoreError::at(&path))
}

/// Make `dir`, its owner's alone: [`places::make_private`].
///
/// # Errors
/// When the folder cannot be made or its mode cannot be set.
pub fn make_private(dir: &Path) -> std::io::Result<()> {
    places::make_private(dir)
}

/// The user's home directory.
pub fn home_dir() -> Option<PathBuf> {
    places::home()
}

/// mailo's config directory: `$XDG_CONFIG_HOME/mailo`, else `$HOME/.config/mailo`, on Linux;
/// the system's per-user folder on macOS and Windows (`mail_secrets::places`); else None.
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

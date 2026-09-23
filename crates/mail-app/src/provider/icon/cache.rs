//! The cache directory: the atomic store, and reading it back once at startup.

use super::{IconError, PNG_MAGIC};
use crate::provider::Provider;
use base64::Engine as _;
use std::path::{Path, PathBuf};

/// Icons read once from the cache directory.
///
/// Missing files are absent. The chip then draws the letter, which is also what
/// a window with the setting on letters draws even when the file is there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Loaded {
    uris: [Option<String>; 6],
}

impl Loaded {
    /// Read every cached PNG under `dir`. A file that is not a PNG is skipped.
    pub(crate) fn read(dir: &Path) -> Self {
        let mut loaded = Self::default();
        for provider in Provider::ALL {
            let Some(path) = cached(dir, provider) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.starts_with(&PNG_MAGIC) {
                loaded.uris[slot(provider)] = Some(data_uri(&bytes));
            }
        }
        loaded
    }

    /// The `data:image/png;base64,` URI for `provider`, when one was cached.
    pub(crate) fn uri(&self, provider: Provider) -> Option<String> {
        self.uris[slot(provider)].clone()
    }
}

/// The cache file for `provider`, when it is a regular file.
pub(crate) fn cached(dir: &Path, provider: Provider) -> Option<PathBuf> {
    let path = dir.join(format!("{}.png", file_stem(provider)));
    path.is_file().then_some(path)
}

/// Write `png` as `<provider>.png`, via a temporary file in the same directory.
///
/// A failure removes the temporary file. The destination is replaced only by
/// rename, so a crash mid-write cannot leave a half-written PNG under the name
/// [`cached`] reads.
pub(crate) fn store(dir: &Path, provider: Provider, png: &[u8]) -> Result<(), IconError> {
    std::fs::create_dir_all(dir)
        .map_err(|err| IconError::Store(format!("{}: {err}", dir.display())))?;
    let name = format!("{}.png", file_stem(provider));
    let dest = dir.join(&name);
    let part = dir.join(format!(".{name}.part"));
    if let Err(err) = std::fs::write(&part, png) {
        let _ = std::fs::remove_file(&part);
        return Err(IconError::Store(format!("{}: {err}", part.display())));
    }
    if let Err(err) = std::fs::rename(&part, &dest) {
        let _ = std::fs::remove_file(&part);
        return Err(IconError::Store(format!("{}: {err}", dest.display())));
    }
    Ok(())
}

pub(super) fn file_stem(provider: Provider) -> &'static str {
    match provider {
        Provider::Google => "google",
        Provider::Microsoft => "microsoft",
        Provider::Fastmail => "fastmail",
        Provider::Icloud => "icloud",
        Provider::Yahoo => "yahoo",
        Provider::Imap => "imap",
    }
}

fn slot(provider: Provider) -> usize {
    match provider {
        Provider::Google => 0,
        Provider::Microsoft => 1,
        Provider::Fastmail => 2,
        Provider::Icloud => 3,
        Provider::Yahoo => 4,
        Provider::Imap => 5,
    }
}

fn data_uri(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

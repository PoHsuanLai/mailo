//! The emoji the user picked last, newest first: the picker's first tab.
//!
//! Kept in `emoji.json` in the state directory beside Today's file, because it is a record of
//! what was used rather than a preference. A stored glyph the table does not have is dropped when
//! the file is read, so what comes back is always something the picker can draw.

use std::path::Path;

use super::{Emoji, find};

/// Where the list is kept, in the state directory.
pub const FILE_NAME: &str = "emoji.json";

/// How many are remembered.
pub const KEPT: usize = 27;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    recent: Vec<String>,
}

/// The remembered emoji, newest first. Nothing when there is no file or it cannot be read.
pub fn load(dir: &Path) -> Vec<&'static Emoji> {
    let stored = mail_core::config::read_json::<Stored>(dir, FILE_NAME);
    let mut out: Vec<&'static Emoji> = Vec::new();
    for glyph in &stored.recent {
        if let Some(emoji) = find(glyph)
            && !out.contains(&emoji)
        {
            out.push(emoji);
        }
    }
    out.truncate(KEPT);
    out
}

/// Remember `recent` in `dir`.
pub fn save(dir: &Path, recent: &[&'static Emoji]) -> Result<(), String> {
    let stored = Stored {
        recent: recent.iter().map(|emoji| emoji.glyph.to_owned()).collect(),
    };
    mail_core::config::write_json(dir, FILE_NAME, &stored)
}

/// `recent` with `picked` first: moved there if it was already in it, and the oldest dropped
/// past [`KEPT`].
pub fn remember(recent: &[&'static Emoji], picked: &'static Emoji) -> Vec<&'static Emoji> {
    std::iter::once(picked)
        .chain(recent.iter().copied().filter(|emoji| *emoji != picked))
        .take(KEPT)
        .collect()
}

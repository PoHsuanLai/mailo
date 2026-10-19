//! The files mailo keeps settings in that are there and cannot be used, found before anything
//! runs over them.
//!
//! A damaged file is the person's: it is never taken for a missing one and never written over
//! unasked. The window asks before it opens (`ui::launch`'s gate): Use Defaults sets each file
//! aside ([`set_aside`]) and opens on the defaults, Quit leaves every file as it was. The command
//! line says what is wrong and stops (`cli::exec`).

use ds_settings::ConfigRoot;
use std::path::{Path, PathBuf};

/// One file that cannot be used, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Damaged {
    pub path: PathBuf,
    pub why: String,
    fix: Fix,
}

/// How a file is put right once the person agrees to the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fix {
    /// `settings.toml`: written again from what could be read. A value that was not valid takes
    /// its default and every other line, comment and unknown key stays; a file that could not be
    /// read at all is moved aside by the write (`ds_settings::Store::save`).
    Rewrite(ConfigRoot),
    /// Moved aside to `<name>.bad-<seconds>`, so the next write starts a new one.
    MoveAside,
}

impl std::fmt::Display for Damaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.why)
    }
}

/// The damaged files in `config` that the command line and the window both read: the settings
/// and the offline choices. The window adds its own (`ui::launch::gate`).
pub fn of_config(config: &Path) -> Vec<Damaged> {
    let root = crate::settings::root_for(config);
    let settings = crate::settings::damage(&root).map(|why| Damaged {
        path: crate::settings::store(root.clone())
            .path::<crate::settings::MailSettings>()
            .unwrap_or_else(|| config.join("settings.toml")),
        why,
        fix: Fix::Rewrite(root),
    });
    settings
        .into_iter()
        .chain(moved(
            config,
            mail_core::offline::FILE_NAME,
            mail_core::offline::damage(config),
        ))
        .collect()
}

/// `name` in `dir`, damaged when `why` says so, to be moved aside once the person agrees.
pub fn moved(dir: &Path, name: &str, why: Option<String>) -> Option<Damaged> {
    why.map(|why| Damaged {
        path: dir.join(name),
        why,
        fix: Fix::MoveAside,
    })
}

/// Put each of `found` right as the person agreed: the defaults take its place, and what it held
/// is kept (`<name>.bad-<now>` beside it) except a `settings.toml` value that was not valid.
/// Stops at the first that cannot be, saying why.
pub fn set_aside(found: &[Damaged], now: i64) -> Result<(), String> {
    for damaged in found {
        match &damaged.fix {
            Fix::Rewrite(root) => {
                let store = crate::settings::store(root.clone());
                let read = store.load::<crate::settings::MailSettings>().value;
                store.save(&read).map_err(|e| e.to_string())?;
            }
            Fix::MoveAside => {
                move_aside(&damaged.path, now).map_err(|e| format!("{damaged}: {e}"))?;
            }
        }
    }
    Ok(())
}

/// Rename `path` to the first free `<name>.bad-<now>` (then `-1`, `-2`, ...) beside it.
fn move_aside(path: &Path, now: i64) -> std::io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the path has no file name"))?
        .to_string_lossy()
        .into_owned();
    let free = (0u32..)
        .map(|n| match n {
            0 => path.with_file_name(format!("{name}.bad-{now}")),
            n => path.with_file_name(format!("{name}.bad-{now}-{n}")),
        })
        .find(|candidate| std::fs::symlink_metadata(candidate).is_err())
        .ok_or_else(|| std::io::Error::other("no free name"))?;
    std::fs::rename(path, &free)?;
    Ok(free)
}

#[cfg(test)]
mod tests;

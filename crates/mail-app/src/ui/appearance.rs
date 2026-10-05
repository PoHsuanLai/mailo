//! Where the window's look is remembered.
//!
//! Config, not mail: it lives under `$XDG_CONFIG_HOME/mailo/` (`~/.config/mailo/` when unset)
//! on Linux and in the system's per-user folder on macOS and Windows
//! (`mail_runtime::places`), never in the mail database. A cosmetic choice must not be a write
//! to the file that holds someone's mail, or be able to fail a migration.
//!
//! The look of the window is the desktop's, not mailo's: theme, accent, motion and typeface are
//! `quire/appearance.toml`, the one file the shell and detent read and write too, and the
//! window follows it live (`launch::DesktopSettings`). mailo keeps no appearance file of its own
//! and writes none. Three older files are still read from mailo's own directory:
//!
//! - `mailo.toml` held mailo's own choice of provider marks. That is a key of
//!   `mailo/settings.toml` now (`crate::settings`), which read it once; nothing writes it.
//! - `appearance.toml` is what mailo kept for quire between v0.2.0 and the desktop's own store.
//!   [`adopt`] copies it into the desktop's directory once, when the desktop has none, and never
//!   writes or deletes it, so going back to an older build loses nothing.
//! - `appearance.json` is what mailo wrote before quire. Read, never written or deleted
//!   ([`legacy`]); its theme seeds the desktop's file when nothing else does.

use super::view::Marks;
use ds::prelude::*;
use ds_settings::{AppName, AppearanceFile, ConfigRoot};
use mail_core::config::{read_json, write_text};
use serde::Deserialize;
use serde::de::Deserializer;
use std::path::{Path, PathBuf};

/// What mailo wrote before quire: theme, motion and marks in one JSON file.
pub const LEGACY_FILE_NAME: &str = "appearance.json";

/// The desktop's appearance file, and the person's stylesheet beside it.
const QUIRE_FILE_NAME: &str = "appearance.toml";
const STYLE_FILE_NAME: &str = "style.css";

/// What [`adopt`] did, for the one line `main` may say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adopted {
    /// The desktop already has its own `appearance.toml`: nothing to bring over.
    Nothing,
    /// mailo's own `appearance.toml` was copied into the desktop's directory.
    FromToml,
    /// The theme in `appearance.json` seeded the desktop's `appearance.toml`.
    FromJson,
    /// Neither old file exists: a first run, and nothing is written.
    FirstRun,
}

/// Bring mailo's old appearance into the desktop's store, once.
///
/// `from` is mailo's config directory and `to` the desktop's (`quire/`). The desktop's
/// `appearance.toml` wins whenever it exists: it is the person's, set in Settings or by another
/// program, and a file mailo kept for itself must never override it. Only when it is missing is
/// mailo's `appearance.toml` copied over byte for byte (its keys are the desktop's: it was
/// quire's file), else the theme of `appearance.json` seeds one; the accents mailo had are
/// retired and motion is an accessibility preference now, so neither carries. The person's
/// `style.css` follows the same rule. Nothing in `from` is written, moved or deleted, and once
/// `to` has the file this does nothing, so mailo stops touching its own copy for good.
pub fn adopt(from: &Path, to: &Path) -> Adopted {
    let mut adopted = adopt_file(from, to, QUIRE_FILE_NAME).unwrap_or_else(|| {
        if to.join(QUIRE_FILE_NAME).exists() {
            Adopted::Nothing
        } else if from.join(LEGACY_FILE_NAME).exists() {
            let mut file = AppearanceFile::default();
            file.appearance.theme = legacy(from).theme;
            match toml::to_string(&file).map(|text| write_text(to, QUIRE_FILE_NAME, &text)) {
                Ok(Ok(())) => Adopted::FromJson,
                _ => Adopted::FirstRun,
            }
        } else {
            Adopted::FirstRun
        }
    });
    if adopt_file(from, to, STYLE_FILE_NAME).is_some() && adopted == Adopted::Nothing {
        adopted = Adopted::FromToml;
    }
    adopted
}

/// `name` copied from `from` to `to` when `to` has none and `from` has one: `Some` when it was.
fn adopt_file(from: &Path, to: &Path, name: &str) -> Option<Adopted> {
    if to.join(name).exists() {
        return None;
    }
    let text = std::fs::read_to_string(from.join(name)).ok()?;
    write_text(to, name, &text).ok()?;
    Some(Adopted::FromToml)
}

/// What `appearance.json` said, read leniently and never written.
///
/// The theme seeds `appearance.toml` and a Space written before Spaces had a theme; the marks
/// seed `mailo.toml`. Its accent and motion are read by nobody.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct Legacy {
    /// Which palette the window resolved to.
    #[serde(deserialize_with = "de_theme")]
    pub theme: Theme,
    /// Provider marks: their icons, or the letter.
    #[serde(deserialize_with = "de_marks")]
    pub marks: Marks,
}

/// `dir/appearance.json`, or the first-run value when there is none or it cannot be read.
pub fn legacy(dir: &Path) -> Legacy {
    read_json(dir, LEGACY_FILE_NAME)
}

/// A stored word, or `None` when the value is some other shape (a number, a table).
fn word<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Word(String),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Word(word) => Some(word),
        Stored::Other(_) => None,
    })
}

fn de_theme<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Theme, D::Error> {
    Ok(word(deserializer)?
        .and_then(|word| Theme::parse(&word))
        .unwrap_or_default())
}

fn de_marks<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Marks, D::Error> {
    Ok(word(deserializer)?
        .and_then(|word| Marks::parse(&word))
        .unwrap_or_default())
}

/// Where the desktop keeps its appearance: [`ConfigRoot::Xdg`] on every platform, the `quire`
/// directory. On macOS that is `~/.config/quire`, and on Windows, where `HOME` is not usually
/// set, it is None and the window's look is quire's default. quire owns that rule, so the
/// migration ([`adopt`]) is handed this directory and not mailo's.
pub fn desktop_dir() -> Option<PathBuf> {
    ConfigRoot::Xdg.dir(AppName::QUIRE)
}

/// Config and state directories the window reads and writes.
///
/// Tests pass a [`tempfile`] pair. A launch with no home directory passes none,
/// and the window keeps the choices in memory for the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDirs {
    pub config: PathBuf,
    pub state: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::{Adopted, Legacy, adopt, legacy};
    use crate::ui::view::{Marks, Theme};
    use ds::prelude::{Accent, Motion};
    use std::path::Path;

    fn entries(dir: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|entry| {
                entry
                    .unwrap_or_else(|e| panic!("{e}"))
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn a_partial_file_keeps_the_fields_it_has() {
        // `sepia` alone matching the default is not enough: a loader that rejects the whole
        // file also returns the default. The rows that set another field are what show the
        // bad word was dropped on its own.
        let cases: &[(&str, &str, Legacy)] = &[
            (
                "theme only",
                r#"{"theme":"dark"}"#,
                Legacy {
                    theme: Theme::Dark,
                    ..Legacy::default()
                },
            ),
            (
                "unknown theme keeps the marks",
                r#"{"theme":"sepia","marks":"letters"}"#,
                Legacy {
                    marks: Marks::Letters,
                    ..Legacy::default()
                },
            ),
            (
                "marks letters",
                r#"{"marks":"letters"}"#,
                Legacy {
                    marks: Marks::Letters,
                    ..Legacy::default()
                },
            ),
            (
                "unknown marks keeps the theme",
                r#"{"marks":"crests","theme":"dark"}"#,
                Legacy {
                    theme: Theme::Dark,
                    ..Legacy::default()
                },
            ),
        ];
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("appearance.json");
        for &(name, bytes, look) in cases {
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(legacy(dir.path()), look, "{name}: {bytes}");
        }
    }

    #[test]
    fn the_accent_and_motion_are_dropped_on_read_and_theme_and_marks_are_kept() {
        // The migration table. Every row names a theme or marks that are not the default, so a
        // loader that threw the file away on meeting `accent` or `motion` would fail it:
        // dropping a retired field must not drop the fields beside it.
        let cases: &[(&str, &str, Legacy)] = &[
            (
                "an old file with every field",
                r#"{"theme":"dark","accent":"pine","motion":"calm"}"#,
                Legacy {
                    theme: Theme::Dark,
                    marks: Marks::Icons,
                },
            ),
            (
                "an accent this build never knew",
                r#"{"accent":"rose","theme":"light"}"#,
                Legacy {
                    theme: Theme::Light,
                    ..Legacy::default()
                },
            ),
            (
                "an accent that is not a word",
                r#"{"accent":7,"marks":"letters"}"#,
                Legacy {
                    marks: Marks::Letters,
                    ..Legacy::default()
                },
            ),
            (
                "an unknown field is ignored",
                r#"{"theme":"light","future":true,"motion":"extra","marks":"letters"}"#,
                Legacy {
                    theme: Theme::Light,
                    marks: Marks::Letters,
                },
            ),
        ];
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("appearance.json");
        for &(name, bytes, look) in cases {
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(legacy(dir.path()), look, "{name}: {bytes}");
            let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(raw, bytes, "{name}: the JSON was rewritten");
        }
    }

    fn write(dir: &Path, name: &str, text: &str) {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(dir.join(name), text).unwrap_or_else(|e| panic!("{name}: {e}"));
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    #[test]
    fn the_old_appearance_moves_into_the_desktops_store_once_and_mailos_copy_is_never_touched() {
        // design/22-SETTINGS.md section 2, "Mailo migration": mailo's file is read once, when
        // the desktop has none, and written nowhere. TempDirs, never the real config directory.
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (mailo, desktop) = (root.path().join("mailo"), root.path().join("quire"));
        let kept = "[appearance]\ntheme = \"dark\"\naccent = \"green\"\n";
        write(&mailo, "appearance.toml", kept);
        write(&mailo, "style.css", "body { color: red }");
        write(&mailo, "appearance.json", r#"{"theme":"light"}"#);

        assert_eq!(adopt(&mailo, &desktop), Adopted::FromToml);
        assert_eq!(
            read(&desktop, "appearance.toml"),
            kept,
            "the toml, not the json"
        );
        assert_eq!(read(&desktop, "style.css"), "body { color: red }");

        // The desktop's is the person's now: a later edit of mailo's copy never reaches it, and
        // nothing in mailo's directory moved.
        write(
            &mailo,
            "appearance.toml",
            "[appearance]\ntheme = \"light\"\n",
        );
        assert_eq!(adopt(&mailo, &desktop), Adopted::Nothing);
        assert_eq!(read(&desktop, "appearance.toml"), kept);
        assert_eq!(read(&mailo, "appearance.json"), r#"{"theme":"light"}"#);
        assert_eq!(
            entries(&mailo),
            ["appearance.json", "appearance.toml", "style.css"]
        );
    }

    #[test]
    fn a_desktop_that_has_an_appearance_is_never_overwritten() {
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (mailo, desktop) = (root.path().join("mailo"), root.path().join("quire"));
        write(
            &mailo,
            "appearance.toml",
            "[appearance]\ntheme = \"dark\"\n",
        );
        write(
            &desktop,
            "appearance.toml",
            "[appearance]\ntheme = \"light\"\n",
        );
        assert_eq!(adopt(&mailo, &desktop), Adopted::Nothing);
        assert_eq!(
            read(&desktop, "appearance.toml"),
            "[appearance]\ntheme = \"light\"\n"
        );
    }

    #[test]
    fn appearance_json_seeds_the_desktops_file_when_it_is_all_there_is() {
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (mailo, desktop) = (root.path().join("mailo"), root.path().join("quire"));
        let json = r#"{"theme":"dark","motion":"calm","marks":"letters"}"#;
        write(&mailo, "appearance.json", json);

        assert_eq!(adopt(&mailo, &desktop), Adopted::FromJson);
        let file: ds_settings::AppearanceFile =
            toml::from_str(&read(&desktop, "appearance.toml")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(file.appearance.theme, Theme::Dark);
        // Motion is an accessibility preference now, and mailo's "calm" was a style: it starts
        // at Standard. mailo retired its accents, so there is nothing to carry: Blue.
        assert_eq!(file.appearance.motion_level, Motion::Standard);
        assert_eq!(file.appearance.accent, Accent::Blue);
        assert_eq!(
            read(&mailo, "appearance.json"),
            json,
            "the JSON was modified"
        );
        assert_eq!(entries(&mailo), ["appearance.json"]);

        // A second run reads nothing from the JSON any more.
        write(&mailo, "appearance.json", r#"{"theme":"light"}"#);
        assert_eq!(adopt(&mailo, &desktop), Adopted::Nothing);
        let again: ds_settings::AppearanceFile =
            toml::from_str(&read(&desktop, "appearance.toml")).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again, file);
    }

    #[test]
    fn nothing_old_is_a_first_run_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let (mailo, desktop) = (root.path().join("mailo"), root.path().join("quire"));
        assert_eq!(adopt(&mailo, &desktop), Adopted::FirstRun);
        assert!(!desktop.exists());
    }
}

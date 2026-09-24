//! Where the window's look is remembered.
//!
//! Config, not mail: it lives under `$XDG_CONFIG_HOME/mailo/` (`~/.config/mailo/` when unset),
//! never in the mail database. A cosmetic choice must not be a write to the file that holds
//! someone's mail, or be able to fail a migration.
//!
//! Three files, each with one owner:
//!
//! - `appearance.toml` is quire's ([`ds_settings::AppearanceFile`]): theme, accent and motion,
//!   read and watched by `ds_settings::use_environment`. [`quire`] creates it on the first run
//!   after the move by importing `appearance.json` once.
//! - `mailo.toml` is mailo's own ([`Appearance`]): what has no place in any quire type, today
//!   whether a provider chip shows its icon. On the first run after the move it starts from
//!   the `marks` in `appearance.json`.
//! - `appearance.json` is what mailo wrote before quire. It is read, never written or
//!   deleted, so going back to an older build loses nothing ([`legacy`]).

use crate::view::{Appearance, Marks, Motion, Theme};
use ds_settings::{AppName, FileName, Format, Settings};
use serde::Deserialize;
use serde::de::Deserializer;
use std::path::{Path, PathBuf};

/// What mailo wrote before quire: theme, motion and marks in one JSON file.
pub const LEGACY_FILE_NAME: &str = "appearance.json";

/// mailo's own window preferences, beside quire's `appearance.toml`.
const PREFS: Settings<Appearance> = Settings::new(FileName("mailo.toml"), Format::Toml);

/// The stored preferences, or the first-run value when there are none or they cannot be read.
///
/// Before `mailo.toml` exists, the marks are the ones `appearance.json` holds, so the first run
/// after the move shows the chips as the last run did. Not an error the user needs to see: a
/// missing or damaged file means the window looks as it did on first run.
pub fn load(dir: &Path) -> Appearance {
    if PREFS.at(dir).path().exists() {
        return PREFS.load(dir);
    }
    Appearance {
        marks: legacy(dir).marks,
    }
}

/// Write `look` to `dir/mailo.toml`, creating the directory if needed.
///
/// The bytes land in a temporary file in `dir` and are renamed into place, so a crash
/// mid-write cannot leave a half-written file for the next launch.
pub fn save(dir: &Path, look: Appearance) -> Result<(), String> {
    PREFS.save(dir, &look).map_err(|e| e.to_string())
}

/// quire's `appearance.toml` in `dir`, importing `appearance.json` into it the first time.
///
/// The import happens once, when the TOML file does not exist yet, and never touches the JSON
/// file. `ds_settings::use_environment` reads and watches the TOML from then on; it does not
/// import on its own for mailo, so `main` calls this before the window opens.
pub fn quire(dir: &Path) -> ds_settings::AppearanceFile {
    ds_settings::load_or_import(dir, &dir.join(LEGACY_FILE_NAME)).unwrap_or_default()
}

/// What `appearance.json` said, read leniently and never written.
///
/// Theme and motion are per Space now: a Space written before that inherits these on its
/// first read (`space::load`). The marks seed `mailo.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct Legacy {
    /// Which palette the window resolved to.
    #[serde(deserialize_with = "de_theme")]
    pub theme: Theme,
    /// How much the window moved.
    #[serde(deserialize_with = "de_motion")]
    pub motion: Motion,
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

fn de_motion<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Motion, D::Error> {
    Ok(word(deserializer)?
        .and_then(|word| Motion::parse(&word))
        .unwrap_or_default())
}

fn de_marks<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Marks, D::Error> {
    Ok(word(deserializer)?
        .and_then(|word| Marks::parse(&word))
        .unwrap_or_default())
}

/// Read `file_name` from `dir`.
///
/// A missing file, or one that is not this type's JSON, is `T::default`: a
/// damaged preference must not stop the window opening.
pub(crate) fn read_json<T>(dir: &Path, file_name: &str) -> T
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
pub(crate) fn write_json(
    dir: &Path,
    file_name: &str,
    value: &impl serde::Serialize,
) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(file_name);
    let tmp = path.with_extension("part");
    let mut body = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    body.push(b'\n');
    std::fs::write(&tmp, &body).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// `$XDG_CONFIG_HOME/mailo`, else `$HOME/.config/mailo`, else None.
pub fn config_dir() -> Option<PathBuf> {
    ds_settings::config_dir(AppName::MAILO)
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

/// `$XDG_STATE_HOME/mailo`, else `$HOME/.local/state/mailo`, else None.
///
/// Today lives here, not beside appearance: it is a list of what was opened,
/// and it expires, so it is state rather than a preference.
pub fn state_dir() -> Option<PathBuf> {
    ds_settings::state_dir(AppName::MAILO)
}

/// `$XDG_CACHE_HOME/mailo`, else `$HOME/.cache/mailo`, else None.
///
/// Provider icons live here, under `providers/`. They are not config and not the
/// mail database: a missing cache is the letter on the chip, not a lost account.
pub fn cache_dir() -> Option<PathBuf> {
    ds_settings::cache_dir(AppName::MAILO)
}

#[cfg(test)]
mod tests {
    use super::{Legacy, legacy, load, quire, save};
    use crate::view::{Appearance, Marks, Motion, Theme};
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
    fn a_look_round_trips() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let fresh = dir.path().join("mailo");
        for marks in Marks::ALL {
            let look = Appearance { marks };
            save(&fresh, look).unwrap_or_else(|e| panic!("{look:?}: {e}"));
            assert_eq!(load(&fresh), look, "{look:?}");
            // The rename is the whole of the write: a temp file left beside the real one
            // is a crash that did not finish, and this directory had no other files.
            assert_eq!(entries(&fresh), ["mailo.toml"], "{look:?}");
        }
    }

    #[test]
    fn a_missing_file_is_the_first_run() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), Appearance::default());
        assert_eq!(legacy(dir.path()), Legacy::default());
    }

    #[test]
    fn garbage_bytes_are_the_first_run() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        const CASES: &[(&str, &[u8])] = &[
            ("empty", b""),
            ("prose", b"not json {{{"),
            ("binary", &[0xff, 0xfe, b'{']),
        ];
        for &(name, bytes) in CASES {
            std::fs::write(dir.path().join("appearance.json"), bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(legacy(dir.path()), Legacy::default(), "{name}");
            assert_eq!(load(dir.path()), Appearance::default(), "{name}");
            std::fs::write(dir.path().join("mailo.toml"), bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(load(dir.path()), Appearance::default(), "{name}");
            std::fs::remove_file(dir.path().join("mailo.toml"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
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
                "unknown theme keeps the motion",
                r#"{"theme":"sepia","motion":"calm"}"#,
                Legacy {
                    motion: Motion::Calm,
                    ..Legacy::default()
                },
            ),
            (
                "motion only",
                r#"{"motion":"calm"}"#,
                Legacy {
                    motion: Motion::Calm,
                    ..Legacy::default()
                },
            ),
            (
                "unknown motion keeps the theme",
                r#"{"theme":"dark","motion":"wild"}"#,
                Legacy {
                    theme: Theme::Dark,
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
    fn the_accent_is_dropped_on_read_and_theme_and_motion_are_kept() {
        // The migration table. Every row names a theme or a motion that is not the default,
        // so a loader that threw the file away on meeting `accent` would fail it: dropping
        // the retired field must not drop the fields beside it.
        let cases: &[(&str, &str, Legacy)] = &[
            (
                "an old file with every field",
                r#"{"theme":"dark","accent":"pine","motion":"calm"}"#,
                Legacy {
                    theme: Theme::Dark,
                    motion: Motion::Calm,
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
                r#"{"accent":7,"motion":"extra"}"#,
                Legacy {
                    motion: Motion::Extra,
                    ..Legacy::default()
                },
            ),
            (
                "an unknown field is ignored",
                r#"{"theme":"light","future":true,"motion":"extra","marks":"letters"}"#,
                Legacy {
                    theme: Theme::Light,
                    motion: Motion::Extra,
                    marks: Marks::Letters,
                },
            ),
        ];
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("appearance.json");
        for &(name, bytes, look) in cases {
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(legacy(dir.path()), look, "{name}: {bytes}");
            // The window's own file never carries the retired word, and saving it leaves the
            // JSON exactly as it was.
            save(dir.path(), load(dir.path())).unwrap_or_else(|e| panic!("{name}: {e}"));
            let prefs = std::fs::read_to_string(dir.path().join("mailo.toml"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!prefs.contains("accent"), "{name}: {prefs}");
            let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(raw, bytes, "{name}: the JSON was rewritten");
            std::fs::remove_file(dir.path().join("mailo.toml"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    #[test]
    fn letters_are_written_and_a_file_without_marks_stays_icons() {
        // Saving Icons and reading Icons would pass even if the field were dropped on
        // the floor: the missing word is the default. Letters is the value that proves
        // the field survived, and the old file proves a missing word is Icons rather
        // than a failure of the whole look.
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        let look = Appearance {
            marks: Marks::Letters,
        };
        save(dir.path(), look).unwrap_or_else(|err| panic!("{err}"));
        let raw = std::fs::read_to_string(dir.path().join("mailo.toml"))
            .unwrap_or_else(|err| panic!("{err}"));
        assert!(raw.contains("\"letters\""), "{raw}");
        assert_eq!(load(dir.path()), look);

        let old = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        std::fs::write(
            old.path().join("appearance.json"),
            r#"{"theme":"light","motion":"extra"}"#,
        )
        .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(load(old.path()).marks, Marks::Icons);
    }

    #[test]
    fn the_first_run_after_the_move_takes_the_marks_from_the_json() {
        // Before `mailo.toml` exists the JSON's choice stands; once it exists it is the
        // window's, and the JSON no longer decides.
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        std::fs::write(
            dir.path().join("appearance.json"),
            r#"{"theme":"dark","motion":"calm","marks":"letters"}"#,
        )
        .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(load(dir.path()).marks, Marks::Letters);
        save(
            dir.path(),
            Appearance {
                marks: Marks::Icons,
            },
        )
        .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(load(dir.path()).marks, Marks::Icons);
    }

    #[test]
    fn appearance_json_is_imported_into_appearance_toml_once_and_never_touched() {
        // design/22-SETTINGS.md section 2, "Mailo migration": read once when the TOML file does
        // not exist, mapped field for field, written as `appearance.toml`; the JSON stays as it
        // was, so a downgrade loses nothing. A TempDir, never the real config directory.
        let dir = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        let json = r#"{"theme":"dark","motion":"calm","marks":"letters"}"#;
        std::fs::write(dir.path().join("appearance.json"), json)
            .unwrap_or_else(|err| panic!("{err}"));

        let first = quire(dir.path());
        assert_eq!(first.appearance.theme, ds::Theme::Dark);
        assert_eq!(first.appearance.motion_level, ds::Motion::Calm);
        // mailo retired its accents, so there is nothing to carry: the design default.
        assert_eq!(first.appearance.accent, ds::Accent::Postmark);
        assert_eq!(entries(dir.path()), ["appearance.json", "appearance.toml"]);

        // A second run reads the TOML, not the JSON: an edit to the JSON no longer reaches it.
        std::fs::write(dir.path().join("appearance.json"), r#"{"theme":"light"}"#)
            .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(quire(dir.path()), first);
        let raw = std::fs::read_to_string(dir.path().join("appearance.json"))
            .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(raw, r#"{"theme":"light"}"#, "the JSON file was modified");

        // No JSON at all is the first run, and writes nothing.
        let empty = tempfile::tempdir().unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(quire(empty.path()), ds_settings::AppearanceFile::default());
        assert!(entries(empty.path()).is_empty());
    }

    #[test]
    fn the_directories_are_mailos() {
        // The XDG resolution itself moved to `ds_settings::dirs`, with its tests; what is
        // left to hold here is that mailo asks for its own name.
        for dir in [super::config_dir(), super::state_dir(), super::cache_dir()]
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

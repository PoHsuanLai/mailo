//! Where the window's look is remembered.
//!
//! Config, not mail: it lives in `$XDG_CONFIG_HOME/mailo/appearance.json`
//! (`~/.config/mailo/` when unset), never in the mail database. A cosmetic
//! choice must not be a write to the file that holds someone's mail, or be able
//! to fail a migration.

use crate::view::Appearance;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "appearance.json";

/// The stored look, or the default when there is none or it cannot be read.
/// Not an error the user needs to see: a missing or damaged file means the window
/// looks as it did on first run.
pub fn load(dir: &Path) -> Appearance {
    read_json(dir, FILE_NAME)
}

/// Write `look` to `dir`, creating the directory if needed.
///
/// The bytes land in a temporary file in `dir` and are renamed into place, so a
/// crash mid-write cannot leave a half-written `appearance.json` for the next launch.
pub fn save(dir: &Path, look: Appearance) -> Result<(), String> {
    write_json(dir, FILE_NAME, &look)
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
    config_dir_from(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn config_dir_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("mailo"))
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
    state_dir_from(std::env::var_os("XDG_STATE_HOME"), std::env::var_os("HOME"))
}

fn state_dir_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local/state")))?;
    Some(base.join("mailo"))
}

/// `$XDG_CACHE_HOME/mailo`, else `$HOME/.cache/mailo`, else None.
///
/// Provider icons live here, under `providers/`. They are not config and not the
/// mail database: a missing cache is the letter on the chip, not a lost account.
pub fn cache_dir() -> Option<PathBuf> {
    cache_dir_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}

fn cache_dir_from(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = xdg
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("mailo"))
}

#[cfg(test)]
mod tests {
    use super::{cache_dir_from, config_dir_from, load, save, state_dir_from};
    use crate::view::{Appearance, Marks, Motion, Theme};
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

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
        const CASES: &[Appearance] = &[
            Appearance {
                theme: Theme::System,
                motion: Motion::Standard,
                marks: Marks::Icons,
            },
            Appearance {
                theme: Theme::Dark,
                motion: Motion::Calm,
                marks: Marks::Letters,
            },
            Appearance {
                theme: Theme::Light,
                motion: Motion::Extra,
                marks: Marks::Icons,
            },
        ];
        for &look in CASES {
            save(&fresh, look).unwrap_or_else(|e| panic!("{look:?}: {e}"));
            assert_eq!(load(&fresh), look, "{look:?}");
            // The rename is the whole of the write: a temp file left beside the real one
            // is a crash that did not finish, and this directory had no other files.
            assert_eq!(entries(&fresh), ["appearance.json"], "{look:?}");
        }
    }

    #[test]
    fn a_missing_file_is_the_first_run() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(load(dir.path()), Appearance::default());
    }

    #[test]
    fn garbage_bytes_are_the_first_run() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("appearance.json");
        const CASES: &[(&str, &[u8])] = &[
            ("empty", b""),
            ("prose", b"not json {{{"),
            ("binary", &[0xff, 0xfe, b'{']),
        ];
        for &(name, bytes) in CASES {
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(load(dir.path()), Appearance::default(), "{name}");
        }
    }

    #[test]
    fn a_partial_file_keeps_the_fields_it_has() {
        // `sepia` alone matching the default is not enough: a loader that rejects the whole
        // file also returns the default. The rows that set another field are what show the
        // bad word was dropped on its own.
        let cases: &[(&str, &str, Appearance)] = &[
            (
                "theme only",
                r#"{"theme":"dark"}"#,
                Appearance {
                    theme: Theme::Dark,
                    ..Appearance::default()
                },
            ),
            (
                "unknown theme keeps the motion",
                r#"{"theme":"sepia","motion":"calm"}"#,
                Appearance {
                    motion: Motion::Calm,
                    ..Appearance::default()
                },
            ),
            (
                "motion only",
                r#"{"motion":"calm"}"#,
                Appearance {
                    motion: Motion::Calm,
                    ..Appearance::default()
                },
            ),
            (
                "unknown motion keeps the theme",
                r#"{"theme":"dark","motion":"wild"}"#,
                Appearance {
                    theme: Theme::Dark,
                    ..Appearance::default()
                },
            ),
            (
                "marks letters",
                r#"{"marks":"letters"}"#,
                Appearance {
                    marks: Marks::Letters,
                    ..Appearance::default()
                },
            ),
            (
                "unknown marks keeps the theme",
                r#"{"marks":"crests","theme":"dark"}"#,
                Appearance {
                    theme: Theme::Dark,
                    ..Appearance::default()
                },
            ),
        ];
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("appearance.json");
        for &(name, bytes, look) in cases {
            std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(load(dir.path()), look, "{name}: {bytes}");
        }
    }

    #[test]
    fn the_accent_is_dropped_on_read_and_theme_and_motion_are_kept() {
        // The migration table. Every row names a theme or a motion that is not the default,
        // so a loader that threw the file away on meeting `accent` would fail it: dropping
        // the retired field must not drop the fields beside it.
        let cases: &[(&str, &str, Appearance)] = &[
            (
                "an old file with every field",
                r#"{"theme":"dark","accent":"pine","motion":"calm"}"#,
                Appearance {
                    theme: Theme::Dark,
                    motion: Motion::Calm,
                    marks: Marks::Icons,
                },
            ),
            (
                "an accent this build never knew",
                r#"{"accent":"rose","theme":"light"}"#,
                Appearance {
                    theme: Theme::Light,
                    ..Appearance::default()
                },
            ),
            (
                "an accent that is not a word",
                r#"{"accent":7,"motion":"extra"}"#,
                Appearance {
                    motion: Motion::Extra,
                    ..Appearance::default()
                },
            ),
            (
                "an unknown field is ignored",
                r#"{"theme":"light","future":true,"motion":"extra","marks":"letters"}"#,
                Appearance {
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
            let read = load(dir.path());
            assert_eq!(read, look, "{name}: {bytes}");
            // Written back, the retired word is gone for good.
            save(dir.path(), read).unwrap_or_else(|e| panic!("{name}: {e}"));
            let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!raw.contains("accent"), "{name}: {raw}");
            assert_eq!(load(dir.path()), look, "{name}: after saving {raw}");
        }
    }

    #[test]
    fn the_config_directory_follows_xdg() {
        struct Case {
            name: &'static str,
            xdg: Option<&'static str>,
            home: Option<&'static str>,
            expect: Option<&'static str>,
        }
        const CASES: &[Case] = &[
            Case {
                name: "xdg wins",
                xdg: Some("/xdg"),
                home: Some("/home/ada"),
                expect: Some("/xdg/mailo"),
            },
            Case {
                name: "home when xdg is unset",
                xdg: None,
                home: Some("/home/ada"),
                expect: Some("/home/ada/.config/mailo"),
            },
            Case {
                name: "neither",
                xdg: None,
                home: None,
                expect: None,
            },
        ];
        for case in CASES {
            assert_eq!(
                config_dir_from(case.xdg.map(OsString::from), case.home.map(OsString::from)),
                case.expect.map(PathBuf::from),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn the_state_directory_follows_xdg() {
        struct Case {
            name: &'static str,
            xdg: Option<&'static str>,
            home: Option<&'static str>,
            expect: Option<&'static str>,
        }
        const CASES: &[Case] = &[
            Case {
                name: "xdg wins",
                xdg: Some("/xdg"),
                home: Some("/home/ada"),
                expect: Some("/xdg/mailo"),
            },
            Case {
                name: "home when xdg is unset",
                xdg: None,
                home: Some("/home/ada"),
                expect: Some("/home/ada/.local/state/mailo"),
            },
            Case {
                name: "neither",
                xdg: None,
                home: None,
                expect: None,
            },
        ];
        for case in CASES {
            assert_eq!(
                state_dir_from(case.xdg.map(OsString::from), case.home.map(OsString::from)),
                case.expect.map(PathBuf::from),
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn the_cache_directory_follows_xdg() {
        struct Case {
            name: &'static str,
            xdg: Option<&'static str>,
            home: Option<&'static str>,
            expect: Option<&'static str>,
        }
        const CASES: &[Case] = &[
            Case {
                name: "xdg wins",
                xdg: Some("/xdg"),
                home: Some("/home/ada"),
                expect: Some("/xdg/mailo"),
            },
            Case {
                name: "home when xdg is unset",
                xdg: None,
                home: Some("/home/ada"),
                expect: Some("/home/ada/.cache/mailo"),
            },
            Case {
                name: "neither",
                xdg: None,
                home: None,
                expect: None,
            },
        ];
        for case in CASES {
            assert_eq!(
                cache_dir_from(case.xdg.map(OsString::from), case.home.map(OsString::from)),
                case.expect.map(PathBuf::from),
                "{}",
                case.name
            );
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
            theme: Theme::Dark,
            marks: Marks::Letters,
            ..Appearance::default()
        };
        save(dir.path(), look).unwrap_or_else(|err| panic!("{err}"));
        let raw = std::fs::read_to_string(dir.path().join("appearance.json"))
            .unwrap_or_else(|err| panic!("{err}"));
        assert!(raw.contains("\"letters\""), "{raw}");
        assert_eq!(load(dir.path()), look);

        std::fs::write(
            dir.path().join("appearance.json"),
            r#"{"theme":"light","motion":"extra"}"#,
        )
        .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(
            load(dir.path()),
            Appearance {
                theme: Theme::Light,
                motion: Motion::Extra,
                marks: Marks::Icons,
            }
        );
    }
}

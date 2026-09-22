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
    let Ok(bytes) = std::fs::read(dir.join(FILE_NAME)) else {
        return Appearance::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Write `look` to `dir`, creating the directory if needed.
///
/// The bytes land in a temporary file in `dir` and are renamed into place, so a
/// crash mid-write cannot leave a half-written `appearance.json` for the next launch.
pub fn save(dir: &Path, look: Appearance) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(FILE_NAME);
    let tmp = path.with_extension("part");
    let mut body = serde_json::to_vec(&look).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::{config_dir_from, load, save};
    use crate::view::{Accent, Appearance, Theme};
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
                accent: Accent::Postmark,
            },
            Appearance {
                theme: Theme::Dark,
                accent: Accent::Pine,
            },
            Appearance {
                theme: Theme::Light,
                accent: Accent::Oxblood,
            },
            Appearance {
                theme: Theme::System,
                accent: Accent::Vermilion,
            },
            Appearance {
                theme: Theme::Dark,
                accent: Accent::Graphite,
            },
            Appearance {
                theme: Theme::Light,
                accent: Accent::Indigo,
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
        // `rose` alone matching the default is not enough: a loader that rejects the whole
        // file also returns the default. The rows that set the other field are what show
        // the bad word was dropped on its own.
        let cases: &[(&str, &str, Appearance)] = &[
            (
                "unknown accent",
                r#"{"accent":"rose"}"#,
                Appearance::default(),
            ),
            (
                "theme only",
                r#"{"theme":"dark"}"#,
                Appearance {
                    theme: Theme::Dark,
                    accent: Accent::default(),
                },
            ),
            (
                "unknown accent keeps the theme",
                r#"{"accent":"rose","theme":"dark"}"#,
                Appearance {
                    theme: Theme::Dark,
                    accent: Accent::default(),
                },
            ),
            (
                "unknown theme keeps the accent",
                r#"{"theme":"sepia","accent":"pine"}"#,
                Appearance {
                    theme: Theme::default(),
                    accent: Accent::Pine,
                },
            ),
            (
                "an extra field is ignored",
                r#"{"theme":"light","accent":"indigo","future":true}"#,
                Appearance {
                    theme: Theme::Light,
                    accent: Accent::Indigo,
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
}

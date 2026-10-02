//! Spelling in the composer: whether it is checked, and whether there is anything to check it
//! with.
//!
//! quire checks, marks and offers suggestions (`ds::EditSurface`'s `spell`, the `spellcheck`
//! feature of `ds-native`), against the system's Hunspell dictionaries; mailo bundles none. What
//! is mailo's is the user's choice to have it at all, kept like the notifications switch in the
//! config directory (`spelling.json`), and saying plainly when no dictionary was found, which is
//! the usual case on macOS and Windows, where there are no system Hunspell directories.

use ds::spell::lang::Lang;
use std::path::Path;

/// Whether the composer marks misspelt words. On unless someone turned it off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Setting {
    #[default]
    On,
    Off,
}

const FILE_NAME: &str = "spelling.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    spelling: Setting,
}

/// The stored setting, or on when there is none or it cannot be read.
pub fn load(dir: &Path) -> Setting {
    mail_core::config::read_json::<Stored>(dir, FILE_NAME).spelling
}

/// Remember `setting` in `dir`.
pub fn save(dir: &Path, setting: Setting) -> Result<(), String> {
    mail_core::config::write_json(dir, FILE_NAME, &Stored { spelling: setting })
}

/// What the checker has to work with, for the language drafts are checked in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dictionaries {
    /// A dictionary answers for the language: this one, by its file name (`en_US`).
    Ready(String),
    /// No dictionary is installed in any directory the checker reads.
    NoneInstalled,
    /// Some are, but none for the language drafts are checked in.
    NoneFor {
        language: String,
        installed: Vec<String>,
    },
    /// The system names no language (`LANG` unset, `C` or `POSIX`), so nothing is checked.
    NoLanguage,
}

/// What checking a draft in `language` (the locale's) would find among `installed`.
pub fn dictionaries(language: Option<&Lang>, installed: &[Lang]) -> Dictionaries {
    if installed.is_empty() {
        return Dictionaries::NoneInstalled;
    }
    let Some(language) = language else {
        return Dictionaries::NoLanguage;
    };
    match ds_blitz::spell::pick(language, installed) {
        Some(found) => Dictionaries::Ready(found.name().to_owned()),
        None => Dictionaries::NoneFor {
            language: language.name().to_owned(),
            installed: installed
                .iter()
                .map(|lang| lang.name().to_owned())
                .collect(),
        },
    }
}

impl Dictionaries {
    /// A sentence for the settings, when there is something the user should know: why nothing
    /// is marked.
    pub fn missing(&self) -> Option<String> {
        match self {
            Dictionaries::Ready(_) => None,
            Dictionaries::NoneInstalled => Some(
                "No spelling dictionaries were found, so nothing is marked. mailo uses the \
                 system's Hunspell dictionaries (a package such as hunspell-en-us) and ships \
                 none of its own."
                    .to_owned(),
            ),
            Dictionaries::NoneFor {
                language,
                installed,
            } => Some(format!(
                "There is no spelling dictionary for {language}, so nothing is marked. \
                 Installed: {}.",
                installed.join(", ")
            )),
            Dictionaries::NoLanguage => Some(
                "The system names no language (LANG), so there is nothing to check spelling \
                 against."
                    .to_owned(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Dictionaries, Setting, dictionaries, load, save};
    use ds::spell::lang::Lang;

    fn langs(names: &[&str]) -> Vec<Lang> {
        names
            .iter()
            .map(|name| Lang::parse(name).unwrap_or_else(|why| panic!("{name}: {why}")))
            .collect()
    }

    #[test]
    fn what_a_language_finds_among_the_installed_dictionaries() {
        let ready = |name: &str| Dictionaries::Ready(name.to_owned());
        let cases: &[(Option<&str>, &[&str], Dictionaries)] = &[
            (Some("en_US"), &[], Dictionaries::NoneInstalled),
            (None, &[], Dictionaries::NoneInstalled),
            (None, &["en_US"], Dictionaries::NoLanguage),
            (Some("en_US"), &["en_US"], ready("en_US")),
            (Some("en_GB"), &["de_DE", "en_US"], ready("en_US")),
            (
                Some("fr_FR"),
                &["de_DE", "en_US"],
                Dictionaries::NoneFor {
                    language: "fr_FR".to_owned(),
                    installed: vec!["de_DE".to_owned(), "en_US".to_owned()],
                },
            ),
        ];
        for (language, installed, expected) in cases {
            let language = language.map(|name| langs(&[name]).remove(0));
            let found = dictionaries(language.as_ref(), &langs(installed));
            assert_eq!(&found, expected, "{language:?} among {installed:?}");
            assert_eq!(
                found.missing().is_none(),
                matches!(expected, Dictionaries::Ready(_)),
                "{found:?} says why nothing is marked exactly when nothing can be"
            );
        }
    }

    #[test]
    fn the_setting_is_on_until_turned_off_and_kept() {
        let dir = tempfile::tempdir().unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(load(dir.path()), Setting::On, "on unless turned off");
        save(dir.path(), Setting::Off).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(load(dir.path()), Setting::Off);
        save(dir.path(), Setting::On).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(load(dir.path()), Setting::On);
        std::fs::write(dir.path().join("spelling.json"), b"not json")
            .unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(load(dir.path()), Setting::On, "an unreadable file is on");
    }
}

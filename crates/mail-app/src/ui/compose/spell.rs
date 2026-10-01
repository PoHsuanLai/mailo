//! Spelling in the body: which checker the window's composer asks, whether it asks at all, and
//! what the settings say about the dictionaries.
//!
//! quire does the checking, the marks and the menu (`EditSurface`'s `spell`, `caret` and
//! `on_replace`); `surface.rs` hands it those. The checker is the `SpellService` `ds_blitz`
//! provides, over the system's Hunspell dictionaries in the locale's language. A test hands the
//! window [`Dictionaries`] instead, and the window checks against those, so no test reads the
//! system's dictionaries or writes the user's learned words.

use dioxus::prelude::*;
use ds::spell::lang::{Lang, Spell};
use ds_blitz::spell::{SpellConfig, locale_lang};

use crate::ui::spelling::{self, Setting};

/// Dictionaries to check against in place of the system's, and the languages a draft is
/// checked in: a root context, for a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dictionaries {
    pub config: SpellConfig,
    pub languages: Vec<Lang>,
}

/// Provide the checker over [`Dictionaries`] when the window was handed them. Called once, at
/// the top of the app; without them the checker `ds_blitz` provides at launch stands.
pub(in crate::ui) fn use_test_dictionaries() {
    if let Some(Dictionaries { config, languages }) = try_consume_context::<Dictionaries>() {
        ds_blitz::spell::provide_with(config, languages);
    }
}

/// The surface's `spell` for the user's setting: in the checker's own language (the locale's),
/// since a draft carries no language of its own.
pub(super) fn spell_of(setting: Setting) -> Spell {
    match setting {
        Setting::On => Spell::On { lang: None },
        Setting::Off => Spell::Off,
    }
}

/// What the checker the window uses has to work with. Reads the dictionary directories, so it
/// is asked once, when the settings open.
pub(in crate::ui) fn dictionaries() -> spelling::Dictionaries {
    let (config, language) = match try_consume_context::<Dictionaries>() {
        Some(Dictionaries { config, languages }) => (config, languages.into_iter().next()),
        None => (
            SpellConfig::system(),
            locale_lang(
                std::env::var("LC_ALL").ok().as_deref(),
                std::env::var("LANG").ok().as_deref(),
            ),
        ),
    };
    spelling::dictionaries(language.as_ref(), &config.installed())
}

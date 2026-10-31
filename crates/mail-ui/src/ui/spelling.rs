//! Spelling in the composer: whether it is checked.
//!
//! quire checks, marks and offers suggestions (`ds::EditSurface`'s `spell`, the `spellcheck`
//! feature of `ds-native`), against the system's Hunspell dictionaries; mailo bundles none. What
//! is mailo's is the user's choice to have it at all (`compose.spelling` in `mailo/settings.toml`,
//! `mail_ui_model::settings`).

/// Whether the composer marks misspelt words. On unless someone turned it off.
pub use mail_ui_model::settings::Spelling as Setting;

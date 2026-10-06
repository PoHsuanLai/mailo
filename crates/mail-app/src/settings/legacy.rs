//! What the per-switch files said before `settings.toml`: read once, to start the new file from
//! them, and never written.
//!
//! `mailo.toml` held the provider marks (and before it `appearance.json`), and `notify.json`,
//! `spelling.json`, `bimi.json` and `server-search.json` one switch each. Each is read as leniently
//! as it always was: a missing or damaged file is that switch's default.

use super::{BrandLogos, MailSettings, NewMail, ProviderMarks, ServerSearch, Spelling};
use mail_core::config::read_json;
use serde::Deserialize;
use std::path::Path;

/// `mailo.toml` and `appearance.json`: the one key either is read for.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Marked {
    marks: Option<ProviderMarks>,
}

/// `spelling.json`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Spelt {
    spelling: Spelling,
}

fn marks(dir: &Path) -> ProviderMarks {
    let from_toml = std::fs::read_to_string(dir.join("mailo.toml"))
        .ok()
        .and_then(|text| toml::from_str::<Marked>(&text).ok())
        .and_then(|marked| marked.marks);
    from_toml
        .or_else(|| read_json::<Marked>(dir, "appearance.json").marks)
        .unwrap_or_default()
}

/// The settings the old files in `dir` held.
pub(super) fn read(dir: &Path) -> MailSettings {
    let mut settings = MailSettings::default();
    settings.window.provider_marks = marks(dir);
    settings.notifications.new_mail = NewMail::from(mail_core::notify::load(dir));
    settings.compose.spelling = read_json::<Spelt>(dir, "spelling.json").spelling;
    settings.reading.brand_logos = BrandLogos::from(mail_core::bimi::load(dir));
    settings.search.server_automatically = ServerSearch::from(mail_core::server_search::load(dir));
    settings
}

//! Download All Mail: the window's half of `mailo offline <account> on|off`, a switch on the
//! account's page, with how much of the account is here under it.
//!
//! A choice is kept at once, in `offline.json`, where every sync reads it (a key per account, so
//! not a schema key). The sync does the fetching; this only says whether it should. The count is
//! read once, when the page opens.

use crate::ui::appearance::WindowDirs;
use crate::ui::copy;
use crate::ui::data::AccountRow;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::*;
use mail_core::offline::{self, Keep};
use mail_core::{SqliteStore, Store as _};
use std::sync::Arc;

/// The account's switch. Mail kept only here has no server to keep a copy of, so its page has
/// none (the caller's to know).
#[component]
pub(super) fn DownloadAll(row: AccountRow) -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let id = row.id.clone();
    // What the store counted, said as a sentence; or why it could not count.
    let said = use_hook({
        let id = id.clone();
        move || {
            try_consume_context::<Arc<SqliteStore>>().map(|store| {
                store
                    .offline(id)
                    .map(|counted| offline::said(&counted))
                    .map_err(|e| e.to_string())
            })
        }
    });
    let mut kept = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(|dirs| offline::load(&dirs.config))
                .unwrap_or_default()
        }
    });
    let mut failed = use_signal(|| None::<String>);
    let current = kept.read().of(id.clone());
    // A choice that could not be kept is said where the count is.
    let help = match (failed(), said) {
        (Some(why), _) => Some(why),
        (None, Some(Ok(said))) => Some(format!("{said}.")),
        (None, Some(Err(why))) => Some(copy::errors::could_not_count(&why)),
        (None, None) => None,
    };
    rsx! {
        FieldRow {
            label: copy::fields::DOWNLOAD_ALL_MAIL,
            help: help.map(TextLine::from),
            Toggle {
                label: copy::fields::DOWNLOAD_ALL_MAIL,
                value: if current == Keep::Everything { Check::On } else { Check::Off },
                onchange: move |now: Check| {
                    let keep = if now == Check::On { Keep::Everything } else { Keep::Bodies };
                    let saved = match &dirs {
                        Some(dirs) => offline::save(&dirs.config, id.clone(), keep).map_err(String::from),
                        None => Err(copy::errors::NO_CONFIG_DIRECTORY.to_owned()),
                    };
                    match saved {
                        Ok(()) => {
                            let now = kept.peek().clone().with(id.clone(), keep);
                            kept.set(now);
                            failed.set(None);
                        }
                        Err(why) => failed.set(Some(why)),
                    }
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "offline_tests.rs"]
mod tests;

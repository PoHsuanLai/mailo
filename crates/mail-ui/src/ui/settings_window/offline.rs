//! Download All Mail: the window's half of `mailo offline <account> on|off`, a switch on the
//! account's page.
//!
//! A choice is kept at once, in `offline.json`, where every sync reads it (a key per account, so
//! not a schema key). The sync does the fetching; this only says whether it should. A choice that
//! cannot be written is said in a dialog, and the switch stays as it was.

use crate::ui::appearance::WindowDirs;
use crate::ui::common::Problem;
use crate::ui::copy;
use crate::ui::data::AccountRow;
use dioxus::prelude::*;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::*;
use mail_client::offline::{self, Keep};

/// The account's switch. Mail kept only here has no server to keep a copy of, so its page has
/// none (the caller's to know).
#[component]
pub(super) fn DownloadAll(row: AccountRow) -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let id = row.id.clone();
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
    rsx! {
        FieldRow {
            label: copy::fields::DOWNLOAD_ALL_MAIL,
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
        if let Some(why) = failed() {
            Problem { title: copy::titles::NOT_SAVED, why, onclose: move |()| failed.set(None) }
        }
    }
}

#[cfg(test)]
#[path = "offline_tests.rs"]
mod tests;

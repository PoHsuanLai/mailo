//! Keep all mail offline: the window's half of `mailo offline <account> on|off`, per account,
//! with how much of each account is here.
//!
//! Every Space's, so it is on Settings' Accounts page, and a choice is kept at once — in
//! `offline.json`, where every sync reads it (a key per account, so not a schema key). The sync does the fetching; this only says whether
//! it should. The counts are read once, when the sheet opens.

use crate::ui::appearance::WindowDirs;
use crate::ui::data::{AccountRow, account_rows};
use crate::ui::space_menu::Seg;
use dioxus::prelude::*;
use mail_core::offline::{self, Keep, Kept};
use mail_domain::Incoming;
use mail_store::{SqliteStore, Store as _};
use std::sync::Arc;

const CHOICES: [(Keep, &str); 2] = [(Keep::Everything, "On"), (Keep::Bodies, "Off")];

/// One account's line: who, and how much of it is here.
#[derive(Clone, PartialEq)]
struct Line {
    row: AccountRow,
    /// What the store counted, said as a sentence; or why it could not count.
    said: Result<String, String>,
}

fn lines(store: &SqliteStore) -> Vec<Line> {
    account_rows(store)
        .into_iter()
        // Mail kept only here has no server to keep a copy of.
        .filter(|row| !matches!(row.plan.incoming, Incoming::Local))
        .map(|row| Line {
            said: store
                .offline(row.id.clone())
                .map(|counted| offline::said(&counted))
                .map_err(|e| e.to_string()),
            row,
        })
        .collect()
}

#[component]
pub(super) fn OfflineCopy() -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let accounts = use_hook(|| {
        try_consume_context::<Arc<SqliteStore>>()
            .map(|store| lines(&store))
            .unwrap_or_default()
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
    if accounts.is_empty() {
        return rsx! {};
    }
    rsx! {
        div {
            div { class: "ed-label", "Keep all mail offline" }
            for line in accounts {
                OneAccount {
                    key: "{line.row.id}",
                    line: line.clone(),
                    kept: kept(),
                    on_keep: {
                        let dirs = dirs.clone();
                        let id = line.row.id;
                        move |keep: Keep| {
                            let saved = match &dirs {
                                Some(dirs) => offline::save(&dirs.config, id.clone(), keep),
                                None => Err("There is no config directory to keep this in.".to_owned()),
                            };
                            match saved {
                                Ok(()) => {
                                    let now = kept.peek().clone().with(id.clone(), keep);
                                    kept.set(now);
                                    failed.set(None);
                                }
                                Err(why) => failed.set(Some(why)),
                            }
                        }
                    },
                }
            }
            p { class: "capnote",
                "On, every sync also fetches the attachments it leaves on the server until opened, a few at a time and the largest last."
            }
            if let Some(why) = failed() {
                p { class: "capnote", "{why}" }
            }
        }
    }
}

#[component]
fn OneAccount(line: Line, kept: Kept, on_keep: EventHandler<Keep>) -> Element {
    let current = kept.of(line.row.id);
    let address = line.row.address.clone();
    let said = match &line.said {
        Ok(said) => said.clone(),
        Err(why) => format!("Could not count: {why}"),
    };
    rsx! {
        div {
            p { class: "capnote", "{address}" }
            Seg {
                label: format!("Keep all mail offline for {address}"),
                options: CHOICES
                    .iter()
                    .map(|(keep, name)| ((*name).to_owned(), *keep == current))
                    .collect::<Vec<_>>(),
                on_pick: move |index: usize| on_keep.call(CHOICES[index % CHOICES.len()].0),
            }
            p { class: "capnote", "{said}" }
        }
    }
}

#[cfg(test)]
#[path = "offline_tests.rs"]
mod tests;

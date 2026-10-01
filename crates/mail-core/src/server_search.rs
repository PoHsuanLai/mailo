//! Searching the server from a search typed here: which accounts can, whether it runs by itself,
//! and the typed line as the filter a server is asked for.
//!
//! A search runs over the store. When it is shown, the list's end offers to search each account
//! in view on its server (`ui::server_search`), which finds what the store never held. It is
//! asked, not automatic, unless the user turned "Search the server automatically" on: a server
//! search is a connection and a query against someone else's machine, and a keystroke should
//! not make one. The switch is the window's, kept like the other switches in the config
//! directory (`server-search.json`), and off unless turned on.
//!
//! What a server is asked is the line exactly as the store is asked it, parsed by the same
//! [`crate::query::parse_with`], except that `label:` resolves against the one account being
//! searched — a label is a name on one server — and a `re:/…/` pattern, which no server
//! matches, is said rather than dropped. Each protocol's translation is `mail_proto::search`'s;
//! the engines are `mail_runtime`'s ([`crate::sync::search_server`]).

use crate::config::{read_json, write_json};
use chrono::{DateTime, Utc};
use mail_domain::{AccountId, AccountPlan, Filter, Incoming, LabelId};
use mail_runtime::{Searched, Unsaid};
use mail_store::SqliteStore;
use std::path::Path;
use std::sync::Arc;

/// Whether a search shown in the list is asked of the server without the button being pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Automatic {
    On,
    /// Only when "Search … on the server" is pressed.
    #[default]
    Off,
}

const FILE_NAME: &str = "server-search.json";

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    automatic: Automatic,
}

/// The stored setting, or off when there is none or it cannot be read.
pub fn load(dir: &Path) -> Automatic {
    read_json::<Stored>(dir, FILE_NAME).automatic
}

/// Remember `automatic` in `dir`.
pub fn save(dir: &Path, automatic: Automatic) -> Result<(), String> {
    write_json(dir, FILE_NAME, &Stored { automatic })
}

/// Whether an account has a server to search: IMAP, JMAP, or Microsoft Graph. POP3 has one
/// mailbox and no search, and local folders have no server.
pub fn searchable(plan: &AccountPlan) -> bool {
    matches!(
        plan.incoming,
        Incoming::Imap { .. } | Incoming::Jmap { .. } | Incoming::Graph
    )
}

/// The search line `input` as a server is asked it, on an account whose labels are `labels`.
///
/// `Err` when a part of it no server can match: a `re:/…/` pattern, which is matched here over
/// what the store holds. An invalid pattern is that too: it matches nothing anywhere.
pub fn filter_of(input: &str, labels: &[(String, LabelId)]) -> Result<Filter, Unsaid> {
    let extracted = crate::search::extract(input).map_err(|why| Unsaid(vec![why]))?;
    if let Some(pattern) = &extracted.regex {
        return Err(Unsaid(vec![format!(
            "re:/{}/ (a pattern is matched on this computer, not by a server)",
            pattern.as_str()
        )]));
    }
    Ok(crate::query::parse_with(
        &extracted.rest,
        &chrono::Local,
        &crate::query::named(labels),
    ))
}

/// Search `account`'s server for the line `input`, and keep what it finds.
///
/// Blocking, with a runtime of its own, like a sync: the window calls it off the drawing thread.
pub fn search(
    store: Arc<SqliteStore>,
    account: AccountId,
    input: &str,
    now: DateTime<Utc>,
) -> Result<Searched, String> {
    let named: Vec<(String, LabelId)> = store
        .labels(account)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|l| (l.name, l.id))
        .collect();
    let filter = match filter_of(input, &named) {
        Ok(filter) => filter,
        Err(unsaid) => return Ok(Searched::Unsaid(unsaid)),
    };
    let labels: Vec<(LabelId, String)> = named.into_iter().map(|(n, id)| (id, n)).collect();
    crate::sync::search_server(&store, account, &filter, &labels, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::TextMatch;

    #[test]
    fn off_until_turned_on() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Automatic::Off);
        save(dir.path(), Automatic::On).unwrap();
        assert_eq!(load(dir.path()), Automatic::On);
    }

    #[test]
    fn the_line_is_parsed_as_the_store_parses_it_and_a_pattern_is_said() {
        let travel = LabelId::generate();
        let labels = [("travel".to_owned(), travel)];
        let cases: Vec<(&str, Result<Filter, ()>)> = vec![
            (
                "from:ada",
                Ok(Filter::From(TextMatch::Contains("ada".to_owned()))),
            ),
            ("label:travel", Ok(Filter::HasLabel(travel))),
            (
                "label:work",
                Ok(Filter::Text(TextMatch::Contains("label:work".to_owned()))),
            ),
            ("re:/lun.h/ friday", Err(())),
        ];
        for (input, expected) in cases {
            assert_eq!(
                filter_of(input, &labels).map_err(|_| ()),
                expected,
                "{input}"
            );
        }
        let said = filter_of("re:/lun.h/", &labels).unwrap_err();
        assert_eq!(
            said.0,
            vec!["re:/lun.h/ (a pattern is matched on this computer, not by a server)".to_owned()]
        );
    }
}

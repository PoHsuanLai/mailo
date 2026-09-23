//! `mailo search`: the top results, marked, then every other match, newest first.
//!
//! The same [`crate::search::search_list`] the window's list box runs, so `from:ada`, a prefix
//! and a phrase cannot mean one thing here and another there, and the top results are the
//! strip the window draws above its rows.

use super::render_list;
use chrono::{DateTime, Utc};
use mail_domain::{LabelId, ThreadSummary};
use mail_store::SqliteStore;

/// The column a top result is marked in. Every other line has as many spaces there, so the
/// rows stay aligned.
const TOP: &str = "top ";
const NOT_TOP: &str = "    ";

/// Search every account for `needle`, at most `limit` rows under the top results.
///
/// `label` resolves `label:`, which is the one operator that needs the store. An invalid
/// `re:/pattern/` is the `regex` crate's own message, printed rather than failed on.
pub(super) fn search(
    store: &SqliteStore,
    needle: &str,
    limit: u32,
    label: &dyn Fn(&str) -> Vec<LabelId>,
    now: DateTime<Utc>,
) -> String {
    let take = usize::try_from(limit).unwrap_or(usize::MAX);
    // Asked for the top results' worth more, so removing them still leaves `limit` rows.
    let page = crate::search::first(take.saturating_add(crate::search::STRIP));
    let searched = match crate::search::search_list(
        needle,
        store,
        &crate::search::Affinity::default(),
        &chrono::Local,
        label,
        page,
        now,
    ) {
        Ok(searched) => searched,
        Err(message) => return format!("{message}\n"),
    };
    let top: Vec<ThreadSummary> = searched
        .top
        .into_iter()
        .map(|(summary, _)| summary)
        .collect();
    let rest: Vec<ThreadSummary> = searched
        .rows
        .into_iter()
        .filter(|row| !top.iter().any(|hit| hit.id == row.id))
        .take(take)
        .collect();
    if top.is_empty() && rest.is_empty() {
        return format!("nothing matches {needle:?}\n");
    }
    let mut out = String::new();
    for line in render_list(&top).lines() {
        out.push_str(TOP);
        out.push_str(line);
        out.push('\n');
    }
    for line in render_list(&rest).lines() {
        out.push_str(NOT_TOP);
        out.push_str(line);
        out.push('\n');
    }
    out
}

//! Which accounts a press of Sync, and the line under the list's title, are about.

use mail_ui_kit::view::Shell;
use porter_core::AccountId;
use std::collections::BTreeSet;

/// The accounts in the Space's `scope` that have something to fetch.
///
/// Only accounts in `with_links` can be fetched, which leaves out the one that keeps its mail
/// on this computer.
pub(super) fn in_scope(
    with_links: &BTreeSet<AccountId>,
    scope: &mail_ui_kit::space::Scope,
) -> Vec<AccountId> {
    with_links
        .iter()
        .filter(|&id| scope.shows(id.clone()))
        .cloned()
        .collect()
}

/// [`in_scope`] for what the window is showing.
pub(super) fn in_view(with_links: &BTreeSet<AccountId>, shell: &Shell) -> Vec<AccountId> {
    in_scope(with_links, &shell.scope)
}

#[cfg(test)]
mod tests;

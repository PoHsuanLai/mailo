//! Which accounts a press of Sync, and the line under the list's title, are about.

use crate::view::Shell;
use mail_domain::AccountId;
use std::collections::BTreeSet;

/// The accounts in view that have something to fetch, from the pressed tile or the Space.
///
/// A pressed tile is that one account; otherwise the Space's `scope`, empty meaning every
/// account. Only accounts in `with_links` can be fetched, which leaves out the one that keeps
/// its mail on this computer.
pub(super) fn in_scope(
    with_links: &BTreeSet<AccountId>,
    pressed: Option<AccountId>,
    scope: &[AccountId],
) -> Vec<AccountId> {
    with_links
        .iter()
        .copied()
        .filter(|id| match pressed {
            Some(one) => *id == one,
            None => scope.is_empty() || scope.contains(id),
        })
        .collect()
}

/// [`in_scope`] for what the window is showing.
pub(super) fn in_view(with_links: &BTreeSet<AccountId>, shell: &Shell) -> Vec<AccountId> {
    in_scope(with_links, shell.account, &shell.scope)
}

#[cfg(test)]
mod tests;

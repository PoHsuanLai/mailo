//! Which accounts a press of Sync, and the line under the list's title, are about.

use crate::ui::view::Shell;
use mail_domain::AccountId;
use std::collections::BTreeSet;

/// The accounts in view that have something to fetch, from the pressed tile or the Space.
///
/// A pressed tile is that one account; otherwise the Space's `scope`. Only accounts in `with_links` can be fetched, which leaves out the one that keeps
/// its mail on this computer.
pub(super) fn in_scope(
    with_links: &BTreeSet<AccountId>,
    pressed: Option<AccountId>,
    scope: &crate::ui::space::Scope,
) -> Vec<AccountId> {
    let scope = scope.narrowed(pressed);
    with_links
        .iter()
        .copied()
        .filter(|id| scope.shows(*id))
        .collect()
}

/// [`in_scope`] for what the window is showing.
pub(super) fn in_view(with_links: &BTreeSet<AccountId>, shell: &Shell) -> Vec<AccountId> {
    in_scope(with_links, shell.account, &shell.scope)
}

#[cfg(test)]
mod tests;

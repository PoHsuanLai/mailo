use crate::ui::view::Listing;
use mail_core::{SqliteStore, Store, Syncing};
use mail_domain::*;
use porter_core::AccountId;

/// How many rows the list pane asks for at a time.
pub(super) const PAGE: u32 = 100;

/// The conversations a listing asks for.
///
/// Shared by the blocking task and the first frame's fallback, for the reason `count_badges` is:
/// two copies would be two chances to disagree about what the list contains.
pub(super) fn list_for(
    store: &SqliteStore,
    listing: Listing,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ThreadSummary> {
    let rows = |query: &Query| {
        store
            .threads(query, now)
            .map(|page| page.items)
            .unwrap_or_default()
    };
    match listing {
        Listing::Threads(query) => rows(&query),
        Listing::Messages(query) => singles_for(store, &query, now)
            .into_iter()
            .map(|single| single.summary)
            .collect(),
        Listing::Inbox { query, scope } => mail_core::follow_up::on_top(
            mail_core::follow_up::returned(store, scope.as_ref(), now),
            rows(&query),
        ),
        Listing::Waiting { scope } => mail_core::follow_up::waiting(store, scope.as_ref(), now),
        Listing::History { scope } => mail_core::history::listed(store, scope.as_ref(), now),
        Listing::Drafts | Listing::Scheduled => Vec::new(),
    }
}

/// The messages a place kept a message to a row lists ([`mail_core::single::listed`]), each
/// with the conversation it is a row of. A store that cannot answer is an empty list, said in
/// the log, as a conversation list's failure is an empty one.
pub(super) fn singles_for(
    store: &SqliteStore,
    query: &Query,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<mail_core::single::Single> {
    match mail_core::single::listed(store, query, now) {
        Ok(page) => page.items,
        Err(why) => {
            log::warn!("the list of single messages could not be read: {why}");
            Vec::new()
        }
    }
}

/// One count per place, for the sidebar's badges.
///
/// A free function rather than a closure, because it is called from two places: once on a
/// blocking thread, and once on the render thread for the first frame, when there is no answer
/// yet. Two copies of it would be two chances for them to disagree about what a badge counts.
pub(super) fn count_badges(store: &SqliteStore, filters: &[Option<Filter>]) -> Vec<Option<u64>> {
    let now = chrono::Utc::now();
    filters
        .iter()
        .map(|filter| match store.count(filter.as_ref()?, now) {
            Ok(0) => None,
            Ok(n) => Some(n),
            // No badge rather than a wrong one, but said: a count that fails every time is a
            // store to look at, not a place with nothing unread.
            Err(why) => {
                log::warn!("a sidebar count failed: {why}");
                None
            }
        })
        .collect()
}

/// One account as the sidebar draws it: the id, the address, and the plan the provider comes from.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AccountRow {
    pub id: AccountId,
    pub address: String,
    pub plan: AccountPlan,
    /// What the person calls it, written in place of the address where the window names it.
    pub description: Option<String>,
    /// Whether the person paused its syncing.
    pub syncing: Syncing,
}

/// Every account, oldest first. A plan that does not parse becomes a plain IMAP account so the
/// account still has a host to name; `'{}'` is what the oldest fixtures wrote.
pub(super) fn account_rows(store: &SqliteStore) -> Vec<AccountRow> {
    store
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .map(|account| {
            let plan = account
                .plan
                .unwrap_or_else(|_| fallback_plan(&account.address));
            // An account whose preferences cannot be read is shown as it was before there were
            // any: by its address, syncing.
            let prefs = store.account_prefs(account.id.clone()).unwrap_or_default();
            AccountRow {
                id: account.id,
                address: account.address,
                plan,
                description: prefs.description,
                syncing: prefs.syncing,
            }
        })
        .collect()
}

impl AccountRow {
    /// Whether this is local folders: mail kept on this computer, never synced, never sent from.
    /// The same test as [`mail_core::sync::local_accounts`].
    pub(in crate::ui) fn is_local(&self) -> bool {
        matches!(self.plan.incoming, Incoming::Local)
    }

    /// The account's name as the window writes it: the description the person gave it, else
    /// its address, or "Local folders".
    pub(in crate::ui) fn shown(&self) -> String {
        if let Some(description) = &self.description {
            description.clone()
        } else if self.is_local() {
            LOCAL_FOLDERS.to_owned()
        } else {
            self.address.clone()
        }
    }

    /// Whether the person paused this account's syncing ([`mail_core::sync::paused`]).
    pub(in crate::ui) fn paused(&self) -> bool {
        self.syncing == Syncing::Paused
    }
}

/// Whether nothing the list shows has a server, so there is nothing for a sync to do: every
/// account in the Space's `scope` is local folders, or there is no account at all. The sidebar then has no sync mark and
/// the list's ⋯ no Sync Now.
pub(super) fn syncs_nothing(rows: &[AccountRow], scope: &crate::ui::space::Scope) -> bool {
    rows.iter()
        .filter(|row| scope.shows(row.id.clone()))
        .all(|row| row.is_local())
}

/// What the window calls the local-only account, wherever it names it.
pub(super) const LOCAL_FOLDERS: &str = "Local folders";

fn fallback_plan(address: &str) -> AccountPlan {
    AccountPlan {
        address: address.to_owned(),
        incoming: Incoming::Imap {
            host: "imap.example".to_owned(),
            port: 993,
            tls: Tls::Implicit,
        },
        outgoing: Outgoing::Smtp {
            host: "smtp.example".to_owned(),
            port: 465,
            tls: Tls::Implicit,
        },
        auth: AuthPlan::Password {
            username: Username::SameAsAddress,
            sasl: vec![SaslMech::Plain],
        },
        identities: Vec::new(),
    }
}

/// Every configured account, or why the store could not say. Where an empty answer would be
/// acted on (taking accounts out of the Spaces), a failed read must not look like no accounts.
pub(super) fn known_accounts(store: &SqliteStore) -> Result<Vec<AccountId>, mail_core::StoreError> {
    Ok(store
        .list_accounts()?
        .into_iter()
        .map(|account| account.id)
        .collect())
}

/// Every configured account, for the places that are not scoped to one.
pub(super) fn accounts(store: &SqliteStore) -> Vec<AccountId> {
    account_rows(store).into_iter().map(|row| row.id).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::fixtures::{seeded, thread_like};
    use crate::ui::view::Shell;

    #[test]
    fn the_history_place_lists_what_was_opened_and_nothing_before() {
        let (store, _dir) = seeded();
        let mut shell = Shell::default();
        let history = shell
            .places
            .iter()
            .position(|place| place.source == crate::ui::view::Source::History)
            .expect("there is a History place");
        shell.select(history);
        let now = chrono::Utc::now();
        assert!(list_for(&store, shell.listing(PAGE), now).is_empty());

        let hi = thread_like(&store, "hi");
        mail_core::history::record(&store, hi, now);
        let listed = list_for(&store, shell.listing(PAGE), now);
        assert_eq!(
            listed.iter().map(|summary| summary.id).collect::<Vec<_>>(),
            [hi]
        );
    }
}

#[cfg(test)]
mod badge_tests {
    use super::*;
    use crate::ui::view::{Listing, Shell, Source, default_places, scoped_unread_filter};
    use mail_domain::{ReadState, ThreadSummary};
    use std::collections::BTreeSet;

    /// The rows `shell` lists for the place it has selected, when that place lists threads.
    fn listed(
        store: &SqliteStore,
        shell: &Shell,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<Vec<ThreadSummary>> {
        match shell.listing(1000) {
            listing @ (Listing::Threads(_) | Listing::Inbox { .. }) => {
                Some(list_for(store, listing, now))
            }
            _ => None,
        }
    }

    /// One source of truth: in every scope the window can show (every account, or a Space of
    /// one), each place's badge is the number of unread conversations opening it lists. A
    /// badge counted over every account beside a list of one said Trash held a message that
    /// was in another account's Trash.
    #[test]
    fn every_badge_counts_the_unread_its_place_lists_in_every_scope() {
        let work = crate::ui::fixtures::work();
        let store = &work.store;
        let now = chrono::Utc::now();
        let mut shell = Shell {
            places: default_places(),
            ..Shell::default()
        };
        let every: Vec<ThreadSummary> = list_for(store, Listing::Threads(shell.query(1000)), now);
        let accounts: BTreeSet<_> = every.iter().map(|row| row.account.clone()).collect();
        assert!(
            accounts.len() > 1,
            "the fixture needs mail in more than one account"
        );

        let mut differed = false;
        let scopes = std::iter::once(crate::ui::space::Scope::All).chain(
            accounts
                .iter()
                .cloned()
                .map(|id| crate::ui::space::Scope::Accounts(vec![id])),
        );
        for scope in scopes {
            shell.scope = scope.clone();
            for (index, place) in default_places().iter().enumerate() {
                if matches!(
                    place.source,
                    Source::Drafts | Source::Waiting | Source::History | Source::Scheduled
                ) {
                    continue;
                }
                shell.selected = index;
                let Some(rows) = listed(store, &shell, now) else {
                    continue;
                };
                let unread = rows
                    .iter()
                    .filter(|row| row.read == ReadState::Unread)
                    .count() as u64;
                let badge = count_badges(
                    store,
                    &[scoped_unread_filter(&place.source, shell.account_filter())],
                )[0]
                .unwrap_or(0);
                let unscoped = count_badges(store, &[scoped_unread_filter(&place.source, None)])[0]
                    .unwrap_or(0);
                differed |= unscoped != badge;
                assert_eq!(
                    badge, unread,
                    "{:?} over {scope:?}: the badge says {badge}, the list has {unread} unread",
                    place.source
                );
            }
        }
        assert!(
            differed,
            "no scope narrowed any badge: the test proves nothing"
        );
    }
}

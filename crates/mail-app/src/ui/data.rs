#[cfg(test)]
use crate::ui::view::Listing;
use mail_client::Client;
use mail_domain::*;
use porter_core::AccountId;

/// How many rows the list pane asks for at a time.
pub(super) const PAGE: u32 = 100;

/// The conversations a listing asks for, as the client reads them: for a test that checks what
/// a place lists.
#[cfg(test)]
pub(super) fn list_for(
    store: &mail_core::SqliteStore,
    listing: Listing,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ThreadSummary> {
    use mail_client::Query as _;
    super::list_search::Request::Place(listing)
        .asked()
        .run(store, now)
        .expect("listed")
        .threads
}

/// One account as the sidebar draws it: the id, the address, and the plan the provider comes from.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AccountRow {
    pub id: AccountId,
    pub address: String,
    pub plan: AccountPlan,
}

/// Every account, oldest first. A plan that does not parse becomes a plain IMAP account so the
/// account still has a host to name; `'{}'` is what the oldest fixtures wrote.
pub(super) fn account_rows(client: &Client) -> Vec<AccountRow> {
    client
        .stored()
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .map(|account| {
            let plan = account
                .plan
                .unwrap_or_else(|_| fallback_plan(&account.address));
            AccountRow {
                id: account.id,
                address: account.address,
                plan,
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

    /// The account's name as the window writes it: its address, or "Local folders".
    pub(in crate::ui) fn shown(&self) -> String {
        if self.is_local() {
            LOCAL_FOLDERS.to_owned()
        } else {
            self.address.clone()
        }
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

/// Every configured account, or why the client could not say. Where an empty answer would be
/// acted on (taking accounts out of the Spaces), a failed read must not look like no accounts.
pub(super) fn known_accounts(client: &Client) -> Result<Vec<AccountId>, mail_core::StoreError> {
    Ok(client
        .stored()
        .list_accounts()?
        .into_iter()
        .map(|account| account.id)
        .collect())
}

/// Every configured account, for the places that are not scoped to one.
pub(super) fn accounts(client: &Client) -> Vec<AccountId> {
    account_rows(client).into_iter().map(|row| row.id).collect()
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
    use crate::ui::view::{Listing, Shell, Source, default_places, scoped_badge};
    use mail_core::SqliteStore;
    use mail_domain::{ReadState, ThreadSummary};
    use std::collections::BTreeSet;

    /// What the client counts for `filter`, the way the sidebar asks for a badge: none for none.
    fn count_badges(store: &SqliteStore, filters: &[Option<Filter>]) -> Vec<Option<u64>> {
        use mail_client::Query as _;
        let asked: Vec<Filter> = filters.iter().flatten().cloned().collect();
        let counted = mail_client::query::Counts(asked)
            .run(store, chrono::Utc::now())
            .expect("counted");
        filters
            .iter()
            .map(|filter| {
                mail_client::query::Counts::of(&counted, filter.as_ref()?).filter(|n| *n > 0)
            })
            .collect()
    }

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
                    &[scoped_badge(&place.source, shell.account_filter())],
                )[0]
                .unwrap_or(0);
                let unscoped =
                    count_badges(store, &[scoped_badge(&place.source, None)])[0].unwrap_or(0);
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

//! What the window's data hooks make of the accounts and the places the client keeps read: they
//! seed a store, so they run beside the window's fixtures.

use mail_client::Syncing;
use mail_core::SqliteStore;
use mail_domain::*;
use mail_ui_kit::data::*;
use mail_ui_kit::view::Listing;

/// The conversations a listing asks for, as the client reads them: for a test that checks what
/// a place lists.
fn list_for(
    store: &mail_core::SqliteStore,
    listing: Listing,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<ThreadSummary> {
    use mail_client::Query as _;
    mail_ui_kit::list_search::Request::Place(listing)
        .asked()
        .run(store, now)
        .expect("listed")
        .threads
}
mod tests {
    use super::*;
    use crate::ui::fixtures::{seeded, thread_like, work};
    use mail_ui_kit::view::Shell;

    /// What a window draws from the accounts the client keeps read is what asking the store
    /// account by account gives, preferences included: a description, a pause.
    #[test]
    fn the_accounts_the_client_keeps_read_make_the_rows_the_store_gives() {
        let built = work();
        let client = crate::ui::fixtures::client(&built.store);
        let asked = account_rows(&client);
        assert!(asked.len() > 1, "the fixture has several accounts");
        client
            .stored()
            .set_account_description(asked[0].id.clone(), Some("Work"))
            .expect("described");
        client
            .stored()
            .set_account_syncing(asked[1].id.clone(), Syncing::Paused)
            .expect("paused");
        let asked = account_rows(&client);
        assert_eq!(asked[0].description.as_deref(), Some("Work"));
        assert_eq!(asked[1].syncing, Syncing::Paused);

        let kept = mail_ui_kit::live::read_now(&client, mail_client::query::Accounts);
        assert!(rows_of(&kept) == asked, "the kept accounts make other rows");
    }

    #[test]
    fn the_history_place_lists_what_was_opened_and_nothing_before() {
        let (store, _dir) = seeded();
        let mut shell = Shell::default();
        let history = shell
            .places
            .iter()
            .position(|place| place.source == mail_ui_kit::view::Source::History)
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
/// What the client counts for `filter`, the way the sidebar asks for a badge: none for none.
fn count_badges(store: &SqliteStore, filters: &[Option<Filter>]) -> Vec<Option<u64>> {
    use mail_client::Query as _;
    let asked: Vec<Filter> = filters.iter().flatten().cloned().collect();
    let counted = mail_client::query::Counts(asked)
        .run(store, chrono::Utc::now())
        .expect("counted");
    filters
        .iter()
        .map(|filter| mail_client::query::Counts::of(&counted, filter.as_ref()?).filter(|n| *n > 0))
        .collect()
}
mod badge_tests {
    use super::*;
    use mail_domain::{ReadState, ThreadSummary};
    use mail_ui_kit::view::{Listing, Shell, Source, default_places, scoped_unread_filter};
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
        let scopes = std::iter::once(mail_ui_kit::space::Scope::All).chain(
            accounts
                .iter()
                .cloned()
                .map(|id| mail_ui_kit::space::Scope::Accounts(vec![id])),
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
mod quick_tests {
    use super::*;
    use mail_domain::{MatchCtx, ThreadId, ThreadSummary};
    use mail_ui_kit::quick::{Quick, Toggle};
    use mail_ui_kit::view::{Listing, Shell, Source, default_places, scoped_unread_filter};

    /// Every combination of the toggles, none included.
    fn combinations() -> Vec<Quick> {
        (0..1u8 << Toggle::ALL.len())
            .map(|bits| {
                Toggle::ALL
                    .into_iter()
                    .enumerate()
                    .filter(|(n, _)| bits & (1 << n) != 0)
                    .fold(Quick::default(), |quick, (_, toggle)| quick.toggled(toggle))
            })
            .collect()
    }

    fn ids(rows: &[ThreadSummary]) -> Vec<ThreadId> {
        rows.iter().map(|row| row.id).collect()
    }

    /// In every place that lists conversations, the toggles keep exactly the rows of the whole
    /// list each toggle's filter fits, in the same order; every page deep is the start of that
    /// list; and the sidebar's badge does not move.
    #[test]
    fn the_toggles_narrow_each_place_and_page_it_as_the_store_does() {
        let work = crate::ui::fixtures::work();
        let store = &work.store;
        let now = chrono::Utc::now();
        let mut narrowed = 0;
        for (index, place) in default_places().iter().enumerate() {
            if !matches!(place.source, Source::Mail(_) | Source::Saved(_)) {
                continue;
            }
            let mut shell = Shell::default();
            shell.select(index);
            let whole = list_for(store, shell.listing(1000), now);
            let badge = |shell: &Shell| {
                count_badges(
                    store,
                    &[scoped_unread_filter(&place.source, shell.account_filter())],
                )
            };
            let badge_before = badge(&shell);
            for quick in combinations() {
                shell.quick = quick.clone();
                let fits = |row: &&ThreadSummary| {
                    quick.on().iter().all(|toggle| {
                        toggle.filter().fit(&MatchCtx {
                            summary: row,
                            corpus: None,
                            folders: &[],
                            sizes: &[],
                            cc: &[],
                            vips: &[],
                            now,
                        })
                    })
                };
                let want: Vec<ThreadId> = whole.iter().filter(fits).map(|row| row.id).collect();
                let got = list_for(store, shell.listing(1000), now);
                assert_eq!(ids(&got), want, "{:?} {:?}", place.source, quick.on());
                if want.len() < whole.len() {
                    narrowed += 1;
                }
                for depth in 1..=want.len() as u32 {
                    let page = list_for(store, shell.listing(depth), now);
                    assert_eq!(
                        ids(&page),
                        want[..depth as usize],
                        "{:?} {:?} at {depth}",
                        place.source,
                        quick.on()
                    );
                }
                assert_eq!(badge(&shell), badge_before, "{:?}", place.source);
            }
        }
        assert!(
            narrowed > 0,
            "no toggle narrowed any place: the test proves nothing"
        );
    }

    /// Empty Trash asks for the whole bin, whatever the toggles show of it; the search's
    /// candidates and the Inbox's returned reminders are held to them.
    #[test]
    fn the_whole_bin_and_the_scopes_beside_the_list() {
        let mut shell = Shell::default();
        let unfiltered = shell.query(10);
        shell.quick = Quick::default().toggled(Toggle::Attachments);
        assert_eq!(shell.whole_query(10), unfiltered);
        assert_ne!(shell.query(10), unfiltered);
        assert_eq!(shell.list_scope(), Some(Toggle::Attachments.filter()));
        match shell.listing(10) {
            Listing::Inbox { scope, .. } => {
                assert_eq!(scope, Some(Toggle::Attachments.filter()))
            }
            other => panic!("the first place is not the Inbox: {other:?}"),
        }
    }
}

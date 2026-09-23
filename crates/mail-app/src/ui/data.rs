use crate::view::Listing;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// How many rows the list pane asks for at a time.
pub(super) const PAGE: u32 = 100;

/// The conversations a listing asks for.
///
/// Shared by the blocking task and the first frame's fallback, for the reason `count_badges` is:
/// two copies would be two chances to disagree about what the list contains.
pub(super) fn list_for(store: &SqliteStore, listing: Listing) -> Vec<ThreadSummary> {
    match listing {
        Listing::Threads(query) => store
            .threads(&query, chrono::Utc::now())
            .map(|page| page.items)
            .unwrap_or_default(),
        Listing::Drafts => Vec::new(),
    }
}

/// One count per place, for the sidebar's badges.
///
/// A free function rather than a closure, because phase 8c calls it from two places: once on a
/// blocking thread, and once on the render thread for the first frame, when there is no answer
/// yet. Two copies of it would be two chances for them to disagree about what a badge counts.
pub(super) fn count_badges(store: &SqliteStore, filters: &[Option<Filter>]) -> Vec<Option<u64>> {
    let now = chrono::Utc::now();
    filters
        .iter()
        .map(|filter| match store.count(filter.as_ref()?, now) {
            Ok(0) | Err(_) => None,
            Ok(n) => Some(n),
        })
        .collect()
}

/// How many conversations to render ahead of the user.
///
/// A screenful, near enough. Warming the whole mailbox would evict the conversations they are
/// about to open in order to hold the ones they are not, which is the cache paying for itself in
/// reverse.
const WARM: u32 = 20;

/// Render the newest conversations in the inbox into the cache.
///
/// Errors are dropped on purpose: nothing here is load-bearing. A message that cannot be read is
/// one the reader will report when it is opened, and failing to warm is only failing to be fast.
pub(super) fn warm_the_first_screenful(store: &SqliteStore) -> usize {
    let query = Query {
        filter: crate::view::place_filter(MailboxRole::Inbox),
        sort: Sort {
            property: Property::Date,
            dir: SortDir::Desc,
        },
        page: PageReq {
            after: None,
            limit: WARM,
        },
    };
    let Ok(page) = store.threads(&query, chrono::Utc::now()) else {
        return 0;
    };
    let mut warmed = 0;
    for summary in page.items {
        let Ok(loaded) = store.thread(summary.id) else {
            continue;
        };
        let messages: Vec<Message> = loaded
            .messages
            .iter()
            .filter_map(|id| store.message(*id).ok())
            .collect();
        warmed += crate::reader::prewarm(store, &messages, mail_mime::SanitizePolicy::CURRENT);
    }
    warmed
}

/// One account as the sidebar draws it: the id, the address, and the plan the provider comes from.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct AccountRow {
    pub id: AccountId,
    pub address: String,
    pub plan: AccountPlan,
}

/// Every account, oldest first. A plan that does not parse becomes a plain IMAP account so the
/// tile still has a host to name; `'{}'` is what the oldest fixtures wrote.
pub(super) fn account_rows(store: &SqliteStore) -> Vec<AccountRow> {
    let db = store.connection();
    let Ok(mut stmt) = db.prepare("SELECT id, address, plan FROM accounts ORDER BY created_at")
    else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok())
        .filter_map(|(id, address, plan)| {
            let id = AccountId::from_uuid(id.parse().ok()?);
            let plan = serde_json::from_str(&plan).unwrap_or_else(|_| fallback_plan(&address));
            Some(AccountRow { id, address, plan })
        })
        .collect()
}

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

/// Every configured account, for the places that are not scoped to one.
pub(super) fn accounts(store: &SqliteStore) -> Vec<AccountId> {
    let db = store.connection();
    let Ok(mut stmt) = db.prepare("SELECT id FROM accounts ORDER BY created_at") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok())
        .filter_map(|id| id.parse().ok())
        .map(AccountId::from_uuid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::warm_the_first_screenful;
    use crate::ui::fixtures::realistic;

    #[test]
    fn the_first_screenful_is_rendered_before_anyone_opens_it() {
        // Phase 8e's caller. Asserted through the function the window mounts rather than through
        // the window, because what is being checked is that the work happens on an ordinary
        // thread with nothing polling it — which is the only reason this part of phase 8 works
        // while F140 stands.
        let (store, _dir) = realistic();

        // Counted by the warming itself rather than by looking at the cache afterwards: the
        // cache is process-wide and these tests share a process, so "something is in it" is a
        // sentence another test can make true. What is asserted is what *this* call did.
        let warming = store.clone();
        let warmed = std::thread::spawn(move || warm_the_first_screenful(&warming))
            .join()
            .expect("warming did not panic");

        assert!(warmed > 0, "the mailbox was not rendered ahead of the user");
    }
}

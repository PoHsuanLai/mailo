use crate::ui::view::Listing;
use mail_domain::*;
use mail_store::{SqliteStore, Store};

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
        Listing::Inbox { query, scope } => mail_core::follow_up::on_top(
            mail_core::follow_up::returned(store, scope.as_ref(), now),
            rows(&query),
        ),
        Listing::Waiting { scope } => mail_core::follow_up::waiting(store, scope.as_ref(), now),
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

/// Whether what the list shows is only local folders, so there is nothing for a sync to do: the
/// pressed tile is local folders, or no tile is pressed and every account in the Space's `scope`
/// (empty meaning all) is. The list then has no Sync button and says nothing about syncing.
pub(super) fn syncs_nothing(
    rows: &[AccountRow],
    pressed: Option<AccountId>,
    scope: &[AccountId],
) -> bool {
    let in_view: Vec<&AccountRow> = rows
        .iter()
        .filter(|row| match pressed {
            Some(id) => row.id == id,
            None => scope.is_empty() || scope.contains(&row.id),
        })
        .collect();
    !in_view.is_empty() && in_view.iter().all(|row| row.is_local())
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

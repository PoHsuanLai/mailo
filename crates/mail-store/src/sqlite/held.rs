//! Accounts whose sign-in mailo holds itself, and the mode that sets them aside.
//!
//! Linked to the desktop's accountd, the accounts Mail shows and syncs are accountd's
//! (`AuthPlan::Granted`). A row whose plan keeps a password or an OAuth sign-in of mailo's own is
//! *held*: it stays in the store exactly as it is (plan, mail, keyring items), for a build that
//! is not linked to read again, and is left out of every list that asks [`SqliteStore::accounts`]
//! and of every thread listing and count while the mode is on. A "Local folders" account has no
//! sign-in and is never held.
//!
//! The mode is the store's, set once by the process that decided the link, and a store that never
//! has it set answers as it always did.

use super::SqliteStore;
use porter_core::AccountId;
use std::sync::atomic::Ordering;

/// SQL over `accounts.plan` that is 1 for a held row and 0 for every other, never NULL. A plan that
/// is not JSON, or says no sign-in (the oldest fixtures wrote `{}`), is not held: it cannot be
/// said to be anyone's sign-in, and is left where it was.
macro_rules! held {
    () => {
        "COALESCE(CASE WHEN json_valid(plan) THEN \
            (json_extract(plan, '$.auth.kind') IN ('password', 'o_auth') \
             AND json_extract(plan, '$.incoming.kind') IS NOT 'local') \
         END, 0)"
    };
}

impl SqliteStore {
    /// Set aside, or take back, the accounts mailo holds the sign-in of. Nothing is written.
    pub fn set_granted_only(&self, on: bool) {
        self.granted_only.store(on, Ordering::Relaxed);
    }

    /// Whether held accounts are set aside.
    pub fn granted_only(&self) -> bool {
        self.granted_only.load(Ordering::Relaxed)
    }

    /// What to select accounts `FROM`: the table, or, with held accounts set aside, the table
    /// without them. Parenthesised, so an alias can follow it.
    pub fn accounts(&self) -> &'static str {
        if self.granted_only() {
            concat!("(SELECT * FROM accounts WHERE NOT (", held!(), "))")
        } else {
            "accounts"
        }
    }

    /// The accounts whose sign-in mailo holds, whatever the mode: how the account list knows
    /// whether to say so. Oldest first.
    pub fn held_accounts(&self) -> Vec<AccountId> {
        let db = self.connection();
        let sql = concat!(
            "SELECT id FROM accounts WHERE (",
            held!(),
            ") ORDER BY created_at, id"
        );
        let Ok(mut stmt) = db.prepare(sql) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok)
            .filter_map(|id| AccountId::parse(&id).ok())
            .collect()
    }

    /// A clause on `ts.account` that leaves held accounts' mail out of a listing, when the mode
    /// is on.
    pub(super) fn set_aside(&self) -> Option<&'static str> {
        self.granted_only().then_some(concat!(
            "(ts.account NOT IN (SELECT id FROM accounts WHERE (",
            held!(),
            ")))"
        ))
    }
}

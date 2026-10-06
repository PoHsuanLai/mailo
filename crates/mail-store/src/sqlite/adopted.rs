//! Which accounts have had their secrets adopted into porter's store (migration 0028).
//!
//! Only the fact that the move is finished is kept here, never a secret. `mail-runtime`'s
//! `adopt` reads [`SqliteStore::unadopted_accounts`], does what each has left, and calls
//! [`SqliteStore::mark_secrets_adopted`] once no entry of an earlier build is left to move.

use super::SqliteStore;
use crate::StoreError;
use chrono::{DateTime, SecondsFormat, Utc};
use porter_core::AccountId;

impl SqliteStore {
    /// The accounts whose secrets have not been recorded as adopted, in the order they were added.
    pub fn unadopted_accounts(&self) -> Result<Vec<AccountId>, StoreError> {
        let db = self.connection();
        let mut stmt = db.prepare(
            "SELECT id FROM accounts
             WHERE id NOT IN (SELECT account FROM secrets_adopted)
             ORDER BY created_at, id",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        ids.iter()
            .map(|id| {
                AccountId::parse(id)
                    .map_err(|e| StoreError::Db(format!("account id {id:?} does not read: {e}")))
            })
            .collect()
    }

    /// Whether `account`'s secrets are recorded as adopted.
    pub fn secrets_adopted(&self, account: &AccountId) -> Result<bool, StoreError> {
        Ok(self.connection().query_row(
            "SELECT count(*) > 0 FROM secrets_adopted WHERE account = ?1",
            [account.to_string()],
            |row| row.get(0),
        )?)
    }

    /// Record that `account`'s secrets are adopted, at `at`. Recording it twice keeps the first
    /// time; an account that is not in the store is not recorded.
    pub fn mark_secrets_adopted(
        &self,
        account: &AccountId,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO secrets_adopted (account, adopted_at)
             SELECT id, ?2 FROM accounts WHERE id = ?1
             ON CONFLICT (account) DO NOTHING",
            rusqlite::params![
                account.to_string(),
                at.to_rfc3339_opts(SecondsFormat::Secs, true)
            ],
        )?;
        Ok(())
    }
}

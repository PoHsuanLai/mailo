//! How each account's passes last ended (migration 0030).

use super::SqliteStore;
use super::row::{from_time, time};
use crate::StoreError;
use crate::changes::Topics;
use crate::passes::{PassFailed, Passes};
use chrono::{DateTime, Utc};
use porter_core::AccountId;
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    /// A pass of `account` that began at `at` ran to its end: the account's mail is as current as
    /// that, and any failure recorded before it is over. An account the store does not hold is
    /// not recorded.
    pub fn pass_finished(&self, account: AccountId, at: DateTime<Utc>) -> Result<(), StoreError> {
        self.touch(Topics::ACCOUNTS).execute(
            "INSERT INTO account_passes (account, finished_at)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM accounts WHERE id = ?1)
             ON CONFLICT (account) DO UPDATE
             SET finished_at = excluded.finished_at, failed_at = NULL, failed_why = NULL",
            params![account.to_string(), from_time(at)],
        )?;
        Ok(())
    }

    /// A pass of `account` that began at `at` could not run, for `why`. When a pass last
    /// finished is kept.
    pub fn pass_failed(
        &self,
        account: AccountId,
        at: DateTime<Utc>,
        why: &str,
    ) -> Result<(), StoreError> {
        self.touch(Topics::ACCOUNTS).execute(
            "INSERT INTO account_passes (account, failed_at, failed_why)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM accounts WHERE id = ?1)
             ON CONFLICT (account) DO UPDATE
             SET failed_at = excluded.failed_at, failed_why = excluded.failed_why",
            params![account.to_string(), from_time(at), why],
        )?;
        Ok(())
    }

    /// How `account`'s passes last ended. Empty when none has ended since this was recorded.
    pub fn passes(&self, account: AccountId) -> Result<Passes, StoreError> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .reader()
            .query_row(
                "SELECT finished_at, failed_at, failed_why FROM account_passes WHERE account = ?1",
                [account.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((finished, failed_at, failed_why)) = row else {
            return Ok(Passes::default());
        };
        let finished = finished
            .map(|text| time("account_passes.finished_at", &text))
            .transpose()?;
        let failed = match (failed_at, failed_why) {
            (Some(at), why) => Some(PassFailed {
                at: time("account_passes.failed_at", &at)?,
                // A failure kept without words (a NULL column) reads as none: no error is lost.
                why: why.unwrap_or_default(),
            }),
            (None, _) => None,
        };
        Ok(Passes { finished, failed })
    }
}

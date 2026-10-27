//! What the person says about an account beyond its plan: its description, whether it syncs,
//! and the folders picked for roles. The only code that reads or writes `account_prefs`, and,
//! with `accounts.rs`, `account_caps`.
//!
//! What the server was last observed to support is kept as it was observed, in
//! `account_caps.observed`, and again with the picked folders laid over it
//! ([`crate::with_picks`]) in `account_caps.caps`, which the folder lists read in SQL. A refresh
//! replaces the first and lays the picks over it again; a pick, or a role given back to
//! Automatic, lays the picks over the observed roles again. So a refresh never undoes a pick,
//! and a role given back plays the folder the server named for it at once.
//!
//! Each change reads and writes on one transaction, so two changes at once cannot each write
//! over what the other read.

use super::SqliteStore;
use super::row::{json, to_json};
use crate::changes::Topics;
use crate::{AccountPrefs, StoreError, Syncing};
use mail_domain::{AccountCaps, MailboxRole};
use porter_core::AccountId;
use rusqlite::{Connection, OptionalExtension, params};

impl SqliteStore {
    /// What the person said about `account`; the defaults when they said nothing.
    pub fn account_prefs(&self, account: AccountId) -> Result<AccountPrefs, StoreError> {
        prefs_on(&self.reader(), &account)
    }

    /// Call `account` `description` in the window, or by its address again with `None` or a
    /// blank one.
    pub fn set_account_description(
        &self,
        account: AccountId,
        description: Option<&str>,
    ) -> Result<(), StoreError> {
        let description = description.map(str::trim).filter(|text| !text.is_empty());
        self.write_prefs(account, |prefs| {
            prefs.description = description.map(str::to_owned);
        })
    }

    /// Pause `account`'s syncing, or let it sync again.
    pub fn set_account_syncing(
        &self,
        account: AccountId,
        syncing: Syncing,
    ) -> Result<(), StoreError> {
        self.write_prefs(account, |prefs| prefs.syncing = syncing)
    }

    /// Make the folder at `path` the one that plays `role` on `account`, or leave the role to
    /// what the server says again with `None`.
    ///
    /// The stored capabilities are written again in the same transaction, since the folder lists
    /// read the roles from them in SQL: the picks laid over what the server was last observed
    /// to say, so a role given back plays the folder the server named for it.
    pub fn set_folder_role(
        &self,
        account: AccountId,
        role: MailboxRole,
        path: Option<&str>,
    ) -> Result<(), StoreError> {
        let db = self.touch(Topics::ACCOUNTS);
        let tx = db.unchecked_transaction()?;
        let prefs = edit_prefs(&tx, &account, |prefs| {
            prefs.folders.retain(|(had, _)| *had != role);
            if let Some(path) = path {
                prefs.folders.retain(|(_, picked)| picked != path);
                prefs.folders.push((role, path.to_owned()));
            }
        })?;
        let observed: Option<String> = tx
            .query_row(
                "SELECT coalesce(observed, caps) FROM account_caps WHERE account = ?1",
                [account.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(observed) = observed {
            let caps = crate::with_picks(json("AccountCaps", &observed)?, &prefs.folders);
            tx.execute(
                "UPDATE account_caps SET caps = ?2 WHERE account = ?1",
                params![account.to_string(), to_json("AccountCaps", &caps)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// `caps` with `account`'s picked folders laid over them.
    pub(super) fn picked(
        &self,
        account: &AccountId,
        caps: AccountCaps,
    ) -> Result<AccountCaps, StoreError> {
        let prefs = self.account_prefs(account.clone())?;
        Ok(crate::with_picks(caps, &prefs.folders))
    }

    /// Change `account`'s preferences as `edit` does, read and written on one transaction.
    fn write_prefs(
        &self,
        account: AccountId,
        edit: impl FnOnce(&mut AccountPrefs),
    ) -> Result<(), StoreError> {
        let db = self.touch(Topics::ACCOUNTS);
        let tx = db.unchecked_transaction()?;
        edit_prefs(&tx, &account, edit)?;
        tx.commit()?;
        Ok(())
    }
}

/// What the person said about `account`, read on `db`; the defaults when they said nothing.
fn prefs_on(db: &Connection, account: &AccountId) -> Result<AccountPrefs, StoreError> {
    let row: Option<(Option<String>, bool, String)> = db
        .query_row(
            "SELECT description, paused, folder_roles FROM account_prefs WHERE account = ?1",
            [account.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((description, paused, folders)) = row else {
        return Ok(AccountPrefs::default());
    };
    Ok(AccountPrefs {
        description,
        syncing: if paused { Syncing::Paused } else { Syncing::On },
        folders: json("AccountPrefs.folder_roles", &folders)?,
    })
}

/// Read `account`'s preferences on `db`, change them as `edit` does, and write them back: what
/// they are now. [`StoreError::NoAccount`] when there is no such account. `db` is the caller's
/// transaction.
fn edit_prefs(
    db: &Connection,
    account: &AccountId,
    edit: impl FnOnce(&mut AccountPrefs),
) -> Result<AccountPrefs, StoreError> {
    let known: Option<i64> = db
        .query_row(
            "SELECT 1 FROM accounts WHERE id = ?1",
            [account.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if known.is_none() {
        return Err(StoreError::NoAccount(account.clone()));
    }
    let mut prefs = prefs_on(db, account)?;
    edit(&mut prefs);
    db.execute(
        "INSERT INTO account_prefs (account, description, paused, folder_roles)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account) DO UPDATE SET description = excluded.description,
             paused = excluded.paused, folder_roles = excluded.folder_roles",
        params![
            account.to_string(),
            prefs.description,
            prefs.syncing == Syncing::Paused,
            to_json("AccountPrefs.folder_roles", &prefs.folders)?,
        ],
    )?;
    Ok(prefs)
}

/// Write `account`'s capabilities as the server was observed to give them, and with its picked
/// folders laid over them, observed at `at` (as the caller writes the time). `db` is the
/// caller's transaction, or the writer.
pub(super) fn write_caps(
    db: &Connection,
    account: &AccountId,
    observed: &AccountCaps,
    at: &str,
) -> Result<(), StoreError> {
    let picks = prefs_on(db, account)?.folders;
    let caps = crate::with_picks(observed.clone(), &picks);
    db.execute(
        "INSERT INTO account_caps (account, caps, observed, observed_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(account) DO UPDATE SET caps = excluded.caps,
             observed = excluded.observed, observed_at = excluded.observed_at",
        params![
            account.to_string(),
            to_json("AccountCaps", &caps)?,
            to_json("AccountCaps", observed)?,
            at,
        ],
    )?;
    Ok(())
}

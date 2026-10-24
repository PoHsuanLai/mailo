//! What the person says about an account beyond its plan: its description, whether it syncs,
//! and the folders picked for roles. The only code that reads or writes `account_prefs`.
//!
//! The picked folders are laid over the observed roles ([`crate::with_picks`]) wherever the
//! store hands capabilities out or takes them in, so a refresh of the capabilities, which
//! replaces what was observed, never replaces what was picked.

use super::SqliteStore;
use super::row::{json, to_json};
use crate::{AccountPrefs, Store, StoreError, Syncing};
use mail_domain::{AccountCaps, MailboxRole};
use porter_core::AccountId;
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    /// What the person said about `account`; the defaults when they said nothing.
    pub fn account_prefs(&self, account: AccountId) -> Result<AccountPrefs, StoreError> {
        let row: Option<(Option<String>, bool, String)> = self
            .reader()
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
    /// The stored capabilities are written again at once, since the folder lists read the roles
    /// from them in SQL. A role given back plays no folder until the next refresh of the
    /// folders says which one the server means: what it said before was replaced by the pick.
    pub fn set_folder_role(
        &self,
        account: AccountId,
        role: MailboxRole,
        path: Option<&str>,
    ) -> Result<(), StoreError> {
        self.write_prefs(account.clone(), |prefs| {
            prefs.folders.retain(|(had, _)| *had != role);
            if let Some(path) = path {
                prefs.folders.retain(|(_, picked)| picked != path);
                prefs.folders.push((role, path.to_owned()));
            }
        })?;
        let Some(mut caps) = self.account_caps(account.clone())? else {
            return Ok(());
        };
        if path.is_none() {
            caps.folders.0.retain(|(_, played)| *played != role);
        }
        let observed = caps.observed_at;
        self.put_caps(account, &caps, observed)
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

    fn write_prefs(
        &self,
        account: AccountId,
        edit: impl FnOnce(&mut AccountPrefs),
    ) -> Result<(), StoreError> {
        if self.account(account.clone())?.is_none() {
            return Err(StoreError::NoAccount(account));
        }
        let mut prefs = self.account_prefs(account.clone())?;
        edit(&mut prefs);
        self.connection().execute(
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
        Ok(())
    }
}

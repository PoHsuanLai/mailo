//! Accounts, the identities they send as, and the few facts kept beside them.
//!
//! The only code that reads or writes the `accounts`, `account_caps` and `identities` tables for
//! anyone else: what a caller asks for is an account or an identity, and what it gets back is
//! one.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json, uuid};
use crate::account::{NewAccount, StoredAccount};
use crate::{FollowUpHold, SenderRecord, StoreError};
use chrono::{DateTime, NaiveDateTime, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{
    AccountCaps, AccountPlan, Address, DraftId, Identity, IdentityId, IsDefault, LabelId,
    LabelOrigin, MessageId, ThreadId,
};
use porter_core::AccountId;
use rusqlite::{OptionalExtension, Row, params};

/// The columns of an identity, in the order [`identity_of`] reads them.
const IDENTITY: &str = "id, account, from_name, from_email, reply_to, signature, is_default";

/// An identity out of a row selected with [`IDENTITY`].
fn identity_of(row: &Row<'_>) -> Result<Identity, StoreError> {
    let id: String = row.get(0)?;
    let account: String = row.get(1)?;
    let reply_to: Option<String> = row.get(4)?;
    let default: String = row.get(6)?;
    Ok(Identity {
        id: IdentityId::from_uuid(uuid("IdentityId", &id)?),
        account: account_id_from_uuid(uuid("AccountId", &account)?),
        from: Address {
            name: row.get(2)?,
            email: row.get(3)?,
        },
        reply_to: match reply_to {
            Some(text) => json::<Option<Address>>("Identity.reply_to", &text)?,
            None => None,
        },
        signature: row.get(5)?,
        default: json::<IsDefault>("Identity.is_default", &default)?,
    })
}

impl SqliteStore {
    /// Every account that is shown, oldest first. With held accounts set aside
    /// ([`SqliteStore::set_granted_only`]) they are not here.
    ///
    /// A row whose id is not an id is left out: there is nothing to call it by.
    pub fn list_accounts(&self) -> Result<Vec<StoredAccount>, StoreError> {
        self.read_accounts(self.accounts(), "", [])
    }

    /// Every account the store holds, whether or not held accounts are set aside. For the code
    /// that has to see them: reconciling with the desktop's accounts, or removing one.
    pub fn list_all_accounts(&self) -> Result<Vec<StoredAccount>, StoreError> {
        self.read_accounts("accounts", "", [])
    }

    /// The account with this address, set aside or not. The address is compared as written.
    pub fn account_by_address(&self, address: &str) -> Result<Option<StoredAccount>, StoreError> {
        Ok(self
            .read_accounts("accounts", "WHERE a.address = ?1", [address])?
            .into_iter()
            .next())
    }

    /// The account with this id, set aside or not.
    pub fn account(&self, id: AccountId) -> Result<Option<StoredAccount>, StoreError> {
        Ok(self
            .read_accounts("accounts", "WHERE a.id = ?1", [id.to_string()])?
            .into_iter()
            .next())
    }

    fn read_accounts<P: rusqlite::Params>(
        &self,
        from: &str,
        filter: &str,
        params: P,
    ) -> Result<Vec<StoredAccount>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare(&format!(
            "SELECT a.id, a.address, a.plan, c.caps
             FROM {from} a LEFT JOIN account_caps c ON c.account = a.id
             {filter} ORDER BY a.created_at"
        ))?;
        let rows = stmt.query_map(params, |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, address, plan, caps) = row?;
            let Ok(id) = uuid("AccountId", &id) else {
                continue;
            };
            out.push(StoredAccount {
                id: account_id_from_uuid(id),
                plan: json("AccountPlan", &plan),
                caps: caps.map(|text| json("AccountCaps", &text)),
                address,
            });
        }
        Ok(out)
    }

    /// Write an account, its identities and its capabilities, all of them or none.
    ///
    /// An address already here keeps its row, its id and when it was created, and has its plan
    /// brought up to date and its capabilities replaced; identities already stored are left as
    /// the user has them. [`StoreError::Conflict`] when the identities or capabilities cannot
    /// go with the account (the address is held by an account with another id), and nothing is
    /// written.
    pub fn upsert_account(&self, new: &NewAccount<'_>) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let account = new.id.to_string();
        tx.execute(
            // The plan is refreshed — a preset may have learned a better host since — while
            // `created_at` and the id stay as they were.
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(address) DO UPDATE SET plan = excluded.plan",
            params![
                account,
                new.address,
                to_json("AccountPlan", new.plan)?,
                new.at.to_rfc3339()
            ],
        )?;
        for identity in new.identities {
            tx.execute(
                "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                     is_default)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(id) DO NOTHING",
                params![
                    identity.id.to_string(),
                    account,
                    identity.from.name,
                    identity.from.email,
                    identity
                        .reply_to
                        .as_ref()
                        .map(|reply_to| to_json("Identity.reply_to", &Some(reply_to)))
                        .transpose()?,
                    identity.signature,
                    to_json("Identity.is_default", &identity.default)?,
                ],
            )?;
        }
        write_caps(&tx, &account, new.caps, new.at)?;
        tx.commit()?;
        Ok(())
    }

    /// Replace the account's plan. [`StoreError::NoAccount`] when there is no such account.
    pub fn set_account_plan(&self, id: AccountId, plan: &AccountPlan) -> Result<(), StoreError> {
        let changed = self.connection().execute(
            "UPDATE accounts SET plan = ?2 WHERE id = ?1",
            params![id.to_string(), to_json("AccountPlan", plan)?],
        )?;
        if changed == 0 {
            return Err(StoreError::NoAccount(id));
        }
        Ok(())
    }

    /// What the server was last observed to support, for one account. `None` when nothing has
    /// connected yet, which a caller must read as "assume nothing" rather than as an error.
    pub fn account_caps(&self, id: AccountId) -> Result<Option<AccountCaps>, StoreError> {
        let stored: Option<String> = self
            .reader()
            .query_row(
                "SELECT caps FROM account_caps WHERE account = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        stored.map(|text| json("AccountCaps", &text)).transpose()
    }

    /// Record what the server was observed to support at `at`, replacing what was there.
    /// [`StoreError::NoAccount`] when there is no such account.
    pub fn set_account_caps(
        &self,
        id: AccountId,
        caps: &AccountCaps,
        at: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        let db = self.connection();
        let known: Option<i64> = db
            .query_row(
                "SELECT 1 FROM accounts WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if known.is_none() {
            return Err(StoreError::NoAccount(id));
        }
        write_caps(&db, &id.to_string(), caps, at)
    }

    /// When the last pass of `account` finished, as the store recorded it: the newest of its
    /// mailboxes' stamps, which every ingest that knows where a mailbox got to writes.
    pub fn last_synced(&self, account: AccountId) -> Result<Option<DateTime<Utc>>, StoreError> {
        let stamp: Option<String> = self.reader().query_row(
            "SELECT MAX(synced_at) FROM sync_state WHERE account = ?1",
            [account.to_string()],
            |row| row.get(0),
        )?;
        // Written by SQLite's own `datetime('now')`, which is not RFC 3339.
        stamp
            .map(|text| {
                NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M:%S")
                    .map(|naive| naive.and_utc())
                    .map_err(|e| StoreError::Decode {
                        what: "sync_state.synced_at".to_owned(),
                        why: e.to_string(),
                    })
            })
            .transpose()
    }

    /// The account's identities, the default first and then by id, so the same one comes first
    /// every time.
    pub fn identities(&self, account: AccountId) -> Result<Vec<Identity>, StoreError> {
        self.read_identities(
            "WHERE account = ?1 ORDER BY is_default DESC, id",
            [account.to_string()],
        )
    }

    /// The identity with this id.
    pub fn identity(&self, id: IdentityId) -> Result<Option<Identity>, StoreError> {
        Ok(self
            .read_identities("WHERE id = ?1", [id.to_string()])?
            .into_iter()
            .next())
    }

    /// The identity of the user's with this address, on any account, ignoring case. The default
    /// one when an address is on several.
    pub fn identity_for_address(&self, address: &str) -> Result<Option<Identity>, StoreError> {
        Ok(self
            .read_identities(
                "WHERE lower(from_email) = lower(?1) ORDER BY is_default DESC, id LIMIT 1",
                [address.trim()],
            )?
            .into_iter()
            .next())
    }

    /// Every address the user sends from on any account, as the identities have them.
    pub fn identity_addresses(&self) -> Result<Vec<String>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare("SELECT from_email FROM identities")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<String>, _>>()?)
    }

    /// Set the signature of an identity, or clear it with `None`.
    pub fn set_signature(
        &self,
        identity: IdentityId,
        signature: Option<&str>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "UPDATE identities SET signature = ?2 WHERE id = ?1",
            params![identity.to_string(), signature],
        )?;
        Ok(())
    }

    fn read_identities<P: rusqlite::Params>(
        &self,
        tail: &str,
        params: P,
    ) -> Result<Vec<Identity>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare(&format!("SELECT {IDENTITY} FROM identities {tail}"))?;
        let mut rows = stmt.query(params)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            out.push(identity_of(row)?);
        }
        Ok(out)
    }

    /// A label the user named, on `account`. [`StoreError::Conflict`] when the account already
    /// has one by that name, which is the answer a caller wants to give the user.
    pub fn create_label(&self, account: AccountId, name: &str) -> Result<LabelId, StoreError> {
        let id = LabelId::generate();
        self.connection().execute(
            "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, ?3, ?4)",
            params![
                id.to_string(),
                account.to_string(),
                name,
                to_json("LabelOrigin", &LabelOrigin::User)?
            ],
        )?;
        Ok(id)
    }

    /// The stored message of `account` with this `Message-ID`, as the message stores it.
    pub fn message_by_rfc_id(
        &self,
        account: AccountId,
        rfc_id: &str,
    ) -> Result<Option<MessageId>, StoreError> {
        self.rfc_lookup("id", account, rfc_id)?
            .map(|id| uuid("MessageId", &id).map(MessageId::from_uuid))
            .transpose()
    }

    /// The conversation the stored message of `account` with this `Message-ID` is in.
    pub fn thread_by_rfc_id(
        &self,
        account: AccountId,
        rfc_id: &str,
    ) -> Result<Option<ThreadId>, StoreError> {
        self.rfc_lookup("thread", account, rfc_id)?
            .map(|id| uuid("ThreadId", &id).map(ThreadId::from_uuid))
            .transpose()
    }

    fn rfc_lookup(
        &self,
        column: &str,
        account: AccountId,
        rfc_id: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(self
            .connection()
            .query_row(
                &format!(
                    "SELECT {column} FROM messages WHERE account = ?1 AND rfc_message_id = ?2
                     LIMIT 1"
                ),
                params![account.to_string(), rfc_id],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// When `account` was first watched, writing `now` if nothing has watched it before. The
    /// first writer wins and every later pass reads what it wrote, so a restart neither re-arms
    /// nor forgets.
    pub fn arm_notify_floor(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, StoreError> {
        let db = self.connection();
        db.execute(
            "INSERT OR IGNORE INTO notify_floor (account, armed_at) VALUES (?1, ?2)",
            params![account.to_string(), now.to_rfc3339()],
        )?;
        let stored: String = db.query_row(
            "SELECT armed_at FROM notify_floor WHERE account = ?1",
            [account.to_string()],
            |r| r.get(0),
        )?;
        time("notify_floor.armed_at", &stored)
    }

    /// Keep a composer's reminder until its message has left, replacing the draft's earlier one.
    pub fn hold_follow_up(&self, held: &FollowUpHold) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO follow_up_held (draft, account, message_id, thread, due_at, set_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(draft) DO UPDATE SET account = excluded.account,
                 message_id = excluded.message_id, thread = excluded.thread,
                 due_at = excluded.due_at, set_at = excluded.set_at",
            params![
                held.draft.to_string(),
                held.account.to_string(),
                held.message_id,
                held.thread.map(|thread| thread.to_string()),
                from_time(held.at),
                from_time(held.set),
            ],
        )?;
        Ok(())
    }

    /// Let a draft's held reminder go. Nothing held is not a failure.
    pub fn release_follow_up(&self, draft: DraftId) -> Result<(), StoreError> {
        self.connection().execute(
            "DELETE FROM follow_up_held WHERE draft = ?1",
            [draft.to_string()],
        )?;
        Ok(())
    }

    /// Every held reminder. One that does not read back is left out: it is a reminder, not mail,
    /// and the rest are still wanted.
    pub fn follow_up_holds(&self) -> Result<Vec<FollowUpHold>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare(
            "SELECT draft, account, message_id, thread, due_at, set_at FROM follow_up_held",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (draft, account, message_id, thread, at, set) = row?;
            let held = (|| -> Result<FollowUpHold, StoreError> {
                Ok(FollowUpHold {
                    draft: DraftId::from_uuid(uuid("DraftId", &draft)?),
                    account: account_id_from_uuid(uuid("AccountId", &account)?),
                    message_id,
                    thread: thread
                        .map(|thread| uuid("ThreadId", &thread).map(ThreadId::from_uuid))
                        .transpose()?,
                    at: time("follow_up_held.due_at", &at)?,
                    set: time("follow_up_held.set_at", &set)?,
                })
            })();
            out.extend(held);
        }
        Ok(out)
    }

    /// Every sender over every message the store holds, with whether the user has written to
    /// them. A sender whose newest date does not read is left out: the cards then say "first
    /// mail", which is the cautious thing for them to say.
    pub fn sender_history(&self) -> Result<Vec<SenderRecord>, StoreError> {
        let db = self.reader();
        // Addresses on the To or Cc of anything sent from one of the user's accounts.
        let mut written = db.prepare(
            "SELECT DISTINCT lower(json_extract(r.value, '$.email'))
               FROM messages m, json_each(m.recipients, '$.to') r
              WHERE lower(m.from_email) IN (SELECT lower(address) FROM accounts)
             UNION
             SELECT DISTINCT lower(json_extract(r.value, '$.email'))
               FROM messages m, json_each(m.recipients, '$.cc') r
              WHERE lower(m.from_email) IN (SELECT lower(address) FROM accounts)",
        )?;
        let written: std::collections::HashSet<String> = written
            .query_map([], |row| row.get::<_, Option<String>>(0))?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        // With one `MAX()` in the select list, SQLite takes the bare `from_name` from the row
        // that holds the maximum, which is the name on their newest message.
        let mut stmt = db.prepare(
            "SELECT lower(from_email), COUNT(DISTINCT thread), MAX(date), from_name
               FROM messages
              GROUP BY lower(from_email)",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (email, threads, last, name) = row?;
            let Ok(last) = time("messages.date", &last) else {
                continue;
            };
            out.push(SenderRecord {
                replied: written.contains(&email),
                email,
                name,
                threads,
                last,
            });
        }
        Ok(out)
    }
}

/// Write the capabilities of an account, replacing what was observed before.
fn write_caps(
    db: &rusqlite::Connection,
    account: &str,
    caps: &AccountCaps,
    at: DateTime<Utc>,
) -> Result<(), StoreError> {
    db.execute(
        "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(account) DO UPDATE
             SET caps = excluded.caps, observed_at = excluded.observed_at",
        params![account, to_json("AccountCaps", caps)?, at.to_rfc3339()],
    )?;
    Ok(())
}

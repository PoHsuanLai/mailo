//! Accounts, the identities they send as, and the few facts kept beside them.
//!
//! The only code that reads or writes the `accounts`, `account_caps` and `identities` tables for
//! anyone else: what a caller asks for is an account or an identity, and what it gets back is
//! one.

use super::SqliteStore;
use super::row::{from_time, json, time, to_json, uuid};
use crate::account::{NewAccount, StoredAccount};
use crate::changes::Topics;
use crate::{FollowUpHold, SenderRecord, StoreError};
use chrono::{DateTime, NaiveDateTime, Utc};
use mail_domain::id::account_id_from_uuid;
use mail_domain::{
    AccountCaps, AccountPlan, Address, DraftId, Identity, IdentityId, IsDefault, LabelId,
    LabelOrigin, MessageId, RemoteRef, ThreadId,
};
use porter_core::AccountId;
use rusqlite::{OptionalExtension, Row, params};

/// The columns of an identity, in the order [`identity_of`] reads them.
///
/// Its signature is the text of its default for new mail (`signatures`), else what the column on
/// `identities` held, which only a row written without going through `set_signature` still has.
const IDENTITY: &str = "id, account, from_name, from_email, reply_to,
    coalesce((SELECT s.body_text FROM signatures s
              WHERE s.identity = identities.id AND s.for_new = 1), signature),
    is_default";

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
    /// Every account, oldest first.
    ///
    /// A row whose id is not an id is left out: there is nothing to call it by.
    pub fn list_accounts(&self) -> Result<Vec<StoredAccount>, StoreError> {
        self.read_accounts("", [])
    }

    /// The account with this address. The address is compared as written.
    pub fn account_by_address(&self, address: &str) -> Result<Option<StoredAccount>, StoreError> {
        Ok(self
            .read_accounts("WHERE a.address = ?1", [address])?
            .into_iter()
            .next())
    }

    /// The account with this id.
    pub fn account(&self, id: AccountId) -> Result<Option<StoredAccount>, StoreError> {
        Ok(self
            .read_accounts("WHERE a.id = ?1", [id.to_string()])?
            .into_iter()
            .next())
    }

    fn read_accounts<P: rusqlite::Params>(
        &self,
        filter: &str,
        params: P,
    ) -> Result<Vec<StoredAccount>, StoreError> {
        let db = self.reader();
        let mut stmt = db.prepare(&format!(
            "SELECT a.id, a.address, a.plan, c.caps
             FROM accounts a LEFT JOIN account_caps c ON c.account = a.id
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
            let id = account_id_from_uuid(id);
            let caps =
                caps.map(|text| json("AccountCaps", &text).and_then(|caps| self.picked(&id, caps)));
            out.push(StoredAccount {
                id,
                plan: json("AccountPlan", &plan),
                caps,
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
        let db = self.touch(Topics::ACCOUNTS | Topics::IDENTITIES);
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
            // The signature an identity arrives with is its first in `signatures`, the default
            // for both; the column it would once have gone in is left empty.
            let added = tx.execute(
                "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                     is_default)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)
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
                    to_json("Identity.is_default", &identity.default)?,
                ],
            )?;
            let signed = identity
                .signature
                .as_deref()
                .filter(|text| !text.trim().is_empty());
            if let (1, Some(text)) = (added, signed) {
                tx.execute(
                    "INSERT INTO signatures
                         (id, identity, name, body_html, body_text, for_new, for_replies)
                     VALUES (?1, ?2, ?3, NULL, ?4, 1, 1)",
                    params![
                        mail_domain::SignatureId::generate().to_string(),
                        identity.id.to_string(),
                        mail_domain::UNNAMED_SIGNATURE,
                        text
                    ],
                )?;
            }
        }
        super::account_prefs::write_caps(&tx, new.id, new.caps, &new.at.to_rfc3339())?;
        tx.commit()?;
        Ok(())
    }

    /// Replace the account's plan. [`StoreError::NoAccount`] when there is no such account.
    pub fn set_account_plan(&self, id: AccountId, plan: &AccountPlan) -> Result<(), StoreError> {
        let changed = self.touch(Topics::ACCOUNTS).execute(
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
        stored
            .map(|text| json("AccountCaps", &text).and_then(|caps| self.picked(&id, caps)))
            .transpose()
    }

    /// The largest message, in octets on the wire, that `id`'s sending server said it takes.
    /// `None` when it stated none, or nothing has asked it yet.
    pub fn send_limit(&self, id: AccountId) -> Result<Option<u64>, StoreError> {
        let stored: Option<Option<i64>> = self
            .reader()
            .query_row(
                "SELECT send_limit FROM account_caps WHERE account = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(stored
            .flatten()
            // `put_send_limit` writes no negative number; one is read as no limit stated.
            .and_then(|octets| u64::try_from(octets).ok()))
    }

    /// Write down what `id`'s sending server said of its largest message: `None` when it stated
    /// none. Kept beside the capabilities, which a sync replaces without touching this.
    ///
    /// An account whose capabilities were never written has nowhere to keep it, and nothing is
    /// written: the next submission hears it again.
    pub fn put_send_limit(&self, id: AccountId, octets: Option<u64>) -> Result<(), StoreError> {
        let octets = octets.map(|octets| i64::try_from(octets).unwrap_or(i64::MAX));
        self.connection().execute(
            "UPDATE account_caps SET send_limit = ?2 WHERE account = ?1",
            params![id.to_string(), octets],
        )?;
        Ok(())
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

    /// Add an identity to its account: an alias the account also sends as.
    ///
    /// [`StoreError::NoAccount`] when the account is not here, and [`StoreError::AddressTaken`]
    /// when one of its identities already has the address, whatever its case; nothing is written
    /// for either.
    pub fn add_identity(&self, identity: &Identity) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let account = identity.account.to_string();
        let known: Option<String> = tx
            .query_row("SELECT id FROM accounts WHERE id = ?1", [&account], |r| {
                r.get(0)
            })
            .optional()?;
        if known.is_none() {
            return Err(StoreError::NoAccount(identity.account.clone()));
        }
        refuse_taken(&tx, &account, &identity.from.email, None)?;
        // As in `put_account`: the signature it arrives with is its first in `signatures`, the
        // default for both, and the column it would once have gone in is left empty.
        tx.execute(
            "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                 is_default)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6)",
            params![
                identity.id.to_string(),
                account,
                identity.from.name,
                identity.from.email,
                reply_to_column(identity.reply_to.as_ref())?,
                to_json("Identity.is_default", &identity.default)?,
            ],
        )?;
        if let Some(text) = identity
            .signature
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            tx.execute(
                "INSERT INTO signatures
                     (id, identity, name, body_html, body_text, for_new, for_replies)
                 VALUES (?1, ?2, ?3, NULL, ?4, 1, 1)",
                params![
                    mail_domain::SignatureId::generate().to_string(),
                    identity.id.to_string(),
                    mail_domain::UNNAMED_SIGNATURE,
                    text
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Change who an identity says it is: its name and address, and where replies to it go.
    ///
    /// Its signature, its account and whether it is the default are not this call's.
    /// [`StoreError::NoIdentity`] when there is no such identity, and
    /// [`StoreError::AddressTaken`] when another identity of its account has the address.
    pub fn edit_identity(
        &self,
        id: IdentityId,
        from: &Address,
        reply_to: Option<&Address>,
    ) -> Result<(), StoreError> {
        let db = self.touch(Topics::IDENTITIES);
        let tx = db.unchecked_transaction()?;
        let account: Option<String> = tx
            .query_row(
                "SELECT account FROM identities WHERE id = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(account) = account else {
            return Err(StoreError::NoIdentity(id));
        };
        refuse_taken(&tx, &account, &from.email, Some(id))?;
        tx.execute(
            "UPDATE identities SET from_name = ?2, from_email = ?3, reply_to = ?4 WHERE id = ?1",
            params![
                id.to_string(),
                from.name,
                from.email,
                reply_to_column(reply_to)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Remove an identity, handing what was written as it to its account's default.
    ///
    /// A draft or a template names the identity it is from, and that name is a foreign key: left
    /// as it is, it would keep the row from going, and a draft whose identity is gone cannot be
    /// sent. So they are moved to the default in the same transaction, which is the address a
    /// person who removed an alias would expect them to leave from. The default itself is not
    /// removed ([`StoreError::Conflict`]): an account without one has nothing to send as.
    /// [`StoreError::NoIdentity`] when there is no such identity.
    pub fn remove_identity(&self, id: IdentityId) -> Result<(), StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let gone = id.to_string();
        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT account, is_default FROM identities WHERE id = ?1",
                [&gone],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((account, default)) = row else {
            return Err(StoreError::NoIdentity(id));
        };
        if json::<IsDefault>("Identity.is_default", &default)? == IsDefault::Default {
            return Err(StoreError::Conflict(
                format!("identity {id} is its account's default").into(),
            ));
        }
        let heir: Option<String> = tx
            .query_row(
                "SELECT id FROM identities WHERE account = ?1 AND id != ?2
                 ORDER BY is_default DESC, id LIMIT 1",
                params![account, gone],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(heir) = heir {
            tx.execute(
                "UPDATE drafts SET identity = ?2 WHERE identity = ?1",
                params![gone, heir],
            )?;
            tx.execute(
                "UPDATE templates SET identity = ?2 WHERE identity = ?1",
                params![gone, heir],
            )?;
        }
        tx.execute("DELETE FROM identities WHERE id = ?1", [&gone])?;
        tx.commit()?;
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
    /// has one by that name, which is the answer a caller wants to give the user; and, on an
    /// account that keeps its labels on the server as keywords, when another label's name makes
    /// the same keyword (`Project X` and `Project_X`), which the server would take for one.
    pub fn create_label(&self, account: AccountId, name: &str) -> Result<LabelId, StoreError> {
        if self.labels_as_keywords(account.clone())?
            && let Some(keyword) = mail_domain::label_keyword(name)
            && self
                .keyed_labels(account.clone())?
                .iter()
                .any(|(_, held)| held.eq_ignore_ascii_case(&keyword))
        {
            return Err(StoreError::Conflict(
                format!("{name:?} makes the same keyword as another label").into(),
            ));
        }
        let id = LabelId::generate();
        self.touch(Topics::LABELS).execute(
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

    /// The thread id the server gave the message at `remote`, if it gave one.
    pub fn provider_thread_of(
        &self,
        account: AccountId,
        remote: &RemoteRef,
    ) -> Result<Option<String>, StoreError> {
        let (mailbox, uidvalidity, uid, _) = crate::remote_row::columns(remote);
        let (Some(uidvalidity), Some(uid)) = (uidvalidity, uid) else {
            return Ok(None);
        };
        Ok(self
            .reader()
            .query_row(
                "SELECT thrid FROM provider_thread
                 WHERE account = ?1 AND mailbox = ?2 AND uidvalidity = ?3 AND uid = ?4",
                params![account.to_string(), mailbox, uidvalidity, uid],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// The conversation a stored message of `account` is in that the server gave this thread id.
    pub fn thread_by_provider_thread(
        &self,
        account: AccountId,
        thrid: &str,
    ) -> Result<Option<ThreadId>, StoreError> {
        self.reader()
            .query_row(
                "SELECT m.thread FROM provider_thread p
                 JOIN remote_map r ON r.account = p.account AND r.mailbox = p.mailbox
                  AND r.uidvalidity = p.uidvalidity AND r.uid = p.uid
                 JOIN messages m ON m.id = r.message
                 WHERE p.account = ?1 AND p.thrid = ?2 LIMIT 1",
                params![account.to_string(), thrid],
                |r| r.get::<_, String>(0),
            )
            .optional()?
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
            .reader()
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
        let db = self.touch(Topics::ACCOUNTS);
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
        self.touch(Topics::FOLLOW_UPS).execute(
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
        self.touch(Topics::FOLLOW_UPS).execute(
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

/// [`StoreError::AddressTaken`] when an identity of `account` other than `except` already sends
/// as `email`, compared as the lookups compare it: without case.
fn refuse_taken(
    db: &rusqlite::Connection,
    account: &str,
    email: &str,
    except: Option<IdentityId>,
) -> Result<(), StoreError> {
    // No identity is the empty id, so with none to except every row is compared.
    let except = except.map(|id| id.to_string()).unwrap_or_default();
    let taken: Option<String> = db
        .query_row(
            "SELECT id FROM identities
             WHERE account = ?1 AND lower(from_email) = lower(?2) AND id != ?3 LIMIT 1",
            params![account, email.trim(), except],
            |r| r.get(0),
        )
        .optional()?;
    match taken {
        Some(_) => Err(StoreError::AddressTaken(email.trim().to_owned())),
        None => Ok(()),
    }
}

/// An identity's `reply_to` column: `NULL` for none, else the address as JSON, as
/// [`SqliteStore::upsert_account`] writes it.
fn reply_to_column(reply_to: Option<&Address>) -> Result<Option<String>, StoreError> {
    reply_to
        .map(|reply_to| to_json("Identity.reply_to", &Some(reply_to)))
        .transpose()
}

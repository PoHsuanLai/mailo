//! The junk filter's word counts (migration 0044). See [`crate::junk`].

use super::SqliteStore;
use crate::StoreError;
use crate::junk::{Judged, JunkCounts, Learned};
use mail_domain::MessageId;
use porter_core::AccountId;
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    /// Count `words`, the distinct words of `message` on `account`, as `judged`. A message
    /// already learned so changes nothing; one learned the other way is taken back first, so
    /// changing one's mind about a message never counts it twice.
    pub fn junk_learn(
        &self,
        account: AccountId,
        message: MessageId,
        words: &[String],
        judged: Judged,
    ) -> Result<Learned, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let prior: Option<String> = tx
            .query_row(
                "SELECT verdict FROM junk_learned WHERE message = ?1",
                params![message.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if prior.as_deref() == Some(judged.as_str()) {
            return Ok(Learned::Unchanged);
        }
        let account = account.to_string();
        let column = |judged: Judged| match judged {
            Judged::Spam => "spam",
            Judged::Ham => "ham",
        };
        let learned = match prior {
            Some(_) => {
                let other = column(match judged {
                    Judged::Spam => Judged::Ham,
                    Judged::Ham => Judged::Spam,
                });
                for word in words {
                    tx.execute(
                        &format!(
                            "UPDATE junk_words SET {other} = max({other} - 1, 0)
                             WHERE account = ?1 AND word = ?2"
                        ),
                        params![account, word],
                    )?;
                }
                tx.execute(
                    &format!(
                        "UPDATE junk_totals SET {other} = max({other} - 1, 0) WHERE account = ?1"
                    ),
                    params![account],
                )?;
                Learned::Turned
            }
            None => Learned::New,
        };
        let this = column(judged);
        for word in words {
            tx.execute(
                &format!(
                    "INSERT INTO junk_words (account, word, {this}) VALUES (?1, ?2, 1)
                     ON CONFLICT (account, word) DO UPDATE SET {this} = {this} + 1"
                ),
                params![account, word],
            )?;
        }
        tx.execute(
            &format!(
                "INSERT INTO junk_totals (account, {this}) VALUES (?1, 1)
                 ON CONFLICT (account) DO UPDATE SET {this} = {this} + 1"
            ),
            params![account],
        )?;
        tx.execute(
            "INSERT INTO junk_learned (message, verdict) VALUES (?1, ?2)
             ON CONFLICT (message) DO UPDATE SET verdict = excluded.verdict",
            params![message.to_string(), judged.as_str()],
        )?;
        tx.commit()?;
        Ok(learned)
    }

    /// What `account`'s filter knows of `words`: its totals, and each word it has seen.
    pub fn junk_counts(
        &self,
        account: AccountId,
        words: &[String],
    ) -> Result<JunkCounts, StoreError> {
        let db = self.reader();
        let account = account.to_string();
        let (spam, ham): (i64, i64) = db
            .query_row(
                "SELECT spam, ham FROM junk_totals WHERE account = ?1",
                params![account],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or((0, 0));
        let mut stmt =
            db.prepare_cached("SELECT spam, ham FROM junk_words WHERE account = ?1 AND word = ?2")?;
        let mut seen = Vec::new();
        for word in words {
            let counted: Option<(i64, i64)> = stmt
                .query_row(params![account, word], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            if let Some((s, h)) = counted {
                seen.push((word.clone(), count(s), count(h)));
            }
        }
        Ok(JunkCounts {
            spam: count(spam),
            ham: count(ham),
            words: seen,
        })
    }
}

/// A stored count, which is never below nought.
fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

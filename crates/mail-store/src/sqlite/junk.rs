//! The junk filter's word counts (migration 0047). See [`crate::junk`].

use super::SqliteStore;
use crate::StoreError;
use crate::junk::{Judged, JunkCounts, Learned};
use mail_domain::MessageId;
use porter_core::AccountId;
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;

/// A message's learned words as the column keeps them: one per line. A word is letters and
/// digits, or `from:` and a domain, so none holds a line break.
fn joined(words: &[String]) -> String {
    words.join("\n")
}

fn split(stored: &str) -> Vec<String> {
    stored.lines().map(str::to_owned).collect()
}

/// The column of `judged` in `junk_words` and `junk_totals`.
fn column(judged: Judged) -> &'static str {
    match judged {
        Judged::Spam => "spam",
        Judged::Ham => "ham",
    }
}

/// Take `words` and one message back out of `judged`'s counts for `account`.
fn take_back(
    tx: &rusqlite::Transaction<'_>,
    account: &str,
    words: &[String],
    judged: Judged,
) -> Result<(), StoreError> {
    let col = column(judged);
    let mut stmt = tx.prepare_cached(&format!(
        "UPDATE junk_words SET {col} = max({col} - 1, 0) WHERE account = ?1 AND word = ?2"
    ))?;
    for word in words {
        stmt.execute(params![account, word])?;
    }
    tx.execute(
        &format!("UPDATE junk_totals SET {col} = max({col} - 1, 0) WHERE account = ?1"),
        params![account],
    )?;
    Ok(())
}

impl SqliteStore {
    /// Count `words`, the distinct words of `message` on `account`, as `judged`. A message
    /// already learned so changes nothing; one learned the other way is taken back first, by the
    /// words it was learned with and not by `words`, so changing one's mind about a message
    /// never counts it twice, whatever it says by then.
    pub fn junk_learn(
        &self,
        account: AccountId,
        message: MessageId,
        words: &[String],
        judged: Judged,
    ) -> Result<Learned, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT verdict, words FROM junk_learned WHERE message = ?1",
                params![message.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if prior.as_ref().map(|(verdict, _)| verdict.as_str()) == Some(judged.as_str()) {
            return Ok(Learned::Unchanged);
        }
        let account = account.to_string();
        let learned = match prior {
            Some((_, taught)) => {
                let other = match judged {
                    Judged::Spam => Judged::Ham,
                    Judged::Ham => Judged::Spam,
                };
                take_back(&tx, &account, &split(&taught), other)?;
                Learned::Turned
            }
            None => Learned::New,
        };
        let this = column(judged);
        {
            let mut stmt = tx.prepare_cached(&format!(
                "INSERT INTO junk_words (account, word, {this}) VALUES (?1, ?2, 1)
                 ON CONFLICT (account, word) DO UPDATE SET {this} = {this} + 1"
            ))?;
            for word in words {
                stmt.execute(params![account, word])?;
            }
        }
        tx.execute(
            &format!(
                "INSERT INTO junk_totals (account, {this}) VALUES (?1, 1)
                 ON CONFLICT (account) DO UPDATE SET {this} = {this} + 1"
            ),
            params![account],
        )?;
        tx.execute(
            "INSERT INTO junk_learned (message, verdict, words) VALUES (?1, ?2, ?3)
             ON CONFLICT (message) DO UPDATE SET verdict = excluded.verdict,
                                                 words = excluded.words",
            params![message.to_string(), judged.as_str(), joined(words)],
        )?;
        tx.commit()?;
        Ok(learned)
    }

    /// Take back what `message` taught `account`'s filter, when it was learned as `judged`: the
    /// words it was learned with are subtracted and it counts as not learned. `false` when it
    /// was not learned, or was learned the other way, which is then left as it is.
    pub fn junk_forget(
        &self,
        account: AccountId,
        message: MessageId,
        judged: Judged,
    ) -> Result<bool, StoreError> {
        let db = self.connection();
        let tx = db.unchecked_transaction()?;
        let learned: Option<String> = tx
            .query_row(
                "SELECT words FROM junk_learned WHERE message = ?1 AND verdict = ?2",
                params![message.to_string(), judged.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        let Some(taught) = learned else {
            return Ok(false);
        };
        take_back(&tx, &account.to_string(), &split(&taught), judged)?;
        tx.execute(
            "DELETE FROM junk_learned WHERE message = ?1",
            params![message.to_string()],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// What `account`'s filter knows of `words`: its totals, and each word it has seen. The
    /// words are looked up [`LOOKUP_CHUNK`] at a time.
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
        let mut counted: HashMap<String, (i64, i64)> = HashMap::new();
        for chunk in words.chunks(LOOKUP_CHUNK) {
            let marks = vec!["?"; chunk.len()].join(",");
            let mut stmt = db.prepare(&format!(
                "SELECT word, spam, ham FROM junk_words WHERE account = ?1 AND word IN ({marks})"
            ))?;
            let args = std::iter::once(&account).chain(chunk);
            let rows = stmt.query_map(rusqlite::params_from_iter(args), |row| {
                Ok((row.get::<_, String>(0)?, (row.get(1)?, row.get(2)?)))
            })?;
            for row in rows {
                let (word, seen) = row?;
                counted.insert(word, seen);
            }
        }
        let seen = words
            .iter()
            .filter_map(|word| {
                counted
                    .get(word)
                    .map(|(s, h)| (word.clone(), count(*s), count(*h)))
            })
            .collect();
        Ok(JunkCounts {
            spam: count(spam),
            ham: count(ham),
            words: seen,
        })
    }
}

/// How many words one `junk_counts` query asks about: well under SQLite's limit on variables.
const LOOKUP_CHUNK: usize = 500;

/// A stored count, which is never below nought.
fn count(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

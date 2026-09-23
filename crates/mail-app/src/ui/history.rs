//! Who has written, how often, when last, and whether you have written back.
//!
//! One grouped read of the `messages` table, the way `data.rs` reads `accounts`: no new `Store`
//! method, nothing written. It is asked once per list revision and shared, never per hover, and
//! the Ctrl T menu ranks people from the same answer.

use crate::search::{Affinity, SenderStats};
use chrono::{DateTime, Utc};
use mail_store::SqliteStore;
use std::collections::{HashMap, HashSet};

/// One sender, as the sender card shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sender {
    /// The display name they used most recently, when they gave one.
    pub name: Option<String>,
    /// Conversations with a message from them.
    pub threads: u32,
    /// Their newest message.
    pub last: DateTime<Utc>,
    /// Whether one of your accounts has sent them anything.
    pub replied: bool,
}

/// Every sender the store holds mail from, keyed by lower-cased address.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct History {
    by_email: HashMap<String, Sender>,
}

impl History {
    /// The history of `email`, matched ASCII-case-insensitively.
    pub(super) fn get(&self, email: &str) -> Option<&Sender> {
        self.by_email.get(&email.to_ascii_lowercase())
    }

    /// The ranker's view of the same history.
    pub(super) fn affinity(&self) -> Affinity {
        let mut affinity = Affinity::default();
        for (email, sender) in &self.by_email {
            affinity.insert(
                email,
                SenderStats {
                    threads: sender.threads,
                    replied: sender.replied,
                },
            );
        }
        affinity
    }

    /// Display names by address, for the menu's People rows.
    pub(super) fn names(&self) -> HashMap<String, String> {
        self.by_email
            .iter()
            .filter_map(|(email, sender)| Some((email.clone(), sender.name.clone()?)))
            .collect()
    }
}

/// Group every message by sender. Two statements: the grouped count, and the addresses your
/// own accounts have written to. An error reads as an empty history — the cards then say
/// "first mail", which is the cautious thing for them to say.
pub(super) fn history(store: &SqliteStore) -> History {
    let db = store.connection();
    let written = written_to(&db);
    // With one `MAX()` in the select list, SQLite takes the bare `from_name` from the row that
    // holds the maximum, which is the name on their newest message.
    let Ok(mut stmt) = db.prepare(
        "SELECT lower(from_email), COUNT(DISTINCT thread), MAX(date), from_name
           FROM messages
          GROUP BY lower(from_email)",
    ) else {
        return History::default();
    };
    let Ok(rows) = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    }) else {
        return History::default();
    };
    let by_email = rows
        .filter_map(Result::ok)
        .filter_map(|(email, threads, last, name)| {
            let last = DateTime::parse_from_rfc3339(&last)
                .ok()?
                .with_timezone(&Utc);
            let replied = written.contains(&email);
            let name = name.filter(|name| !name.trim().is_empty());
            Some((
                email,
                Sender {
                    name,
                    threads,
                    last,
                    replied,
                },
            ))
        })
        .collect();
    History { by_email }
}

/// Addresses on the To or Cc of anything sent from one of your accounts.
fn written_to(db: &rusqlite::Connection) -> HashSet<String> {
    let Ok(mut stmt) = db.prepare(
        "SELECT DISTINCT lower(json_extract(r.value, '$.email'))
           FROM messages m, json_each(m.recipients, '$.to') r
          WHERE lower(m.from_email) IN (SELECT lower(address) FROM accounts)
         UNION
         SELECT DISTINCT lower(json_extract(r.value, '$.email'))
           FROM messages m, json_each(m.recipients, '$.cc') r
          WHERE lower(m.from_email) IN (SELECT lower(address) FROM accounts)",
    ) else {
        return HashSet::new();
    };
    stmt.query_map([], |row| row.get::<_, Option<String>>(0))
        .map(|rows| rows.filter_map(Result::ok).flatten().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;

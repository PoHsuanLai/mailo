//! Who has written, how often, when last, and whether you have written back.
//!
//! One grouped read of the client ([`mail_client::Stored::sender_history`]): nothing written.
//! It is asked once per move of the store and shared, never per hover, and the search bar ranks
//! people from the same answer.

use chrono::{DateTime, Utc};
use mail_client::Client;
use mail_client::search::{Affinity, SenderStats};
use std::collections::HashMap;

/// One sender, as the sender card shows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sender {
    /// The display name they used most recently, when they gave one.
    pub name: Option<String>,
    /// Conversations with a message from them.
    pub threads: u32,
    /// Their newest message.
    pub last: DateTime<Utc>,
    /// Whether one of your accounts has sent them anything.
    pub replied: bool,
}

/// Every sender the client holds mail from, keyed by lower-cased address.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct History {
    by_email: HashMap<String, Sender>,
}

impl History {
    /// The history of `email`, matched ASCII-case-insensitively.
    pub fn get(&self, email: &str) -> Option<&Sender> {
        self.by_email.get(&email.to_ascii_lowercase())
    }

    /// The ranker's view of the same history.
    pub fn affinity(&self) -> Affinity {
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
    pub fn names(&self) -> HashMap<String, String> {
        self.by_email
            .iter()
            .filter_map(|(email, sender)| Some((email.clone(), sender.name.clone()?)))
            .collect()
    }
}

/// Every sender the client holds mail from, with the addresses your own accounts have written
/// to. An error reads as an empty history — the cards then say
/// "first mail", which is the cautious thing for them to say.
pub fn history(client: &Client) -> History {
    let by_email = client
        .stored()
        .sender_history()
        .unwrap_or_default()
        .into_iter()
        .map(|record| {
            let name = record.name.filter(|name| !name.trim().is_empty());
            (
                record.email,
                Sender {
                    name,
                    threads: record.threads,
                    last: record.last,
                    replied: record.replied,
                },
            )
        })
        .collect();
    History { by_email }
}

//! Searching the server: what a search found, and how it is kept.
//!
//! A search typed here runs over the store. What the store never held — mail older than a sync
//! reached, in folders nobody follows, on Gmail anywhere but the inbox and Sent — only the
//! server can find. Each engine asks its server in its own protocol
//! ([`mail_proto::search`] says how and what cannot be asked), and what comes back is kept the
//! way a sync keeps a new message: its headers fetched by the header path every sync uses, and
//! absorbed into the store, so it lists, opens and threads as any other message does. Its body
//! is not fetched for the search, and nothing remote in it is loaded: opening it later fetches
//! the body as opening any headers-only message does.
//!
//! What was held already is not fetched again, and a message new to this computer is marked
//! ([`mail_store::Store::mark_found`]) so the list can say where it came from.

use mail_domain::MessageId;
pub use mail_proto::search::Unsaid;

/// Most messages one search of the server brings here: the newest it found. The rest are
/// counted in [`ServerHits::more`], not fetched.
pub const SERVER_HITS: usize = 50;

/// What a search of the server found, now held here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerHits {
    /// Every message the server named that is held here now, newest first by the server's
    /// order, each once: those already held, and those this search brought.
    pub messages: Vec<MessageId>,
    /// How many of them this search brought: headers fetched, messages new to this computer.
    pub fetched: usize,
    /// How many more the server matched than were taken, at least. Graph says only whether
    /// there is another page, which counts as one.
    pub more: u64,
}

impl ServerHits {
    /// Add `held` in order, each message once.
    pub(crate) fn hold(&mut self, held: impl IntoIterator<Item = MessageId>) {
        for id in held {
            if !self.messages.contains(&id) {
                self.messages.push(id);
            }
        }
    }
}

/// What asking the server came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Searched {
    /// The server was asked, or nothing needed asking.
    Found(ServerHits),
    /// Nothing was sent: these parts of the query cannot be asked of this server faithfully.
    Unsaid(Unsaid),
}

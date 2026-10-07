//! Tokio runtime: the only crate that opens sockets, spawns tasks, or reads the clock.
//!
//! Everything below returns `IoNeed` and gets fed `IoReady`. This crate owns the loop that
//! satisfies those, which is what makes cancellation expressible at all — see [`drive`].

pub mod account_secrets;
pub mod assemble;
pub mod authorize;
pub mod bimi;
pub mod carddav;
pub mod clients;
pub mod drive;
pub mod engine;
pub mod epoch;
pub mod error;
pub mod graph;
pub mod http;
pub mod jmap;
pub mod link;
pub mod lookup;
pub mod pgp;
pub mod places;
pub mod reparse;
pub mod search;
pub mod sieve;
pub mod signing_store;
pub mod smime;
pub mod tokens;
pub mod transport;
pub mod unsubscribe;
pub mod wanted;
pub mod wkd;

pub use account_secrets::{
    AccountSecrets, PlatformSecrets, block_on, own_secrets, platform_secrets,
};
pub use assemble::{Arrival, Destination, absorb, absorb_into, assemble};
pub use drive::{Cancel, drive};
pub use engine::{AccountEngine, PartBudget, SyncReport, Woke};
pub use error::RuntimeError;
pub use jmap::JmapEngine;
pub use link::{Accountd, Link};
pub use porter_oauth::ClientRegistry;
pub use reparse::reparse_queued;
pub use search::{SERVER_HITS, Searched, ServerHits, Unsaid};
pub use signing_store::{KeyringSigningStore, MapSigningStore, SigningStore, off_runtime};
pub use tokens::{AfterRefusal, Held, OAuthTokens, Token, TokenSource};
pub use transport::Transport;

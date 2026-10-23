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
pub mod fetch;
pub mod graph;
pub mod http;
pub mod jmap;
pub mod lookup;
pub mod no_keyring;
pub mod pgp;
pub mod places;
pub mod quarantine;
pub mod remote_image;
pub mod reparse;
pub mod schedule;
pub mod search;
pub mod sending;
pub mod sieve;
pub mod signing_store;
pub mod smime;
#[cfg(feature = "testing")]
pub mod testing;
pub mod tokens;
pub mod transport;
pub mod unsubscribe;
pub mod wanted;
pub mod wkd;

pub use account_secrets::{
    AccountSecrets, PlatformSecrets, SecretTrouble, block_on, platform_secrets,
};
pub use assemble::{Absorbed, Arrival, Destination, absorb, absorb_counted, absorb_into, assemble};
pub use drive::{Cancel, drive, drive_within};
pub use engine::{AccountEngine, IDLE_REARM, PartBudget, SyncReport, Woke};
pub use error::{Failure, Logged, RuntimeError, Source};
pub use jmap::JmapEngine;
pub use no_keyring::NoKeyring;
pub use porter_oauth::ClientRegistry;
pub use reparse::reparse_queued;
pub use search::{SERVER_HITS, Searched, ServerHits, Unsaid};
pub use signing_store::{KeyringSigningStore, SigningStore, off_runtime};
pub use tokens::{AfterRefusal, Held, OAuthTokens, Token, TokenSource};
pub use transport::Transport;

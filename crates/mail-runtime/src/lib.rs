//! Tokio runtime: the only crate that opens sockets, spawns tasks, or reads the clock.
//!
//! Everything below returns `IoNeed` and gets fed `IoReady`. This crate owns the loop that
//! satisfies those, which is what makes cancellation expressible at all — see [`drive`].

pub mod assemble;
pub mod bimi;
pub mod carddav;
pub mod drive;
pub mod engine;
pub mod epoch;
pub mod error;
pub mod fetch;
pub mod graph;
pub mod jmap;
pub mod pgp;
pub mod quarantine;
pub mod reparse;
pub mod schedule;
pub mod search;
pub mod sending;
pub mod server_drafts;
pub mod sieve;
pub mod smime;
pub mod unsubscribe;
pub mod wanted;

pub use assemble::{Absorbed, Arrival, Destination, absorb, absorb_counted, absorb_into, assemble};
pub use drive::{Cancel, drive, drive_within};
pub use engine::{AccountEngine, IDLE_REARM, Notify, PartBudget, SyncReport, Woke};
pub use error::{Logged, RuntimeError};
pub use jmap::JmapEngine;
pub use reparse::reparse_queued;
pub use search::{SERVER_HITS, Searched, ServerHits, Unsaid};

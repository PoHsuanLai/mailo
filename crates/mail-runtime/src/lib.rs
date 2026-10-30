//! The protocol drivers: what an account's server is asked, over `mail-net`'s connections and
//! `mail-secrets`' credentials, and what comes back is put in the store.
//!
//! Everything below returns `IoNeed` and gets fed `IoReady`. [`drive`] is the loop that satisfies
//! those, which is what makes cancellation expressible at all. IMAP's pass (`fetch`), Graph, JMAP,
//! CardDAV, ManageSieve, submission and the lookups (BIMI, keys) are here; the engines that run
//! them on a schedule are `mail-engine`'s, over this crate, so nothing here names an engine.

pub mod assemble;
pub mod bimi;
pub mod carddav;
pub mod drive;
pub mod epoch;
pub mod error;
pub mod fetch;
pub mod graph;
pub mod jmap;
pub mod pass;
pub mod pgp;
pub mod quarantine;
pub mod reparse;
pub mod search;
pub mod sending;
pub mod server_drafts;
pub mod sieve;
pub mod smime;
pub mod unsubscribe;
pub mod wanted;

pub use assemble::{Absorbed, Arrival, Destination, absorb, absorb_counted, absorb_into, assemble};
pub use drive::{Cancel, drive, drive_within};
pub use error::{Logged, RuntimeError};
pub use jmap::JmapEngine;
pub use pass::{IDLE_REARM, SyncReport, Woke};
pub use reparse::reparse_queued;
pub use search::{SERVER_HITS, Searched, ServerHits, Unsaid};

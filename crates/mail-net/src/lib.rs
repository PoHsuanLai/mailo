//! The wire: connections, TLS, DNS and HTTP clients, and the lookups that are only a fetch.
//!
//! Nothing here knows the store, an account's secrets or a protocol's conversation. A
//! [`Transport`] is a socket (or a relay's stream) a sans-I/O machine is driven over; the
//! clients are the HTTPS ones every driver and lookup builds from, with their limits and their
//! roots; the lookups (Web Key Directory, remote images) are one fetch each.

pub(crate) mod deflate;
pub mod error;
pub mod http;
pub mod lookup;
pub mod public;
pub mod remote_image;
pub mod roots;
#[cfg(feature = "testing")]
pub mod testing;
pub mod transport;
pub mod wkd;

pub use error::{Failure, NetError, Source, within};
pub use transport::Transport;

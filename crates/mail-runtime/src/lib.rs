//! Tokio runtime: the only crate that opens sockets, spawns tasks, or reads the clock.
//!
//! Everything below returns `IoNeed` and gets fed `IoReady`. This crate owns the loop that
//! satisfies those, which is what makes cancellation expressible at all — see [`drive`].

pub mod assemble;
pub mod carddav;
pub mod drive;
pub mod engine;
pub mod error;
pub mod graph;
pub mod loopback;
pub mod oauth;
pub mod renewal;
pub mod reparse;
pub mod secrets;
pub mod signin;
pub mod transport;
pub mod unsubscribe;

pub use assemble::{Arrival, Destination, absorb, absorb_into, assemble};
pub use drive::{Cancel, drive};
pub use engine::{AccountEngine, SyncReport};
pub use error::RuntimeError;
pub use loopback::Loopback;
pub use renewal::{AfterRefusal, Held, Renewal, Token};
pub use reparse::reparse_queued;
pub use secrets::{KeyringSecrets, MapSecrets, Secrets};
pub use signin::{OAuthRegistry, Registration};
pub use transport::Transport;

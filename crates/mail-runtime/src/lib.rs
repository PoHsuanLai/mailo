//! Tokio runtime: the only crate that opens sockets, spawns tasks, or reads the clock.
//!
//! Everything below returns `IoNeed` and gets fed `IoReady`. This crate owns the loop that
//! satisfies those, which is what makes cancellation expressible at all — see [`drive`].

pub mod assemble;
pub mod drive;
pub mod engine;
pub mod error;
pub mod loopback;
pub mod oauth;
pub mod secrets;
pub mod transport;

pub use assemble::{Arrival, absorb, assemble};
pub use drive::{Cancel, drive};
pub use engine::{AccountEngine, SyncReport};
pub use error::RuntimeError;
pub use loopback::Loopback;
pub use secrets::{KeyringSecrets, MapSecrets, Secrets};
pub use transport::Transport;

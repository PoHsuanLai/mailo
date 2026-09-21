//! Tokio runtime: the only crate that opens sockets, spawns tasks, or reads the clock.
//!
//! Everything below returns `IoNeed` and gets fed `IoReady`. This crate owns the loop that
//! satisfies those, which is what makes cancellation expressible at all — see [`drive`].

pub mod drive;
pub mod error;
pub mod transport;

pub use drive::{Cancel, drive};
pub use error::RuntimeError;
pub use transport::Transport;

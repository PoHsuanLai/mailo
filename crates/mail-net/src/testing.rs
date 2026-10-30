//! What a test swaps in at the wire: an in-memory transport, and HTTPS listeners to point a
//! client at.
//!
//! Behind the `testing` feature, which a crate enables for its tests and the program never does.

pub mod tls;

use crate::Transport;
use porter_client::AuthenticatedStream;
use porter_core::stream::{DuplexEnd, duplex};

/// A connection that goes nowhere: the [`Transport`] a machine is driven over, and the far end a
/// test plays the server on. The transport is taken as already secure and already signed in, so
/// the machine over it must not authenticate, which is what a test of the engines wants.
pub fn memory_transport() -> (Transport, DuplexEnd) {
    let (app, server) = duplex(64 * 1024);
    let transport = Transport::relayed(AuthenticatedStream::Memory(app))
        .expect("an in-memory stream needs no socket");
    (transport, server)
}

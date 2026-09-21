//! An in-memory [`Store`], for tests and for the parity proptest.
//!
//! Its query path is implemented *by calling* [`mail_domain::Filter::fit`] — never by a second
//! hand-written matcher. A second matcher would be a third implementation of the same
//! semantics, and the parity test would then be comparing two wrongs.

/// Everything held in memory. Cheap to construct, and never touches the disk.
#[derive(Debug, Default)]
pub struct MemoryStore {
    // wave 2
}

impl MemoryStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

// `impl Store for MemoryStore` is wave 2's job; an empty impl block does not compile.

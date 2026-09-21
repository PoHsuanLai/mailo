//! Sans-I/O protocol machines: IMAP, POP3, SMTP, OAuth, and the backends that drive them.
//!
//! Nothing here opens a socket, spawns a task, or reads a clock. A machine is fed bytes and
//! returns what it needs next; `mail-runtime` owns the loop that satisfies those needs. Tests
//! are byte transcripts — see `tests/traces/FORMAT.md`.

pub mod machine;
pub mod pop3;

pub use machine::{Backend, IoNeed, IoReady, Machine, Progress, ProtoError, ProtoOutcome};
pub use pop3::{ListEntry, Pop3Command, Pop3Reply, Pop3Session, UidlEntry};

#[cfg(test)]
mod tests {
    // The replay harness lives in tests/replay.rs; see tests/traces/FORMAT.md.
}

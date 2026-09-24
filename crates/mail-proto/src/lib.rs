//! Sans-I/O protocol machines: IMAP, POP3, SMTP, OAuth, and the backends that drive them; and
//! what an account-discovery answer means (`discover`).
//!
//! Nothing here opens a socket, spawns a task, or reads a clock. A machine is fed bytes and
//! returns what it needs next; `mail-runtime` owns the loop that satisfies those needs. Tests
//! are byte transcripts — see `tests/traces/FORMAT.md`.

pub mod backend;
pub mod diagnose;
pub mod discover;
pub mod imap;
pub mod machine;
pub mod mutf7;
pub mod pop3;
pub mod sieve;
pub mod smtp;

pub use diagnose::{explain, explain_text};
pub use imap::{
    Completed, ImapAuth, ImapCommand, ImapSession, ImapTranscript, Untagged, has_capability,
};
pub use machine::{Backend, IoNeed, IoReady, Machine, Progress, ProtoError, ProtoOutcome, Refusal};
pub use pop3::{ListEntry, Pop3Command, Pop3Reply, Pop3Session, UidlEntry};
pub use smtp::{
    Advertised, EhloExtensions, ReplyText, SizeLimit, SmtpReply, SmtpSession, Submission,
};

#[cfg(test)]
mod tests {
    // The replay harness lives in tests/replay.rs; see tests/traces/FORMAT.md.
}

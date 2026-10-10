//! Sans-I/O protocol machines: IMAP, POP3, SMTP, OAuth, and the backends that drive them; JMAP's
//! requests and responses as values (`jmap`); a search typed here as each protocol asks one
//! (`search`).
//!
//! Nothing here opens a socket, spawns a task, or reads a clock. A machine is fed bytes and
//! returns what it needs next; `mail-runtime` owns the loop that satisfies those needs. Tests
//! are byte transcripts — see `tests/traces/FORMAT.md`.

pub mod backend;
pub mod diagnose;
pub mod error;
pub mod imap;
pub mod jmap;
pub mod machine;
pub mod mutf7;
pub mod pop3;
pub mod search;
pub mod sieve;
pub mod smtp;

pub use diagnose::{Diagnosis, diagnose, diagnose_text};
pub use error::{ProtoError, Refusal};
pub use imap::{
    Access, Completed, ImapAuth, ImapCommand, ImapSession, ImapTranscript, Untagged, has_capability,
};
pub use machine::{Backend, IoNeed, IoReady, Machine, Moved, Progress, ProtoOutcome};
pub use pop3::{ListEntry, Pop3Command, Pop3Reply, Pop3Session, UidlEntry};
pub use smtp::{
    Advertised, Authentication, EhloExtensions, ReplyText, SignIn, SizeLimit, SmtpReply,
    SmtpSession, Submission,
};

#[cfg(test)]
mod tests {
    // The replay harness lives in tests/common/mod.rs; see tests/traces/FORMAT.md.
}

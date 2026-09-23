//! Backends: `ProtoOp` in, domain values out.
//!
//! A backend is **one connection**. POP3 and SMTP are different connections to different hosts,
//! so submission is not part of an incoming backend — the runtime routes `ProtoOp::Submit` to a
//! submitting backend of its own. The plan originally described `ImapBackend` as "IMAP session
//! plus SMTP submit", which would have put two sockets behind one `Machine` and made the drive
//! loop's single-transport shape a lie.

mod folders;
pub mod imap;
pub mod pop3;
pub mod smtp;

pub use imap::ImapBackend;
pub use pop3::Pop3Backend;
pub use smtp::SmtpBackend;

/// Whether a command walk needs the account's authentication in front of it.
///
/// Shared by POP3 and IMAP because it is one question, asked of two protocols. It lives here
/// rather than in either backend so that neither owns the other's vocabulary.
///
/// The answer belongs to the session factory in both cases: the factory is the only thing
/// holding the credential, and therefore the only thing that can know whether this account logs
/// in with a password or a bearer token. `ImapBackend` used to decide for itself — it named
/// `AUTHENTICATE XOAUTH2` at eleven call sites — which meant password IMAP could not work
/// through it however well `ImapSession` supported `LOGIN`, and every non-Gmail server is
/// password IMAP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Authenticate {
    /// Prepend the account's auth commands before these.
    First,
    /// Run these as given; the walk authenticates itself, or does not need to.
    No,
}

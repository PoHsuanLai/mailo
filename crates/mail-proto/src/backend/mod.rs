//! Backends: `ProtoOp` in, domain values out.
//!
//! A backend is **one connection**. POP3 and SMTP are different connections to different hosts,
//! so submission is not part of an incoming backend — the runtime routes `ProtoOp::Submit` to a
//! submitting backend of its own. The plan originally described `ImapBackend` as "IMAP session
//! plus SMTP submit", which would have put two sockets behind one `Machine` and made the drive
//! loop's single-transport shape a lie.

pub mod imap;
pub mod pop3;
pub mod smtp;

pub use imap::ImapBackend;
pub use pop3::{Authenticate, Pop3Backend};
pub use smtp::SmtpBackend;

//! A command line that could not be read, with the words that say what to type instead.
//!
//! This is about the terminal's own grammar, so it lives with the command line and not in
//! `mail-core`, which knows no commands.

use std::fmt;

/// A command line that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageError {
    /// A command with nothing after it: the synopsis alone.
    Synopsis(&'static str),
    Missing {
        command: &'static str,
        verb: String,
        what: &'static str,
        usage: &'static str,
    },
    UnknownOption {
        option: String,
        usage: &'static str,
    },
    UnknownCommand {
        command: &'static str,
        verb: String,
        usage: &'static str,
    },
    NotAMessageId {
        raw: String,
        usage: &'static str,
    },
    /// The domain's own words about a fingerprint that would not read.
    PgpFingerprint(mail_domain::ParseFingerprintError),
    SmimeFingerprint(mail_domain::ParseFingerprintError),
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UsageError::Synopsis(usage) => write!(f, "{usage}"),
            UsageError::Missing {
                command,
                verb,
                what,
                usage,
            } => write!(f, "{command} {verb} needs {what}\n\n{usage}"),
            UsageError::UnknownOption { option, usage } => {
                write!(f, "unknown option {option:?}\n\n{usage}")
            }
            UsageError::UnknownCommand {
                command,
                verb,
                usage,
            } => write!(f, "unknown {command} command {verb:?}\n\n{usage}"),
            UsageError::NotAMessageId { raw, usage } => {
                write!(f, "{raw:?} is not a message id\n\n{usage}")
            }
            UsageError::PgpFingerprint(why) => write!(f, "{why}; `mailo pgp keys` lists them"),
            UsageError::SmimeFingerprint(why) => {
                write!(f, "{why}; `mailo smime list` lists them")
            }
        }
    }
}

impl std::error::Error for UsageError {}

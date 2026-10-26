//! The local junk filter's memory: word counts per account, learned from mail the user called
//! spam or not spam (migration 0044). What the counts mean is `mail_core::junk`'s; this keeps
//! them, and keeps a message from being counted twice.

/// What a message was learned as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Judged {
    Spam,
    Ham,
}

impl Judged {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Judged::Spam => "spam",
            Judged::Ham => "ham",
        }
    }
}

/// What learning one message did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Learned {
    /// It was not learned before.
    New,
    /// It was learned as this already: nothing changed.
    Unchanged,
    /// It was learned the other way, which was taken back first.
    Turned,
}

/// What an account's filter knows about some words.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JunkCounts {
    /// Messages learned as spam, and as not spam.
    pub spam: u64,
    pub ham: u64,
    /// Each word asked about that has been seen: in how many spam and how many good messages.
    pub words: Vec<(String, u64, u64)>,
}

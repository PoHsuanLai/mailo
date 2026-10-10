//! How much of an account's mail is on this computer, and which parts of it are not.
//!
//! A large IMAP message is stored rebuilt from its parts, with its attachments left on the server
//! until opened (`plan.md` 9.6). An account kept offline in full fetches those too, a few each
//! pass; these are the two questions that needs the store to answer, the same way in SQLite and
//! in memory.

use mail_domain::{Attachment, Body, Message, MessageId, PartContent};

/// One attachment a sync left on the server, with what fetching it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePart {
    pub message: MessageId,
    /// The IMAP section it is (`"2"`, `"1.3"`).
    pub section: mail_domain::Section,
    /// The server's figure for it, encoded: about a third more than the file for base64.
    pub size: u64,
}

/// How much of one account is held here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Offline {
    /// Every message the account holds, headers-only ones included.
    pub messages: u64,
    /// Those whose body is here and none of whose attachments waits on the server: what can be
    /// read, saved and forwarded whole with no connection.
    pub held: u64,
    /// Attachments still on the server, across every message.
    pub parts_remote: u64,
    /// Their size by the server's figures.
    pub remote_bytes: u64,
}

impl Offline {
    /// Count one message in.
    pub(crate) fn add(&mut self, message: &Message) {
        self.messages += 1;
        let remote: Vec<&Attachment> = message
            .attachments
            .iter()
            .filter(|a| is_remote(a))
            .collect();
        if matches!(message.body, Body::Present { .. }) && remote.is_empty() {
            self.held += 1;
        }
        self.parts_remote += remote.len() as u64;
        self.remote_bytes += remote.iter().map(|a| a.size).sum::<u64>();
    }
}

/// Whether `attachment` waits on the server as a section that can be asked for.
///
/// A stored row with neither a blob nor a section reads back as a remote part with an empty
/// section (`mail_domain::content`); nothing can fetch that, so it is not counted as waiting.
pub(crate) fn is_remote(attachment: &Attachment) -> bool {
    matches!(&attachment.content, PartContent::Remote { section } if !section.is_root())
}

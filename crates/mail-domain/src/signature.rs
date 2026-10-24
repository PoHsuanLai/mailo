//! Signatures: the lines a message ends with, several to an identity.
//!
//! Kept beside the identity they sign for rather than on it, because one address is signed
//! differently for different people: a long one with a telephone number for strangers, a name
//! alone for a reply to a colleague. One of an identity's may be the default for new mail and
//! one, the same or another, the default for replies; with neither a message starts unsigned.
//!
//! Written in the composer's editor and kept twice: the editor's HTML, which is what an HTML
//! part carries, and the plain text a text part carries and the command line reads and writes.
//! The plain text is never empty when the HTML is not, so a reader of either is never told less.
//!
//! Local only. No server keeps signatures, and one that sent them would not agree with this
//! shape.

use crate::id::{IdentityId, SignatureId};
use crate::state::IsDefault;
use serde::{Deserialize, Serialize};

/// What a signature is called when its maker gives it no name.
pub const UNNAMED_SIGNATURE: &str = "Signature";

/// One signature of an identity's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub id: SignatureId,
    /// The identity it signs for. Deleted with it.
    pub identity: IdentityId,
    /// What the user calls it, in the composer's picker and in Settings. Never empty.
    pub name: String,
    /// The composer's HTML for it, when it was written there. `None` for one only ever set as
    /// text, by the command line or before signatures had a table: its text is all it says.
    pub html: Option<String>,
    /// The signature as plain text, without the `-- ` line above it.
    pub text: String,
    /// Whether a new message from the identity starts with it.
    pub new_mail: IsDefault,
    /// Whether a reply or a forward from the identity starts with it.
    pub replies: IsDefault,
}

/// Which of an identity's defaults a message starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureUse {
    /// A message that answers nothing.
    NewMail,
    /// A reply or a forward.
    Replies,
}

impl Signature {
    /// Whether it is the identity's default for `purpose`.
    pub fn default_for(&self, purpose: SignatureUse) -> IsDefault {
        match purpose {
            SignatureUse::NewMail => self.new_mail,
            SignatureUse::Replies => self.replies,
        }
    }

    /// Whether it says nothing at all, in either form: a signature of only blank lines is none.
    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }
}

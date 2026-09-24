//! Locally-assigned identifiers. Every id in this crate is local; the mapping to a remote
//! server's notion of identity is [`crate::RemoteRef`], and it is many-to-one.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! uuid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// A fresh, random identifier.
            ///
            /// Named `generate` rather than `new` so that no `Default` impl is implied: a
            /// derived default would silently mint a new id inside `..Default::default()`,
            /// which is never what the caller meant.
            pub fn generate() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wrap an existing UUID, e.g. one read back from the store.
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            /// The underlying UUID, for storage and logging.
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

uuid_id!(
    /// One configured account.
    AccountId
);
uuid_id!(
    /// One conversation, scoped to an account. Threads are not merged across accounts in v1.
    ThreadId
);
uuid_id!(
    /// One message. Distinct from [`crate::MessageKey`], which is how we recognise the *same*
    /// message arriving under a different [`crate::RemoteRef`].
    MessageId
);
uuid_id!(
    /// One draft, local-first and persisted before it is ever sent.
    DraftId
);
uuid_id!(
    /// One template: a message kept to start new ones from. Local only, never on a server.
    TemplateId
);
uuid_id!(
    /// One rule: a filter and what to do with the mail it matches. Local, and compiled to a
    /// server's Sieve script where the server takes one.
    RuleId
);
uuid_id!(
    /// One flat label. Provider folders and Gmail categories become labels too; see
    /// [`crate::LabelOrigin`].
    LabelId
);
uuid_id!(
    /// One saved view. Views are local rows and never go on the wire.
    ViewId
);
uuid_id!(
    /// Content-addressed bytes on disk: raw `.eml`, unsanitized HTML, attachment parts.
    BlobId
);
uuid_id!(
    /// One send-as identity belonging to an account.
    IdentityId
);
uuid_id!(
    /// One applied [`crate::Patch`]. Recorded so a pending local change can be re-layered on
    /// top of server truth, or undone if its remote counterpart fails permanently.
    ChangeId
);

/// A queued unit of remote work. Store-assigned and monotonic, because the outbox drains in
/// insertion order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OutboxId(i64);

impl OutboxId {
    /// Wrap a store-assigned rowid.
    pub const fn from_i64(raw: i64) -> Self {
        Self(raw)
    }

    /// The underlying rowid.
    pub const fn as_i64(&self) -> i64 {
        self.0
    }
}

impl fmt::Display for OutboxId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

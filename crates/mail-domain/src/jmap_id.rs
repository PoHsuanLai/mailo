//! The ids a JMAP server hands out (RFC 8620 section 1.2), one type per kind of thing they name.
//!
//! An id is opaque text that only the server that issued it can interpret, and it is only
//! meaningful for the kind of record it names: an email id passed where a mailbox id belongs is
//! a request for some other record, or none. They are different types so that cannot compile.
//! They are not the local ids of [`crate::id`], which this client mints; hence the prefix.
//!
//! Each is stored and sent as the text it is: serde sees a bare string.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;

macro_rules! jmap_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = std::convert::Infallible;

            fn from_str(id: &str) -> Result<Self, Self::Err> {
                Ok(Self(id.to_owned()))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.0 == other
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.0 == *other
            }
        }
    };
}

jmap_id!(
    /// A JMAP account: what every method call names as `accountId`.
    JmapAccountId
);
jmap_id!(
    /// A JMAP mailbox.
    JmapMailboxId
);
jmap_id!(
    /// A JMAP email, which is one object wherever it is filed.
    JmapEmailId
);
jmap_id!(
    /// A JMAP thread.
    JmapThreadId
);
jmap_id!(
    /// A JMAP blob: the raw bytes of an email, to download, or an upload to import.
    JmapBlobId
);
jmap_id!(
    /// A JMAP identity: an address the account may send as.
    JmapIdentityId
);

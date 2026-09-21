//! Account configuration, discovered capability, auth, and credentials.
//!
//! [`AccountPlan`] and [`AccountCaps`] are deliberately separate types. A plan is *configured*
//! — it comes from a preset or the user, it is persisted, and it changes only when the user
//! edits it. Capabilities are *discovered* from `CAPABILITY` and `LIST` on every connect.
//! Nesting the second inside the first forces presets that lie, or a rewrite of the user's
//! saved configuration on every connect.

use crate::content::Address;
use crate::id::{AccountId, IdentityId};
use crate::state::{IsDefault, MailboxRole};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

/// Everything the user configured about an account. Persisted as JSON in `accounts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountPlan {
    pub address: String,
    pub incoming: Incoming,
    pub outgoing: Outgoing,
    pub auth: AuthPlan,
    pub identities: Vec<Identity>,
}

/// How mail arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Incoming {
    Imap {
        host: String,
        port: u16,
        tls: Tls,
    },
    Pop3 {
        host: String,
        port: u16,
        tls: Tls,
        leave: LeaveOnServer,
    },
}

/// How mail leaves. SMTP is not a third incoming backend; both backends submit through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Outgoing {
    Smtp { host: String, port: u16, tls: Tls },
}

/// Transport security.
///
/// There is no opportunistic-STARTTLS variant on purpose. Opportunistic STARTTLS is
/// strippable by an active attacker and downgrades silently to cleartext credentials.
/// Either the upgrade is required, or the user knowingly chose [`Tls::Plaintext`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tls {
    /// TLS from the first byte (993 / 995 / 465).
    Implicit,
    /// Plaintext connect, then a mandatory `STARTTLS`. Failure to upgrade aborts.
    StartTlsRequired,
    /// No encryption. Only ever set deliberately.
    Plaintext,
}

/// Whether POP3 deletes what it has downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaveOnServer {
    Keep,
    DeleteAfterFetch,
}

/// How to authenticate. One plan covers both directions in v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum AuthPlan {
    /// The OAuth client id is deliberately absent. It is deployment configuration — it differs
    /// per build channel and is not a property of the user's account — so it lives in runtime
    /// config, keyed by [`OAuthIssuer`], and never in a persisted `AccountPlan`.
    OAuth {
        issuer: OAuthIssuer,
        scopes: Vec<String>,
    },
    Password {
        username: Username,
        /// Acceptable SASL mechanisms, most preferred first. A list because some servers
        /// offer only `LOGIN`.
        sasl: Vec<SaslMech>,
    },
}

/// An OAuth authorization server. Names an issuer, not a mail provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthIssuer {
    Google,
}

/// How to derive the login name from the account address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Username {
    /// `user@example.edu` logs in as `user@example.edu`.
    SameAsAddress,
    /// `user@example.edu` logs in as `user`.
    LocalPart,
    /// A login name unrelated to the address.
    Literal(String),
}

impl Username {
    /// Resolve against an account address.
    pub fn resolve(&self, address: &str) -> String {
        match self {
            Username::SameAsAddress => address.to_owned(),
            Username::LocalPart => address.split('@').next().unwrap_or(address).to_owned(),
            Username::Literal(name) => name.clone(),
        }
    }
}

/// A SASL mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SaslMech {
    Plain,
    Login,
    CramMd5,
    XOauth2,
}

/// A send-as address belonging to an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub id: IdentityId,
    pub account: AccountId,
    pub from: Address,
    pub reply_to: Option<Address>,
    pub signature: Option<String>,
    pub default: IsDefault,
}

/// What the server turned out to support. Discovered, cached, and refreshed — never part of
/// the user's saved configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountCaps {
    pub labels: ServerLabels,
    pub threads: ServerThreads,
    pub watch: WatchMode,
    pub archive: ArchiveMeans,
    pub folders: FolderRoles,
    pub condstore: Condstore,
    pub move_ext: MoveExt,
    /// When these were last observed. Stale caps are re-fetched on connect.
    pub observed_at: DateTime<Utc>,
}

/// Whether the server can hold label membership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerLabels {
    Supported,
    /// Labels exist only in our store. Every POP3 account, and IMAP servers without keywords.
    LocalOnly,
}

/// Where conversation grouping comes from. JWZ runs either way, so POP3 and IMAP share one
/// set of [`crate::ThreadId`] rules; a provider id is only a hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerThreads {
    ProviderId,
    Jwz,
}

/// How we learn that new mail arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum WatchMode {
    Idle,
    Poll { every: Duration },
}

/// What "archive" does on this server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum ArchiveMeans {
    /// Gmail: remove `INBOX` membership, the message stays in All Mail.
    DropInbox,
    /// Generic IMAP: move to a named folder.
    MoveToFolder(String),
    /// POP3: archiving exists only in our store.
    LocalOnly,
}

/// Which IMAP folder path plays which role, from `LIST (SPECIAL-USE)` plus name heuristics.
/// Paths not listed here become [`crate::LabelOrigin::Provider`] labels.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FolderRoles(pub Vec<(String, MailboxRole)>);

impl FolderRoles {
    /// The path serving `role`, if any.
    pub fn path(&self, role: MailboxRole) -> Option<&str> {
        self.0
            .iter()
            .find(|(_, r)| *r == role)
            .map(|(p, _)| p.as_str())
    }

    /// The role of `path`, if it has one.
    pub fn role(&self, path: &str) -> Option<MailboxRole> {
        self.0.iter().find(|(p, _)| p == path).map(|(_, r)| *r)
    }
}

/// Whether `CONDSTORE` is available.
///
/// Without it, noticing that a message was marked read in another client means
/// `FETCH 1:* (FLAGS)` across the whole mailbox on every poll. With it, one `CHANGEDSINCE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condstore {
    Supported,
    Absent,
}

/// Whether the `MOVE` extension is available, or a copy/store/expunge dance is needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveExt {
    Supported,
    Absent,
}

/// Which secret is being asked for. Incoming and outgoing may need different ones.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretKey {
    pub account: AccountId,
    pub purpose: SecretPurpose,
}

/// What a secret is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretPurpose {
    IncomingPassword,
    OutgoingPassword,
    OAuthRefresh,
}

/// A secret. Lives in the platform keyring and never in SQLite.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Credential {
    Password(String),
    #[serde(rename = "oauth")]
    OAuth {
        access: String,
        refresh: String,
        /// When `access` stops working. The runtime refreshes ahead of this.
        expires_at: DateTime<Utc>,
    },
}

// Written by hand, not derived: a derived Debug puts the password in every log line, panic
// message and error chain that ever touches this value.
impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Credential::Password(_) => f.write_str("Credential::Password(<redacted>)"),
            Credential::OAuth { expires_at, .. } => f
                .debug_struct("Credential::OAuth")
                .field("access", &"<redacted>")
                .field("refresh", &"<redacted>")
                .field("expires_at", expires_at)
                .finish(),
        }
    }
}

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

impl AccountPlan {
    /// The name to authenticate as.
    ///
    /// On this plan rather than on the caller because getting it wrong is an authentication
    /// failure with no explanation attached, and because two callers deriving it separately is
    /// how one of them keeps working while the other silently cannot log in. NTU wants the
    /// local part; most providers want the whole address.
    pub fn username(&self) -> String {
        match &self.auth {
            AuthPlan::Password { username, .. } => username.resolve(&self.address),
            AuthPlan::OAuth { .. } => self.address.clone(),
        }
    }

    /// Mechanisms this account will use, most preferred first.
    pub fn sasl(&self) -> Vec<SaslMech> {
        match &self.auth {
            AuthPlan::Password { sasl, .. } => sasl.clone(),
            AuthPlan::OAuth { .. } => vec![SaslMech::XOauth2],
        }
    }

    /// The name to send in `EHLO`.
    ///
    /// The sender's own domain. A client has no reliable way to learn a name that resolves
    /// back to it, and submission servers do not check: they authenticate the session instead.
    /// `localhost` is the one answer some servers actively reject, so it is not the fallback.
    pub fn ehlo(&self) -> String {
        match self.address.rsplit_once('@') {
            Some((_, domain)) if !domain.is_empty() => domain.to_owned(),
            _ => "mailo.invalid".to_owned(),
        }
    }
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
    /// The Microsoft identity platform, for managed Microsoft 365 tenants.
    ///
    /// Not personal Outlook.com. Basic authentication was retired there on 2024-09-16, and
    /// recently-created personal mailboxes are reported to have SMTP client authentication
    /// permanently off — failing even under OAuth, which is a different and worse problem than
    /// the one this variant solves.
    Microsoft,
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
    XOauth2,
}

// CRAM-MD5 is deliberately absent. The NTU spike found `SASL PLAIN` only, Gmail and Microsoft
// both want XOAUTH2, and a mechanism we cannot exercise against a real server is one we should
// not claim to support. Add it back when an account needs it, with a trace.

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
    /// Whether `\Deleted` + `EXPUNGE` may ever be issued. See [`ExpungeMeans`].
    #[serde(default)]
    pub expunge: ExpungeMeans,
    /// POP3 `TOP`: fetch headers without marking the message read.
    #[serde(default)]
    pub top: Supported,
    /// POP3 `PIPELINING`: send several commands before reading replies.
    #[serde(default)]
    pub pipelining: Supported,
    /// How many connections this server tolerates at once.
    #[serde(default)]
    pub connections: ConnectionBudget,
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

/// Whether a capability is available.
///
/// Three-valued on purpose. `Absent` means the server did not advertise it; `Withdrawn` means it
/// advertised it and then misbehaved, which is a different fact and must not be undone by the
/// next capability refresh. Servers lie in both directions: UW IMAP advertises `UIDPLUS` and
/// omits `APPENDUID`, and Thunderbird trusting that deleted the wrong draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Supported {
    Yes,
    #[default]
    Absent,
    /// Advertised, then observed not to work. Latched: never re-enabled by an advertisement.
    Withdrawn,
}

impl Supported {
    /// Whether the capability may be used.
    pub fn usable(self) -> bool {
        matches!(self, Supported::Yes)
    }

    /// Record that the capability misbehaved. Latches, so a later `CAPABILITY` cannot undo it.
    pub fn withdraw(&mut self) {
        *self = Supported::Withdrawn;
    }
}

/// Whether this account may ever be told to expunge.
///
/// `Forbidden` is the default wherever labels are server-side, and it is not a performance
/// choice. Gmail routes `EXPUNGE` through a per-account `expungeBehavior` whose values include
/// `deleteForever`, and there is no capability, no `STATUS` item and no other way to read it over
/// IMAP. On an account set that way, `\Deleted` + `EXPUNGE` destroys mail irrecoverably, and a
/// local undo patch restores only our own row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpungeMeans {
    /// Never issue `EXPUNGE`. A "move" is a copy plus a label change; nothing is deleted.
    #[default]
    Forbidden,
    /// The server deletes only what is flagged, and only in the selected mailbox.
    Allowed,
}

/// How many connections a server tolerates before it starts refusing or locking the account.
///
/// Gmail allows roughly fifteen simultaneous IMAP connections and punishes excess with a lockout
/// measured in hours, so this is a budget to stay under rather than a limit to discover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionBudget {
    pub max: u8,
}

impl Default for ConnectionBudget {
    fn default() -> Self {
        // One to watch, one to work. Raising this needs evidence that a server tolerates it.
        Self { max: 2 }
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

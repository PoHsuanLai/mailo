//! What the store says about accounts and the identities they send as.
//!
//! The rows themselves are the store's: callers ask for these values and give these values, and
//! never see the columns they are kept in.

use crate::StoreError;
use chrono::{DateTime, Utc};
use mail_domain::{
    AccountCaps, AccountPlan, ArchiveMeans, FolderRoles, Identity, MailboxRole, ServerLabels,
};
use porter_core::AccountId;

/// One account as it is stored.
///
/// The plan and the capabilities are kept as JSON and decoded when read, and a row that no
/// longer decodes is still an account: the address and the id are good. So each is a `Result`
/// that the caller reads the way it needs to, where one that cannot read a plan would still list
/// the account and another would refuse to go on.
#[derive(Debug)]
pub struct StoredAccount {
    pub id: AccountId,
    pub address: String,
    pub plan: Result<AccountPlan, StoreError>,
    /// `None` when nothing has connected yet, and the capabilities have never been written.
    pub caps: Option<Result<AccountCaps, StoreError>>,
}

/// An account to write: all of it, or none of it.
#[derive(Debug, Clone, Copy)]
pub struct NewAccount<'a> {
    pub id: &'a AccountId,
    /// The address the account is found by, which is unique. Written as given.
    pub address: &'a str,
    pub plan: &'a AccountPlan,
    /// What the server is expected to support, which a real connection later replaces.
    pub caps: &'a AccountCaps,
    /// The identities to send as. One already stored is left as it is, so a signature the user
    /// set survives an account being added again.
    pub identities: &'a [Identity],
    /// When the account was created, kept as it was if the address is already here.
    pub at: DateTime<Utc>,
}

/// Whether an account syncs. Paused, it keeps its mail and its settings and is left out of every
/// pass until it is switched back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Syncing {
    #[default]
    On,
    Paused,
}

/// What the person says about an account beyond its plan.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AccountPrefs {
    /// What the account is called in the window in place of its address. Never blank.
    pub description: Option<String>,
    pub syncing: Syncing,
    /// The server folder picked for each role, in place of the one auto-detection found.
    pub folders: Vec<(MailboxRole, String)>,
    /// Whether labels are written to the server as keywords where it takes them
    /// ([`ServerLabels::Keywords`]). Off until the person says so: a label's name is then shared
    /// with every other mail program on the account. Gmail's own labels are not asked.
    pub keep_labels: bool,
}

impl AccountPrefs {
    /// `caps` as the person has them: the folders they picked laid over the roles the server
    /// was observed to give ([`with_picks`]), and labels kept as keywords only if they said to.
    pub fn over(&self, caps: AccountCaps) -> AccountCaps {
        let mut caps = with_picks(caps, &self.folders);
        if caps.labels == ServerLabels::Keywords && !self.keep_labels {
            caps.labels = ServerLabels::LocalOnly;
        }
        caps
    }
}

/// `caps` with the folders the person picked laid over the roles the server was observed to
/// give: each picked role is that folder's alone, and the folder plays no other. An archive
/// folder picked is also where archiving moves mail, on a server that archives by moving it.
pub fn with_picks(mut caps: AccountCaps, picks: &[(MailboxRole, String)]) -> AccountCaps {
    let mut roles = caps.folders.0;
    for (role, path) in picks {
        roles.retain(|(had, played)| had != path && played != role);
        roles.push((path.clone(), *role));
        if *role == MailboxRole::Archive && matches!(caps.archive, ArchiveMeans::MoveToFolder(_)) {
            caps.archive = ArchiveMeans::MoveToFolder(path.clone());
        }
    }
    caps.folders = FolderRoles(roles);
    caps
}

//! Mailboxes (RFC 8621 §2): what the server lists, and where that files an email here.

use super::field::{malformed, opt_string, string, unsigned};
use crate::ProtoError;
use mail_domain::{AccountId, Folder, FolderRoles, Holds, MailboxRole, SpecialUse, Subscription};
use serde_json::Value;

/// The separator this client puts between a mailbox and its parent's name.
///
/// JMAP has no hierarchy delimiter — a mailbox names its parent by id — so a path is ours to
/// spell, and `/` is what every IMAP server this client meets uses.
pub const DELIMITER: char = '/';

/// A mailbox role from the IANA registry RFC 8621 §2 draws on, or one this client does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JmapRole {
    Inbox,
    Archive,
    Drafts,
    Sent,
    Trash,
    Junk,
    /// Every email, as a view.
    All,
    Flagged,
    Important,
    Other(String),
}

impl JmapRole {
    fn parse(role: &str) -> JmapRole {
        // Roles are registered in lowercase and compared exactly (RFC 8621 §2).
        match role {
            "inbox" => JmapRole::Inbox,
            "archive" => JmapRole::Archive,
            "drafts" => JmapRole::Drafts,
            "sent" => JmapRole::Sent,
            "trash" => JmapRole::Trash,
            "junk" => JmapRole::Junk,
            "all" => JmapRole::All,
            "flagged" => JmapRole::Flagged,
            "important" => JmapRole::Important,
            other => JmapRole::Other(other.to_owned()),
        }
    }

    /// Where an email in a mailbox with this role is filed here, if the role files at all.
    pub fn filed_as(&self) -> Option<MailboxRole> {
        match self {
            JmapRole::Inbox => Some(MailboxRole::Inbox),
            JmapRole::Archive => Some(MailboxRole::Archive),
            JmapRole::Drafts => Some(MailboxRole::Drafts),
            JmapRole::Sent => Some(MailboxRole::Sent),
            JmapRole::Trash => Some(MailboxRole::Trash),
            JmapRole::Junk => Some(MailboxRole::Spam),
            JmapRole::All | JmapRole::Flagged | JmapRole::Important | JmapRole::Other(_) => None,
        }
    }

    fn special(&self) -> Option<SpecialUse> {
        match self {
            JmapRole::Inbox => Some(SpecialUse::Inbox),
            JmapRole::Archive => Some(SpecialUse::Archive),
            JmapRole::Drafts => Some(SpecialUse::Drafts),
            JmapRole::Sent => Some(SpecialUse::Sent),
            JmapRole::Trash => Some(SpecialUse::Trash),
            JmapRole::Junk => Some(SpecialUse::Junk),
            JmapRole::All => Some(SpecialUse::All),
            JmapRole::Flagged => Some(SpecialUse::Flagged),
            JmapRole::Important => Some(SpecialUse::Important),
            JmapRole::Other(_) => None,
        }
    }
}

/// One mailbox, as `Mailbox/get` described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JmapMailbox {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub role: Option<JmapRole>,
    pub sort_order: u64,
    pub subscription: Subscription,
}

/// Every mailbox the account has, and the state they were listed at.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mailboxes {
    pub state: String,
    pub list: Vec<JmapMailbox>,
}

impl Mailboxes {
    /// Parse the arguments of a `Mailbox/get` answer.
    pub fn parse(args: &Value) -> Result<Mailboxes, ProtoError> {
        let list = args
            .get("list")
            .and_then(Value::as_array)
            .ok_or_else(|| malformed("Mailbox/get has no list"))?
            .iter()
            .map(|m| {
                Ok(JmapMailbox {
                    id: string(m, "id")?.to_owned(),
                    name: string(m, "name")?.to_owned(),
                    parent: opt_string(m, "parentId")?.map(str::to_owned),
                    role: opt_string(m, "role")?.map(JmapRole::parse),
                    sort_order: unsigned(m, "sortOrder", 0)?,
                    // Required by RFC 8621; a server that leaves it out is taken to mean the
                    // usual, since an unsubscribed inbox would hide the user's mail.
                    subscription: match m.get("isSubscribed").and_then(Value::as_bool) {
                        Some(false) => Subscription::Unsubscribed,
                        _ => Subscription::Subscribed,
                    },
                })
            })
            .collect::<Result<Vec<_>, ProtoError>>()?;
        Ok(Mailboxes {
            state: string(args, "state")?.to_owned(),
            list,
        })
    }

    fn by_id(&self, id: &str) -> Option<&JmapMailbox> {
        self.list.iter().find(|m| m.id == id)
    }

    /// The path this client spells a mailbox by: its ancestors' names and its own, joined by
    /// [`DELIMITER`]. A parent the server did not list, or a loop of parents, ends the walk
    /// rather than the program.
    pub fn path(&self, id: &str) -> Option<String> {
        let mut names = Vec::new();
        let mut at = self.by_id(id)?;
        names.push(at.name.as_str());
        while let Some(parent) = at.parent.as_deref().and_then(|p| self.by_id(p)) {
            if names.len() > self.list.len() {
                break;
            }
            names.push(parent.name.as_str());
            at = parent;
        }
        names.reverse();
        Some(names.join(&DELIMITER.to_string()))
    }

    /// The mailbox this client spells `path`, if the server lists one.
    pub fn id_for_path(&self, path: &str) -> Option<&str> {
        self.list
            .iter()
            .find(|m| self.path(&m.id).as_deref() == Some(path))
            .map(|m| m.id.as_str())
    }

    /// The mailbox that files as `role`, if there is one. The first by sort order when the
    /// server marks several, which RFC 8621 allows only for roles this client does not file by.
    pub fn id_for_role(&self, role: MailboxRole) -> Option<&str> {
        self.list
            .iter()
            .filter(|m| m.role.as_ref().and_then(JmapRole::filed_as) == Some(role))
            .min_by_key(|m| m.sort_order)
            .map(|m| m.id.as_str())
    }

    /// The mailboxes whose mail is not synced: Drafts, which holds this and other clients'
    /// unfinished mail, and Junk. The same two an IMAP account leaves alone.
    pub fn unfollowed(&self) -> Vec<String> {
        self.list
            .iter()
            .filter(|m| matches!(m.role, Some(JmapRole::Drafts | JmapRole::Junk)))
            .map(|m| m.id.clone())
            .collect()
    }

    /// Which path serves which role, for [`mail_domain::AccountCaps::folders`].
    pub fn roles(&self) -> FolderRoles {
        FolderRoles(
            self.list
                .iter()
                .filter_map(|m| {
                    let role = m.role.as_ref()?.filed_as()?;
                    Some((self.path(&m.id)?, role))
                })
                .collect(),
        )
    }

    /// Every mailbox as a [`Folder`], for the folder list.
    pub fn folders(&self, account: AccountId) -> Vec<Folder> {
        let mut folders: Vec<Folder> = self
            .list
            .iter()
            .filter_map(|m| {
                Some(Folder {
                    account,
                    path: self.path(&m.id)?,
                    delimiter: Some(DELIMITER),
                    special: m.role.as_ref().and_then(JmapRole::special),
                    subscription: m.subscription,
                    holds: Holds::Mail,
                })
            })
            .collect();
        folders.sort_by(|a, b| a.path.cmp(&b.path));
        folders
    }

    /// The names an email in `ids` carries as labels: every mailbox it is in that has no role.
    pub fn labels(&self, ids: &[String]) -> Vec<String> {
        let mut labels: Vec<String> = ids
            .iter()
            .filter_map(|id| self.by_id(id))
            .filter(|m| m.role.is_none())
            .filter_map(|m| self.path(&m.id))
            .collect();
        labels.sort();
        labels
    }

    /// The roles of the mailboxes in `ids` that file.
    pub(super) fn filing_roles(&self, ids: &[String]) -> Vec<MailboxRole> {
        ids.iter()
            .filter_map(|id| self.by_id(id))
            .filter_map(|m| m.role.as_ref().and_then(JmapRole::filed_as))
            .collect()
    }

    /// Whether any mailbox in `ids` is one without a role: a label.
    pub(super) fn any_label(&self, ids: &[String]) -> bool {
        ids.iter()
            .filter_map(|id| self.by_id(id))
            .any(|m| m.role.is_none())
    }
}

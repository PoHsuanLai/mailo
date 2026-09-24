//! Changing records (RFC 8620 §5.3): the patches this client sends, and what came back.
//!
//! Every change to an email is a patch of `keywords/…` and `mailboxIds/…` entries, never a whole
//! `keywords` or `mailboxIds` value. A patch names only what the user changed, so a keyword or a
//! mailbox another client added a second ago is left alone rather than overwritten by what this
//! client last saw.

use super::Mailboxes;
use super::field::{malformed, object};
use super::mailbox::DELIMITER;
use crate::{ProtoError, Refusal};
use mail_domain::{FolderWork, Keyword, MailboxRole, ReadState, Star, Subscription};
use serde_json::{Map, Value, json};
use std::time::Duration;

/// The patch that sets read and starred state.
pub fn flags_patch(read: Option<ReadState>, star: Option<Star>) -> Map<String, Value> {
    let mut patch = Map::new();
    if let Some(read) = read {
        patch.insert("keywords/$seen".to_owned(), on(read == ReadState::Read));
    }
    if let Some(star) = star {
        patch.insert("keywords/$flagged".to_owned(), on(star == Star::Starred));
    }
    patch
}

/// The patch that adds one keyword.
pub fn keyword_patch(keyword: Keyword) -> Map<String, Value> {
    let name = match keyword {
        Keyword::MdnSent => "$MDNSent",
    };
    let mut patch = Map::new();
    patch.insert(format!("keywords/{name}"), json!(true));
    patch
}

/// `true` to set a map entry, `null` to remove it: how a patch says "not any more".
fn on(yes: bool) -> Value {
    if yes { json!(true) } else { Value::Null }
}

/// The patch that files an email under `role`: into the mailbox with that role, and out of the
/// others an email is moved between — Inbox, Archive, Trash and Junk.
///
/// Mailboxes without a role are labels and stay. So does Sent: archiving a message you sent does
/// not unsend it, and taking it out of Sent would leave another client's Sent folder short.
///
/// Archiving on a server with no Archive mailbox takes the email out of the inbox and nothing
/// else, which is what archiving is where mailboxes are labels. A server refuses that for an
/// email in no other mailbox (an email must be in at least one), and the refusal is permanent:
/// the local archive is undone rather than retried.
pub fn filing_patch(
    role: MailboxRole,
    mailboxes: &Mailboxes,
) -> Result<Map<String, Value>, ProtoError> {
    let target = mailboxes.id_for_role(role);
    if target.is_none() && role != MailboxRole::Archive {
        return Err(ProtoError::Unsupported(format!(
            "the server lists no mailbox for {role:?}"
        )));
    }
    let mut patch = Map::new();
    for moving in [
        MailboxRole::Inbox,
        MailboxRole::Archive,
        MailboxRole::Trash,
        MailboxRole::Spam,
    ] {
        if let Some(id) = mailboxes
            .id_for_role(moving)
            .filter(|id| Some(*id) != target)
        {
            patch.insert(format!("mailboxIds/{id}"), Value::Null);
        }
    }
    if let Some(id) = target {
        patch.insert(format!("mailboxIds/{id}"), json!(true));
    }
    Ok(patch)
}

/// The patch that adds and removes labels, by the path each mailbox is spelled with here.
///
/// A label to add that no mailbox answers to is refused: adding an email to a mailbox that does
/// not exist is not something to do quietly. One to remove that is not there is already done.
pub fn labels_patch(
    add: &[String],
    remove: &[String],
    mailboxes: &Mailboxes,
) -> Result<Map<String, Value>, ProtoError> {
    let mut patch = Map::new();
    for name in remove {
        if let Some(id) = mailboxes.id_for_path(name) {
            patch.insert(format!("mailboxIds/{id}"), Value::Null);
        }
    }
    for name in add {
        let id = mailboxes
            .id_for_path(name)
            .ok_or_else(|| ProtoError::Refused {
                kind: Refusal::Permanent,
                text: format!("the server has no mailbox called {name:?}"),
            })?;
        patch.insert(format!("mailboxIds/{id}"), json!(true));
    }
    Ok(patch)
}

/// The `Mailbox/set` arguments that do `work`.
///
/// A delete never removes mail: `onDestroyRemoveEmails` is `false`, so a server asked to delete
/// a mailbox that still holds email refuses (`mailboxHasEmail`) and the local change is undone.
/// With `true` the server would destroy every email in no other mailbox, for good.
pub fn folder_work(
    work: &FolderWork,
    mailboxes: &Mailboxes,
) -> Result<Map<String, Value>, ProtoError> {
    let existing = |path: &str| {
        mailboxes
            .id_for_path(path)
            .map(str::to_owned)
            .ok_or_else(|| ProtoError::Refused {
                kind: Refusal::Permanent,
                text: format!("the server has no mailbox called {path:?}"),
            })
    };
    let mut args = Map::new();
    match work {
        FolderWork::Create { path } => {
            let (parent, name) = placed(path, mailboxes)?;
            args.insert(
                "create".to_owned(),
                json!({ "new": { "name": name, "parentId": parent, "isSubscribed": true } }),
            );
        }
        FolderWork::Rename { from, to } => {
            let id = existing(from)?;
            let (parent, name) = placed(to, mailboxes)?;
            args.insert(
                "update".to_owned(),
                keyed(id, json!({ "name": name, "parentId": parent })),
            );
        }
        FolderWork::Delete { path, .. } => {
            args.insert("destroy".to_owned(), json!([existing(path)?]));
            args.insert("onDestroyRemoveEmails".to_owned(), json!(false));
        }
        FolderWork::Subscribe { path, subscription } => {
            let id = existing(path)?;
            let yes = *subscription == Subscription::Subscribed;
            args.insert(
                "update".to_owned(),
                keyed(id, json!({ "isSubscribed": yes })),
            );
        }
    }
    Ok(args)
}

/// `{ key: value }`, for a key only known at run time.
fn keyed(key: String, value: Value) -> Value {
    let mut map = Map::new();
    map.insert(key, value);
    Value::Object(map)
}

/// The parent id and own name a new or renamed mailbox at `path` takes.
fn placed(path: &str, mailboxes: &Mailboxes) -> Result<(Value, String), ProtoError> {
    match path.rsplit_once(DELIMITER) {
        None => Ok((Value::Null, path.to_owned())),
        Some((parent, name)) => {
            let id = mailboxes
                .id_for_path(parent)
                .ok_or_else(|| ProtoError::Refused {
                    kind: Refusal::Permanent,
                    text: format!("the server has no mailbox called {parent:?} to put it in"),
                })?;
            Ok((json!(id), name.to_owned()))
        }
    }
}

/// Why one record in a `/set` was not changed (RFC 8620 §5.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetError {
    pub kind: String,
    pub description: Option<String>,
}

impl SetError {
    /// What a refusal of this kind means for a retry.
    pub fn proto(&self, what: &str) -> ProtoError {
        let text = match &self.description {
            Some(d) => format!("{what}: {}: {d}", self.kind),
            None => format!("{what}: {}", self.kind),
        };
        match self.kind.as_str() {
            "rateLimit" => ProtoError::Throttled {
                reason: text,
                retry_after: Some(Duration::from_secs(60 * 15)),
            },
            // "The server is temporarily unable to complete the request."
            "serverFail" | "serverUnavailable" => ProtoError::Refused {
                kind: Refusal::Transient,
                text,
            },
            _ => ProtoError::Refused {
                kind: Refusal::Permanent,
                text,
            },
        }
    }

    /// Whether the record is gone: nothing left to change, which is not a failure.
    pub fn not_found(&self) -> bool {
        self.kind == "notFound"
    }
}

/// A `/set` answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetResult {
    /// Creation id to the server's record of what it made.
    pub created: Vec<(String, Value)>,
    pub updated: Vec<String>,
    pub destroyed: Vec<String>,
    /// Every record the server did not change, by creation id or record id, and why.
    pub refused: Vec<(String, SetError)>,
}

impl SetResult {
    /// Parse the arguments of a `Foo/set` (or `Email/import`) answer.
    pub fn parse(args: &Value) -> Result<SetResult, ProtoError> {
        object(args, "a /set answer")?;
        let map = |key: &str| -> Vec<(String, Value)> {
            args.get(key)
                .and_then(Value::as_object)
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default()
        };
        let mut refused = Vec::new();
        for key in ["notCreated", "notUpdated", "notDestroyed"] {
            for (id, error) in map(key) {
                let kind = error
                    .get("type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| malformed(format!("{key} entry has no type")))?;
                refused.push((
                    id,
                    SetError {
                        kind: kind.to_owned(),
                        description: error
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    },
                ));
            }
        }
        Ok(SetResult {
            created: map("created"),
            updated: map("updated").into_iter().map(|(k, _)| k).collect(),
            destroyed: super::field::strings(args, "destroyed")?,
            refused,
        })
    }

    /// The first refusal that matters: one that is not a record already gone.
    pub fn first_refusal(&self, what: &str) -> Option<ProtoError> {
        self.refused
            .iter()
            .find(|(_, e)| !e.not_found())
            .map(|(_, e)| e.proto(what))
    }
}

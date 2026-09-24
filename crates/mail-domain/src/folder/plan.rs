//! Deciding a folder change: the refusals, the local patch, its undo, and the remote work.

use super::{
    Folder, FolderError, FolderWork, NonEmpty, Subscription, created, delimiter_of, is_inbox, same,
};
use crate::account::{AccountCaps, Incoming, ServerLabels};
use crate::content::Label;
use crate::id::{AccountId, ChangeId, LabelId, MessageId};
use crate::op::{Applied, Change, Patch, RemoteIntent};
use crate::remote::MailboxRef;
use crate::state::{LabelOrigin, Membership};

/// What this client holds in one folder, for deciding whether deleting it loses mail.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FolderContents {
    /// Messages with a server address in this mailbox.
    pub mapped: Vec<MessageId>,
    /// Messages carrying the label of the same name, where mailboxes are labels.
    pub labelled: Vec<MessageId>,
}

impl FolderContents {
    /// How many distinct messages that is.
    pub fn count(&self) -> u64 {
        let mut all: Vec<MessageId> = self.mapped.iter().chain(&self.labelled).copied().collect();
        all.sort();
        all.dedup();
        all.len() as u64
    }
}

/// Everything [`plan`] reads. Gathered by the caller, which is the one holding a store.
#[derive(Debug, Clone, Copy)]
pub struct FolderCtx<'a> {
    pub account: AccountId,
    pub incoming: &'a Incoming,
    pub caps: &'a AccountCaps,
    /// The account's folders, as listed.
    pub folders: &'a [Folder],
    /// The account's labels.
    pub labels: &'a [Label],
    /// What is held in the folder [`FolderWork::path`] names. Read only by a delete.
    pub contents: &'a FolderContents,
}

/// Decide `work`: refuse it, or say what changes here, how to undo that, and what to ask of
/// the server.
///
/// Pure, like [`crate::Op::apply`], and returning the same [`Applied`], so a caller applies
/// `forward`, queues `remote` with `inverse` as its undo, and a permanent refusal from the
/// server puts everything back.
///
/// On an account whose mailboxes are labels, the label moves with the folder: created with it,
/// renamed with it by the store, and taken off every message when the folder is deleted —
/// which on such a server is all a delete does. Never `\Deleted` and `EXPUNGE`.
pub fn plan(work: &FolderWork, ctx: &FolderCtx<'_>) -> Result<Applied, FolderError> {
    match ctx.incoming {
        Incoming::Pop3 { .. } => return Err(FolderError::SingleMailbox),
        Incoming::Local => return Err(FolderError::KeptLocally),
        Incoming::Imap { .. } => {}
    }
    let (forward, inverse) = match work {
        FolderWork::Create { path } => create(path, ctx)?,
        FolderWork::Rename { from, to } => rename(from, to, ctx)?,
        FolderWork::Delete { path, non_empty } => delete(path, *non_empty, ctx)?,
        FolderWork::Subscribe { path, subscription } => subscribe(path, *subscription, ctx)?,
    };
    Ok(Applied {
        forward: Patch {
            id: ChangeId::generate(),
            changes: forward,
        },
        inverse: Patch {
            id: ChangeId::generate(),
            changes: inverse,
        },
        remote: Some(RemoteIntent::Folder(work.clone())),
    })
}

type Changes = (Vec<Change>, Vec<Change>);

fn create(path: &str, ctx: &FolderCtx<'_>) -> Result<Changes, FolderError> {
    check_name(path, delimiter_of(ctx.folders))?;
    if ctx.folders.iter().any(|f| same(&f.path, path)) || is_inbox(path) {
        return Err(FolderError::Exists(path.to_owned()));
    }
    let folder = created(ctx.account, path, ctx.folders);
    let mut forward = vec![Change::FolderUpsert(folder)];
    let mut inverse = vec![Change::FolderRemove(mailbox(ctx, path))];
    // A label to file mail under straight away, where mailboxes are labels. Not when one of
    // that name is already held: the name is unique per account, and it is already usable.
    if labels_are_mailboxes(ctx) && label_named(ctx, path).is_none() {
        let label = Label {
            id: LabelId::generate(),
            account: ctx.account,
            name: path.to_owned(),
            color: None,
            origin: LabelOrigin::Provider,
        };
        inverse.push(Change::LabelRemove(label.id));
        forward.push(Change::LabelUpsert(label));
    }
    Ok((forward, inverse))
}

fn rename(from: &str, to: &str, ctx: &FolderCtx<'_>) -> Result<Changes, FolderError> {
    let folder = existing(from, ctx)?;
    untouchable(folder, ctx)?;
    check_name(to, folder.delimiter)?;
    if ctx.folders.iter().any(|f| same(&f.path, to)) || is_inbox(to) {
        return Err(FolderError::Exists(to.to_owned()));
    }
    if folder.contains(to) {
        return Err(FolderError::BadName {
            name: to.to_owned(),
            why: format!("a folder cannot be moved inside itself ({from})"),
        });
    }
    let from = folder.path.as_str();
    let forward = Change::FolderRename {
        from: mailbox(ctx, from),
        to: to.to_owned(),
        delimiter: folder.delimiter,
    };
    let inverse = Change::FolderRename {
        from: mailbox(ctx, to),
        to: from.to_owned(),
        delimiter: folder.delimiter,
    };
    Ok((vec![forward], vec![inverse]))
}

fn delete(path: &str, non_empty: NonEmpty, ctx: &FolderCtx<'_>) -> Result<Changes, FolderError> {
    let folder = existing(path, ctx)?;
    untouchable(folder, ctx)?;
    // Deleting a mailbox leaves the ones inside it on most servers, turned into names that hold
    // nothing, and removes them on others. Neither is what someone deleting one folder expects,
    // so the inside goes first, by hand.
    if ctx
        .folders
        .iter()
        .any(|f| f.path != folder.path && folder.contains(&f.path))
    {
        return Err(FolderError::HasChildren(path.to_owned()));
    }
    let messages = ctx.contents.count();
    if messages > 0 && non_empty == NonEmpty::Refuse {
        return Err(FolderError::NotEmpty {
            path: path.to_owned(),
            messages,
        });
    }

    let mut forward = Vec::new();
    // The undo puts the label back before anything is filed under it again.
    let mut inverse = Vec::new();
    if labels_are_mailboxes(ctx)
        && let Some(label) = label_named(ctx, path)
    {
        for message in &ctx.contents.labelled {
            forward.push(Change::MessageLabel(*message, label.id, Membership::Out));
        }
        forward.push(Change::LabelRemove(label.id));
        inverse.push(Change::LabelUpsert(label.clone()));
        for message in &ctx.contents.labelled {
            inverse.push(Change::MessageLabel(*message, label.id, Membership::In));
        }
    }
    forward.push(Change::FolderRemove(mailbox(ctx, &folder.path)));
    inverse.insert(0, Change::FolderUpsert(folder.clone()));
    Ok((forward, inverse))
}

fn subscribe(
    path: &str,
    subscription: Subscription,
    ctx: &FolderCtx<'_>,
) -> Result<Changes, FolderError> {
    let folder = existing(path, ctx)?;
    let mut changed = folder.clone();
    changed.subscription = subscription;
    // Sent even when nothing changes here: the server may disagree with the last listing, and
    // the request is idempotent.
    Ok((
        vec![Change::FolderUpsert(changed)],
        vec![Change::FolderUpsert(folder.clone())],
    ))
}

/// The folder called `path`, or why there is none.
fn existing<'a>(path: &str, ctx: &FolderCtx<'a>) -> Result<&'a Folder, FolderError> {
    ctx.folders
        .iter()
        .find(|f| same(&f.path, path))
        .ok_or_else(|| FolderError::Unknown(path.to_owned()))
}

/// Refuse to rename or delete a special-use folder, or one with a special-use folder inside it:
/// moving `[Gmail]` would move Sent and Trash out from under every client that uses them.
fn untouchable(folder: &Folder, ctx: &FolderCtx<'_>) -> Result<(), FolderError> {
    let within = ctx.folders.iter().filter(|f| folder.contains(&f.path));
    for f in std::iter::once(folder).chain(within) {
        if let Some(special) = f.protected() {
            return Err(FolderError::Special {
                path: f.path.clone(),
                special,
            });
        }
    }
    Ok(())
}

/// Whether this account's server presents labels as mailboxes.
fn labels_are_mailboxes(ctx: &FolderCtx<'_>) -> bool {
    ctx.caps.labels == ServerLabels::Supported
}

/// The server's label called `name`. A label the user made here is theirs, not the server's,
/// and is left alone.
fn label_named<'a>(ctx: &FolderCtx<'a>, name: &str) -> Option<&'a Label> {
    ctx.labels
        .iter()
        .find(|l| l.name == name && l.origin == LabelOrigin::Provider)
}

fn mailbox(ctx: &FolderCtx<'_>, path: &str) -> MailboxRef {
    MailboxRef {
        account: ctx.account,
        path: path.to_owned(),
    }
}

/// Refuse a name no server should be asked to create.
///
/// Control characters would split the command they travel in; `*` and `%` are `LIST`
/// wildcards (RFC 3501 §6.3.8), so a folder named with one can be created and then never
/// listed by name. A leading, trailing or doubled separator names a level with no name.
fn check_name(name: &str, delimiter: Option<char>) -> Result<(), FolderError> {
    let bad = |why: &str| {
        Err(FolderError::BadName {
            name: name.to_owned(),
            why: why.to_owned(),
        })
    };
    if name.trim().is_empty() {
        return bad("it is empty");
    }
    if name.chars().any(char::is_control) {
        return bad("it contains a control character");
    }
    if name.contains(['*', '%']) {
        return bad("* and % are wildcards to the server");
    }
    if let Some(d) = delimiter {
        let doubled: String = [d, d].iter().collect();
        if name.starts_with(d) || name.ends_with(d) || name.contains(&doubled) {
            return bad(&format!(
                "{d:?} separates folders, and a level cannot be empty"
            ));
        }
    }
    Ok(())
}

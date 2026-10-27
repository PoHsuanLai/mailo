//! The copies Open writes to open an attachment in its app, and removing them again as
//! `attachments.remove_unedited` says: when mailo quits, or once their message is deleted.
//!
//! Only a copy recorded here is ever removed, and only while it is as it was written. Open
//! records each file it writes into its own folder (`reading::peek`'s, under the system's
//! temporary folder), with its size and modification time; a file whose size or time has moved
//! since is the person's work, and is left. A path outside the folder it was recorded in is never
//! recorded, and is checked again before anything is removed, so nothing saved to Downloads, to a
//! folder the person chose, or anywhere else can be reached from here.

use crate::settings::RemoveUnedited;
use dioxus::prelude::*;
use mail_client::Client;
use mail_client::StoreError;
use mail_domain::{MailboxRole, MessageId};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

/// One copy Open wrote: where, out of which message, and how it was as it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) struct Written {
    /// The folder Open writes into, which `path` is inside.
    root: PathBuf,
    pub path: PathBuf,
    pub message: MessageId,
    bytes: u64,
    modified: SystemTime,
}

/// Why a file was not recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::ui) enum NotRecorded {
    /// It is not inside the folder Open writes into.
    Outside,
    /// It is not a plain file that could be looked at: gone, a link, a folder.
    NotAFile,
}

/// Every copy Open wrote and has not yet removed or let go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::ui) struct Ledger {
    entries: Vec<Written>,
}

impl Ledger {
    /// Record `path`, just written into `root` for `message`, as it is now. A path that is not a
    /// file inside `root` is refused.
    pub(in crate::ui) fn record(
        &mut self,
        root: &Path,
        path: &Path,
        message: MessageId,
    ) -> Result<(), NotRecorded> {
        if !inside(root, path) {
            return Err(NotRecorded::Outside);
        }
        let (bytes, modified) = state(path).ok_or(NotRecorded::NotAFile)?;
        self.entries.retain(|kept| kept.path != path);
        self.entries.push(Written {
            root: root.to_owned(),
            path: path.to_owned(),
            message,
            bytes,
            modified,
        });
        Ok(())
    }

    /// Remove each recorded copy `due` picks that is as it was written, and let go of every one it
    /// picks, removed or not: an edited copy is the person's from then on. Returns what was
    /// removed.
    pub(in crate::ui) fn remove_unedited(
        &mut self,
        due: impl Fn(&Written) -> bool,
    ) -> Vec<PathBuf> {
        let (picked, kept): (Vec<Written>, Vec<Written>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|w| due(w));
        self.entries = kept;
        picked
            .into_iter()
            .filter(|written| unedited(written) && std::fs::remove_file(&written.path).is_ok())
            .map(|written| written.path)
            .collect()
    }

    /// Whether nothing is recorded.
    pub(in crate::ui) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The messages the recorded copies came out of, each once.
    fn messages(&self) -> Vec<MessageId> {
        let mut messages: Vec<MessageId> = self.entries.iter().map(|w| w.message).collect();
        messages.sort();
        messages.dedup();
        messages
    }
}

/// Whether `path` names something strictly inside `root`, with no step up or aside on the way.
fn inside(root: &Path, path: &Path) -> bool {
    root.is_absolute()
        && path != root
        && path.starts_with(root)
        && path
            .components()
            .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
}

/// The size and modification time of the file at `path`; `None` for anything but a plain file
/// (a link is not followed).
fn state(path: &Path) -> Option<(u64, SystemTime)> {
    // A file that cannot be looked at is not one to record or remove.
    let meta = path.symlink_metadata().ok()?;
    if !meta.file_type().is_file() {
        return None;
    }
    // Nor is one whose time the system does not keep.
    Some((meta.len(), meta.modified().ok()?))
}

/// Whether `written` is still inside its folder and as it was written.
fn unedited(written: &Written) -> bool {
    inside(&written.root, &written.path)
        && state(&written.path) == Some((written.bytes, written.modified))
}

/// The copies this run's Open wrote. Open and the quit both reach it, and they share no window.
static OPENED: Mutex<Ledger> = Mutex::new(Ledger {
    entries: Vec::new(),
});

fn opened() -> std::sync::MutexGuard<'static, Ledger> {
    OPENED.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Record a copy Open wrote into `root` for `message`. One that cannot be recorded is only
/// logged: it stays where it is, as every copy did before it could be removed.
pub(in crate::ui) fn record(root: &Path, path: &Path, message: MessageId) {
    if let Err(why) = opened().record(root, path, message) {
        log::warn!(
            "an opened attachment was not recorded for removal ({why:?}): {}",
            path.display()
        );
    }
}

/// What the quit does: under [`RemoveUnedited::WhenMailoQuits`], every unedited copy goes.
pub(in crate::ui) fn remove_on_quit(setting: RemoveUnedited) {
    if setting == RemoveUnedited::WhenMailoQuits {
        let removed = opened().remove_unedited(|_| true);
        log::info!("removed {} unedited opened attachments", removed.len());
    }
}

/// Remove the unedited copies whose message is deleted: in Trash, or no longer stored.
pub(in crate::ui) fn remove_for_deleted(client: &Client) -> Vec<PathBuf> {
    let messages = opened().messages();
    let deleted: Vec<MessageId> = messages
        .into_iter()
        .filter(|message| is_deleted(client, *message))
        .collect();
    if deleted.is_empty() {
        return Vec::new();
    }
    opened().remove_unedited(|written| deleted.contains(&written.message))
}

/// Whether `message` is deleted: in Trash, or gone from the client. A client that could not be read
/// says nothing either way, and the copy stays.
fn is_deleted(client: &Client, message: MessageId) -> bool {
    match client.stored().message(message) {
        Ok(stored) => stored.mailbox == MailboxRole::Trash,
        Err(StoreError::NoMessage(_)) => true,
        Err(_) => false,
    }
}

/// Under [`RemoveUnedited::AfterMessageIsDeleted`], look again on every revision, since a delete
/// is one, and remove the copies whose message went.
pub(in crate::ui) fn use_remove_unedited() {
    let settings = crate::ui::prefs::use_settings();
    let _removing = use_resource(move || {
        // Read only to run again on the next revision.
        let _ = crate::ui::live::moved();
        let due =
            settings.read().attachments.remove_unedited == RemoveUnedited::AfterMessageIsDeleted;
        async move {
            if !due || opened().is_empty() {
                return;
            }
            let client = crate::ui::live::client();
            // The client is read and the files removed off the thread that draws; nothing is said.
            let _ = tokio::task::spawn_blocking(move || remove_for_deleted(&client)).await;
        }
    });
}

#[cfg(test)]
#[path = "unedited_tests.rs"]
mod tests;

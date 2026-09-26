//! What the window was started to show: the inbox, or one conversation.
//!
//! `mailo` with no arguments opens the window on the inbox. `mailo open <thread>` opens it on
//! that conversation, which is what a desktop notification's click needs to become: the
//! notification names the thread (`x-mailo-thread`), and whoever hears the click can start the
//! window this way. `mailo mailto:…` opens it on a composer holding what the link asks for,
//! which is how the desktop runs its handler for the scheme (`packaging/mailo.desktop`, `%u`):
//! [`mailto_of`] reads the link and [`start_mailto`] makes it a draft. Everything else on the
//! command line is the CLI's, and [`start_of`] says so by answering `None`.

use crate::view::Shell;
use mail_domain::{DraftId, ThreadId};
use mail_mime::MailtoUri;
use mail_store::SqliteStore;

/// Where the window opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Inbox,
    Thread(ThreadId),
    /// The composer, on a draft already saved (the one a `mailto:` link made).
    Compose(DraftId),
}

/// The window's reading of the command line: `None` when the arguments are a CLI command, else
/// where to open, or why `open` could not be read.
pub fn start_of(args: &[String]) -> Option<Result<Start, String>> {
    match args {
        [] => Some(Ok(Start::Inbox)),
        [open, rest @ ..] if open == "open" => Some(match rest {
            [thread] => thread
                .parse()
                .map(|uuid| Start::Thread(ThreadId::from_uuid(uuid)))
                .map_err(|_| format!("{thread:?} is not a thread id")),
            [] => Err("open needs a thread id: mailo open <thread>".to_owned()),
            _ => Err("open takes one thread id: mailo open <thread>".to_owned()),
        }),
        _ => None,
    }
}

/// The `mailto:` link the window was started with, when the one argument is one.
///
/// Only a lone argument: a desktop runs the handler with exactly the link (`Exec=mailo %u`), and
/// a link beside other words is a command line nobody's desktop wrote. `None` leaves the
/// arguments to [`start_of`] and the CLI.
pub fn mailto_of(args: &[String]) -> Option<MailtoUri> {
    match args {
        [uri] => MailtoUri::parse(uri),
        _ => None,
    }
}

/// Save the draft `link` asks for and say to open the composer on it.
///
/// From the first sending account, as the window's Compose does (`ui::ops::start_new`): the
/// composer shows who it is from and lets that be changed before anything is sent.
pub fn start_mailto(
    store: &SqliteStore,
    link: &MailtoUri,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Start, String> {
    let account = match crate::compose::sending_accounts(store).first() {
        Some((_, id)) => *id,
        None => crate::compose::account_for(store, None)?,
    };
    crate::compose::draft_mailto(store, account, link, now).map(|draft| Start::Compose(draft.id))
}

/// Open `thread` in the reader, from the inbox.
///
/// The inbox because that is where a notification's mail is — the watch announces nothing that
/// is anywhere else — and because the place selected is what Next and Previous step through.
/// Whatever was open is closed first, through the same method a click on a row uses, so consent
/// to remote images and an open find do not carry over.
pub fn open_thread(shell: &mut Shell, thread: ThreadId) {
    if let Some(inbox) = shell.places.iter().position(|place| {
        place.source
            == crate::view::Source::Mail(crate::view::place_filter(mail_domain::MailboxRole::Inbox))
    }) {
        shell.select(inbox);
    }
    shell.open(thread);
}

#[cfg(test)]
mod tests;

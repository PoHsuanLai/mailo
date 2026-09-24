//! What the window was started to show: the inbox, or one conversation.
//!
//! `mailo` with no arguments opens the window on the inbox. `mailo open <thread>` opens it on
//! that conversation, which is what a desktop notification's click needs to become: the
//! notification names the thread (`x-mailo-thread`), and whoever hears the click can start the
//! window this way. Everything else on the command line is the CLI's, and [`start_of`] says so by
//! answering `None`.

use crate::view::Shell;
use mail_domain::ThreadId;

/// Where the window opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    Inbox,
    Thread(ThreadId),
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

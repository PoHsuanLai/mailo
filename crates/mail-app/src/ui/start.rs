//! What the window was started to show: the inbox, one conversation, or one account's inbox.
//!
//! `mailo` with no arguments opens the window on the inbox. `mailo open <thread>` opens it on
//! that conversation, which is what a desktop notification's click needs to become: the
//! notification names the thread (`x-mailo-thread`), and whoever hears the click can start the
//! window this way. `mailo open --inbox <account>` is a summary's click: that account's inbox.
//! Either is shown in a Space that holds the account ([`space_for`]). `mailo mailto:…` opens it
//! on a composer holding what the link asks for, which is how the desktop runs its handler for
//! the scheme (`packaging/org.quire.Mail.desktop`, `%u`):
//! [`mailto_of`] reads the link and [`start_mailto`] makes it a draft. Everything else on the
//! command line is the CLI's, and [`start_of`] says so by answering `None`.

use crate::ui::view::Shell;
use mail_core::SqliteStore;
use mail_core::Store as _;
use mail_domain::{DraftId, ThreadId};
use mail_mime::MailtoUri;
use porter_core::AccountId;

/// Where the window opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Inbox,
    Thread(ThreadId),
    /// The inbox, in a Space that shows this account: a summary banner's click.
    InboxOf(AccountId),
    /// The composer, on a draft already saved (the one a `mailto:` link made).
    Compose(DraftId),
}

/// The window's reading of the command line: `None` when the arguments are a CLI command, else
/// where to open, or why `open` could not be read.
pub fn start_of(args: &[String]) -> Option<Result<Start, String>> {
    match args {
        [] => Some(Ok(Start::Inbox)),
        [open, rest @ ..] if open == "open" => Some(match rest {
            [inbox, account] if inbox == "--inbox" => AccountId::parse(account)
                .map(Start::InboxOf)
                .map_err(|_| crate::ui::copy::errors::not_an_account_id(account)),
            [inbox] if inbox == "--inbox" => {
                Err(crate::ui::copy::errors::OPEN_INBOX_NEEDS_ACCOUNT.to_owned())
            }
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
        [uri] => uri.parse().ok(),
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
    let account = match mail_core::compose::sending_accounts(store).first() {
        Some((_, id)) => id.clone(),
        None => mail_core::compose::account_for(store, None)?,
    };
    mail_core::compose::draft_mailto(store, account, link, now)
        .map(|draft| Start::Compose(draft.id))
        .map_err(String::from)
}

/// Open `thread` in the reader, from the inbox.
///
/// The inbox because that is where a notification's mail is — the watch announces nothing that
/// is anywhere else — and because the place selected is what Next and Previous step through.
/// Whatever was open is closed first, through the same method a click on a row uses, so consent
/// to remote images and an open find do not carry over.
pub fn open_thread(shell: &mut Shell, thread: ThreadId) {
    select_inbox(shell);
    shell.open(thread);
}

/// Show the Inbox, closing whatever was open as a click on the sidebar's Inbox does.
fn select_inbox(shell: &mut Shell) {
    if let Some(inbox) = shell.places.iter().position(|place| {
        place.source
            == crate::ui::view::Source::Mail(mail_core::place::place_filter(
                mail_domain::MailboxRole::Inbox,
            ))
    }) {
        shell.select(inbox);
    }
}

/// Show in `shell` what a click asked for: a conversation in the reader over the Inbox
/// ([`open_thread`]), or a summary's Inbox. The Space it is shown in is chosen first
/// ([`space_for`]); this is only what the Space shows. A plain start, and a composer's, leave
/// the shell as it is.
pub fn show(shell: &mut Shell, start: &Start) {
    match start {
        Start::Thread(thread) => open_thread(shell, *thread),
        Start::InboxOf(_) => select_inbox(shell),
        Start::Inbox | Start::Compose(_) => {}
    }
}

/// The account whose mail `start` shows: a summary's own, a conversation's as the store has it.
/// `None` for a start that names no account, and for a conversation the store does not have
/// (deleted since the banner, or never this Profile's).
pub fn account_of(store: &SqliteStore, start: &Start) -> Option<AccountId> {
    match start {
        // A conversation that cannot be read names no account; it is still opened, where the
        // window is, and the reader says what it can.
        Start::Thread(thread) => store
            .thread(*thread)
            .ok()
            .map(|thread| thread.summary.account),
        Start::InboxOf(account) => Some(account.clone()),
        Start::Inbox | Start::Compose(_) => None,
    }
}

/// The Space among `spaces` to show `start` in: the one on screen when it shows the account,
/// else the first in order that does ([`crate::ui::space::holding`]). `None` keeps the window in
/// the Space it is in, which is what happens when no Space shows the account: the conversation
/// still opens in the reader there, over that Space's Inbox, whose list does not have its row.
pub fn space_for(
    spaces: &crate::ui::space::Spaces,
    store: &SqliteStore,
    start: &Start,
) -> Option<crate::ui::space::SpaceId> {
    crate::ui::space::holding(spaces, &account_of(store, start)?)
}

#[cfg(test)]
mod tests;

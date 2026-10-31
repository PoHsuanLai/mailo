//! What the window is asked to open: one conversation, one account's inbox, a reply, an archive,
//! a `mailto:` link, or the window itself.
//!
//! Each ask is one type that says how it is written as a URI and as a command line, which
//! Space it arrives in, what it shows and what it does there ([`Ask`]). The same six answer
//! everywhere an ask comes from: the command line (`mailo open <thread>`, [`parse_args`]), a
//! second process or a notification server over the bus (a URI, [`parse_uri`]), the intent
//! provider, and the window's own launch. A click's activation token goes with the ask once, in
//! an [`Open`].
//!
//! `mailo open --reply <thread>` and `--archive <thread>` are a banner's buttons;
//! `mailo open --inbox <account>` is a summary's click. `mailo mailto:…` is how the desktop
//! runs the scheme's handler (`packaging/org.quire.Mail.desktop`, `%u`). Everything else on the
//! command line is the CLI's, and [`parse_args`] says so by answering `None`.

use crate::handoff::ActivationToken;
use crate::ui::copy::errors;
use crate::ui::motion::{Follow, tell};
use crate::ui::space::{Handle, SpaceId, Spaces};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use mail_client::Client;
use mail_domain::{DraftId, ThreadId};
use mail_mime::MailtoUri;
use porter_core::AccountId;
use std::sync::Arc;

/// Something a person or another program asks the window to open.
pub trait Ask: Send + Sync + std::fmt::Debug {
    /// The ask as the URI a running window is handed: `mailo:thread/<id>`, `mailto:…`.
    fn uri(&self) -> String;

    /// The ask as the command line that starts a window on it: `["open", "--reply", "<id>"]`.
    fn args(&self) -> Vec<String>;

    /// The account whose mail this shows, to arrive in a Space that holds it. `None` names no
    /// account, and keeps the window in the Space it is in.
    fn account(&self, _client: &Client) -> Option<AccountId> {
        None
    }

    /// Whether a running window shows it (switching Space, selecting the Inbox) before `run`.
    /// A launch always does. An ask that acts in place, wherever the window is, says no.
    fn arrives_first(&self) -> bool {
        true
    }

    /// Show it in `shell`: select the Inbox, open the conversation. The Space it is shown in is
    /// chosen first ([`Window::arrive`]); this is only what the Space shows.
    fn show(&self, _shell: &mut Shell) {}

    /// What it does once shown: reply, archive, save a draft and compose.
    fn run(&self, _window: Window) {}
}

/// The window an ask is run in: what it shows and the Spaces it can switch.
#[derive(Clone, Copy)]
pub struct Window {
    pub shell: Signal<Shell>,
    pub spaces: Handle,
}

impl std::fmt::Debug for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Window(..)")
    }
}

impl Window {
    /// Show `ask`: first in the Space [`space_for`] chooses, switched to as ⌘1-9 would (where the
    /// window was is remembered for the Space it leaves), then what it shows there. No switch
    /// when the window is already in a Space that shows the account, when none does, or while a
    /// Space's part is open (quire refuses one then); it is then shown in the Space on screen.
    pub fn arrive(self, ask: &dyn Ask) {
        let client = crate::ui::live::client();
        let (chosen, current) = {
            let list = self.spaces.spaces();
            let list = list.peek();
            (space_for(&list, &client, ask), list.current().id)
        };
        if let Some(id) = chosen.filter(|id| *id != current) {
            // `None` while a Space's part is open: the ask is then shown where the window is.
            let _ = self.spaces.switch(id);
        }
        let mut shell = self.shell;
        ask.show(&mut shell.write());
    }
}

/// An ask and the activation token that came with it: what a launcher, a notification click or a
/// second launch handed over, which a compositor wants to see before it lets a window that exists
/// take the keyboard.
#[derive(Debug, Clone)]
pub struct Open {
    pub ask: Arc<dyn Ask>,
    pub token: Option<ActivationToken>,
}

impl Open {
    /// `ask`, with the click's `token` if it had one.
    pub fn new(ask: impl Ask + 'static, token: Option<ActivationToken>) -> Open {
        Open {
            ask: Arc::new(ask),
            token,
        }
    }

    /// The plain start: raise the window and show what it shows.
    pub fn raise() -> Open {
        Open::new(Raise, None)
    }
}

impl PartialEq for Open {
    fn eq(&self, other: &Open) -> bool {
        self.token == other.token && self.ask.uri() == other.ask.uri()
    }
}

impl Eq for Open {}

/// `ask` as a shared one.
fn shared(ask: impl Ask + 'static) -> Arc<dyn Ask> {
    Arc::new(ask)
}

/// The ask `uri` makes, or `None` for one that is not of ours.
pub fn parse_uri(uri: &str) -> Option<Arc<dyn Ask>> {
    Thread::from_uri(uri)
        .map(shared)
        .or_else(|| Reply::from_uri(uri).map(shared))
        .or_else(|| Archive::from_uri(uri).map(shared))
        .or_else(|| Inbox::from_uri(uri).map(shared))
        .or_else(|| Mailto::parse(uri).map(shared))
        .or_else(|| Raise::from_uri(uri).map(shared))
}

/// The window's reading of the command line: `None` when the arguments are a CLI command, else
/// the ask, or why `open` could not be read.
///
/// A `mailto:` link is read only as a lone argument: a desktop runs the handler with exactly the
/// link (`Exec=mailo %u`), and a link beside other words is a command line nobody's desktop wrote.
pub fn parse_args(args: &[String]) -> Option<Result<Arc<dyn Ask>, String>> {
    if let [text] = args
        && let Some(link) = Mailto::parse(text)
    {
        return Some(Ok(shared(link)));
    }
    match args {
        [] => Some(Ok(shared(Raise))),
        [open, rest @ ..] if open == "open" => Some(match rest {
            [inbox, account] if inbox == "--inbox" => AccountId::parse(account)
                .map(|account| shared(Inbox(account)))
                .map_err(|_| errors::not_an_account_id(account)),
            [inbox] if inbox == "--inbox" => Err(errors::OPEN_INBOX_NEEDS_ACCOUNT.to_owned()),
            [reply, thread] if reply == "--reply" => {
                thread_arg(thread).map(|thread| shared(Reply(thread)))
            }
            [archive, thread] if archive == "--archive" => {
                thread_arg(thread).map(|thread| shared(Archive(thread)))
            }
            [thread] => thread_arg(thread).map(|thread| shared(Thread(thread))),
            [] => Err(errors::OPEN_NEEDS_THREAD.to_owned()),
            _ => Err(errors::OPEN_TAKES_ONE_THREAD.to_owned()),
        }),
        _ => None,
    }
}

/// The conversation a command-line word names.
fn thread_arg(text: &str) -> Result<ThreadId, String> {
    text.parse()
        .map(ThreadId::from_uuid)
        .map_err(|_| crate::ui::copy::errors::not_a_thread_id(text))
}

/// The conversation `uri` names after `prefix`, or `None`.
fn thread_after(uri: &str, prefix: &str) -> Option<ThreadId> {
    uri.strip_prefix(prefix)?
        .parse()
        .ok()
        .map(ThreadId::from_uuid)
}

/// The account `thread` is in, as the client has it: `None` for a conversation it does not have
/// (deleted since the banner, or never this Profile's). It is still opened, where the window is,
/// and the reader says what it can.
fn account_of_thread(client: &Client, thread: ThreadId) -> Option<AccountId> {
    client
        .stored()
        .thread(thread)
        .ok()
        .map(|thread| thread.summary.account)
}

/// A conversation, opened in the reader over the Inbox.
#[derive(Debug, Clone, Copy)]
pub struct Thread(pub ThreadId);

impl Thread {
    /// The scheme and path of a conversation's URI: `mailo:thread/<id>`.
    pub const PREFIX: &'static str = "mailo:thread/";

    /// The conversation `uri` names, or `None` for one that is not a conversation of ours.
    pub fn from_uri(uri: &str) -> Option<Thread> {
        thread_after(uri, Self::PREFIX).map(Thread)
    }
}

impl Ask for Thread {
    fn uri(&self) -> String {
        format!("{}{}", Self::PREFIX, self.0)
    }

    fn args(&self) -> Vec<String> {
        vec!["open".to_owned(), self.0.to_string()]
    }

    fn account(&self, client: &Client) -> Option<AccountId> {
        account_of_thread(client, self.0)
    }

    fn show(&self, shell: &mut Shell) {
        open_thread(shell, self.0);
    }

    fn run(&self, _window: Window) {
        // Logged: the one trace of what a click did to a window with no other way to ask it.
        log::info!(target: crate::ui::TRACE, "opened conversation {}", self.0);
    }
}

/// A reply to a conversation, opened in the composer: a banner's Reply button.
#[derive(Debug, Clone, Copy)]
pub struct Reply(pub ThreadId);

impl Reply {
    /// The scheme and path of a reply to a conversation: `mailo:reply/<id>`.
    pub const PREFIX: &'static str = "mailo:reply/";

    /// The conversation `uri` asks to reply to, or `None` for one that is not of ours.
    pub fn from_uri(uri: &str) -> Option<Reply> {
        thread_after(uri, Self::PREFIX).map(Reply)
    }
}

impl Ask for Reply {
    fn uri(&self) -> String {
        format!("{}{}", Self::PREFIX, self.0)
    }

    fn args(&self) -> Vec<String> {
        vec!["open".to_owned(), "--reply".to_owned(), self.0.to_string()]
    }

    fn account(&self, client: &Client) -> Option<AccountId> {
        account_of_thread(client, self.0)
    }

    fn show(&self, shell: &mut Shell) {
        open_thread(shell, self.0);
    }

    fn arrives_first(&self) -> bool {
        false
    }

    /// From the account the conversation is in, with its newest message answered, as the Reply
    /// key does.
    fn run(&self, window: Window) {
        log::info!(target: crate::ui::TRACE, "asked for a reply to {}", self.0);
        let client = crate::ui::live::client();
        let writes = crate::ui::ops::Writes::On(
            self.0,
            crate::ui::ops::Composes::Reply(mail_domain::ReplyScope::Sender),
        );
        // `open_composer` says why not, in the log, itself.
        let _ = crate::ui::ops::open_composer(&client, window.shell, writes);
    }
}

/// A conversation archived by the window, as its Archive key does, with the toast and the Undo:
/// a banner's Archive button.
#[derive(Debug, Clone, Copy)]
pub struct Archive(pub ThreadId);

impl Archive {
    /// The scheme and path of an archive of a conversation: `mailo:archive/<id>`.
    pub const PREFIX: &'static str = "mailo:archive/";

    /// The conversation `uri` asks to archive, or `None` for one that is not of ours.
    pub fn from_uri(uri: &str) -> Option<Archive> {
        thread_after(uri, Self::PREFIX).map(Archive)
    }
}

impl Ask for Archive {
    fn uri(&self) -> String {
        format!("{}{}", Self::PREFIX, self.0)
    }

    fn args(&self) -> Vec<String> {
        vec![
            "open".to_owned(),
            "--archive".to_owned(),
            self.0.to_string(),
        ]
    }

    fn account(&self, client: &Client) -> Option<AccountId> {
        account_of_thread(client, self.0)
    }

    fn show(&self, shell: &mut Shell) {
        select_inbox(shell);
    }

    fn arrives_first(&self) -> bool {
        false
    }

    /// As the window's own Archive does ([`crate::ui::motion::act_kind`]): the row leaves, a
    /// toast says so and offers Undo, and ⌘Z takes it back. A conversation the client does not
    /// have is said in the same toast.
    fn run(&self, window: Window) {
        log::info!(target: crate::ui::TRACE, "asked to archive {}", self.0);
        let client = crate::ui::live::client();
        if client.stored().thread(self.0).is_err() {
            crate::ui::motion::tell(
                crate::ui::copy::errors::NOT_ARCHIVED.to_owned(),
                crate::ui::motion::Follow::Nothing,
            );
            return;
        }
        crate::ui::motion::act_kind(&client, window.shell, self.0, mail_domain::OpKind::Archive);
    }
}

/// The inbox, in a Space that shows this account: a summary banner's click.
#[derive(Debug, Clone)]
pub struct Inbox(pub AccountId);

impl Inbox {
    /// The scheme and path of an account's inbox: `mailo:inbox/<account>`.
    pub const PREFIX: &'static str = "mailo:inbox/";

    /// The account whose inbox `uri` names, or `None` for one that is not an inbox of ours.
    pub fn from_uri(uri: &str) -> Option<Inbox> {
        AccountId::parse(uri.strip_prefix(Self::PREFIX)?)
            .ok()
            .map(Inbox)
    }
}

impl Ask for Inbox {
    fn uri(&self) -> String {
        format!("{}{}", Self::PREFIX, self.0.as_str())
    }

    fn args(&self) -> Vec<String> {
        vec!["open".to_owned(), "--inbox".to_owned(), self.0.to_string()]
    }

    fn account(&self, _client: &Client) -> Option<AccountId> {
        Some(self.0.clone())
    }

    fn show(&self, shell: &mut Shell) {
        select_inbox(shell);
    }

    fn run(&self, _window: Window) {
        log::info!(target: crate::ui::TRACE, "opened the inbox of {}", self.0);
    }
}

/// The composer, on what a `mailto:` link asks for.
#[derive(Debug, Clone)]
pub struct Mailto {
    text: String,
    link: MailtoUri,
}

impl Mailto {
    /// The link `text` says, or `None` when it is not a `mailto:` link.
    pub fn parse(text: &str) -> Option<Mailto> {
        text.parse().ok().map(|link| Mailto {
            text: text.to_owned(),
            link,
        })
    }

    /// A link to `email`: every byte but a letter, a digit and `@ . _ + -` escaped, so the link
    /// reads back as the one address (`mail_mime::mailto`) whatever the address holds.
    pub fn to(email: &str) -> Mailto {
        let mut text = String::from("mailto:");
        for byte in email.bytes() {
            if byte.is_ascii_alphanumeric() || b"@._+-".contains(&byte) {
                text.push(char::from(byte));
            } else {
                text.push_str(&format!("%{byte:02X}"));
            }
        }
        // The scheme is written above, so it always reads back as a link.
        let link: MailtoUri = text.parse().unwrap_or_default();
        Mailto { text, link }
    }

    /// What the link asks for.
    pub fn link(&self) -> &MailtoUri {
        &self.link
    }

    /// Save the draft the link asks for.
    ///
    /// From the first sending account, as the window's Compose does (`ui::ops::start_new`): the
    /// composer shows who it is from and lets that be changed before anything is sent.
    pub fn draft(
        &self,
        client: &Client,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<DraftId, String> {
        let account = match client.compose().sending_accounts().first() {
            Some((_, id)) => id.clone(),
            None => client.compose().account_for(None)?,
        };
        client
            .compose()
            .draft_mailto(account, &self.link, now)
            .map(|draft| draft.id)
            .map_err(String::from)
    }
}

impl Ask for Mailto {
    fn uri(&self) -> String {
        self.text.clone()
    }

    fn args(&self) -> Vec<String> {
        vec![self.text.clone()]
    }

    /// The draft is saved first, then the composer opens on it.
    fn run(&self, window: Window) {
        let client = crate::ui::live::client();
        match self.draft(&client, chrono::Utc::now()) {
            Ok(id) => {
                let mut shell = window.shell;
                if let Err(why) = shell.write().compose_id(&client, id) {
                    log::warn!("compose: {why}");
                    tell(errors::mailto_not_opened(&why.to_string()), Follow::Nothing);
                }
            }
            Err(why) => {
                log::warn!("{why}");
                tell(errors::mailto_not_opened(&why), Follow::Nothing);
            }
        }
    }
}

/// The window itself, raised and left as it is: a bare `mailo`, the Dock icon clicked again.
#[derive(Debug, Clone, Copy)]
pub struct Raise;

impl Raise {
    /// What a request for the window alone says.
    pub const URI: &'static str = "mailo:activate";

    /// `Raise` for the one URI that asks for it.
    pub fn from_uri(uri: &str) -> Option<Raise> {
        (uri == Self::URI).then_some(Raise)
    }
}

impl Ask for Raise {
    fn uri(&self) -> String {
        Self::URI.to_owned()
    }

    fn args(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Open `thread` in the reader, from the inbox.
///
/// The inbox because that is where a notification's mail is — the watch announces nothing that
/// is anywhere else — and because the place selected is what Next and Previous step through.
/// Whatever was open is closed first, through the same method a click on a row uses, so consent
/// to remote images and an open find do not carry over.
fn open_thread(shell: &mut Shell, thread: ThreadId) {
    select_inbox(shell);
    shell.open(thread);
}

/// Show the Inbox, closing whatever was open as a click on the sidebar's Inbox does.
fn select_inbox(shell: &mut Shell) {
    if let Some(inbox) = shell.places.iter().position(|place| {
        place.source
            == crate::ui::view::Source::Mail(mail_logic::mailbox_filter::for_role(
                mail_domain::MailboxRole::Inbox,
            ))
    }) {
        shell.select(inbox);
    }
}

/// The Space among `spaces` to show `ask` in: the one on screen when it shows the account, else
/// the first in order that does ([`crate::ui::space::holding`]). `None` keeps the window in the
/// Space it is in, which is what happens when no Space shows the account: the conversation still
/// opens in the reader there, over that Space's Inbox, whose list does not have its row.
pub(in crate::ui) fn space_for(spaces: &Spaces, client: &Client, ask: &dyn Ask) -> Option<SpaceId> {
    crate::ui::space::holding(spaces, &ask.account(client)?)
}

#[cfg(test)]
mod tests;

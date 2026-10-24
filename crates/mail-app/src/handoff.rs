//! Handing a conversation to the mailo window that is already running.
//!
//! A click on a new-mail banner, or `mailo open <thread>` from anywhere, should open the message
//! in the window the person has, not start a second program. The running window therefore claims
//! `io.github.PoHsuanLai.mailo` on the session bus and answers `org.freedesktop.Application`, the
//! freedesktop interface a launcher or a notification server may call on an application:
//! `Open(uris, platform_data)` with `mailo:thread/<id>` for a conversation, `mailo:inbox/<account>`
//! for an account's inbox (a summary banner's click), `Activate` for the window itself. The name is the app id mailo's Flatpak already has, because a D-Bus name needs a
//! dot and the desktop entry's own id (`mailo`) has none.
//!
//! Two ends. [`serve`] is the window's: it queues each request for the window to read on the
//! thread that draws (`ui::handoff::use_handoff`). [`deliver`] is the client's, used by `mailo open` before it
//! opens a window of its own: it answers whether a running window took the request.
//!
//! The name is mailo's own, and an `Open` carries the click's activation token in
//! `platform_data["activation-token"]`. A request keeps it ([`ActivationToken`]); the window opens
//! the conversation where the person is already reading and raises itself with the token
//! (`ds_blitz::WindowHandle::focus_with_token`), which on Wayland is how a compositor lets a window
//! that exists take the keyboard. A token is good once, so only the first request of a call has it.
//!
//! macOS has no session bus and listens on a Unix socket instead (`macos.rs`); Windows has neither: there
//! `deliver` answers `false` and nothing is served.
//!
//! Each Profile's window is reached apart: the default Profile's at [`NAME`], another's at a name
//! and path of its own ([`name_of`], [`path_of`]; on macOS a socket of its own). So
//! `mailo --profile work` hands to the work window, or opens one, and never to another Profile's.

use mail_core::profile::ProfileId;
use mail_domain::ThreadId;
use mail_mime::MailtoUri;
use porter_core::AccountId;

/// The well-known name the default Profile's running window owns.
pub const NAME: &str = "io.github.PoHsuanLai.mailo";

/// Where its `org.freedesktop.Application` object is.
pub const PATH: &str = "/io/github/PoHsuanLai/mailo";

/// The bus name `profile`'s window owns: [`NAME`] for the default Profile, the name the desktop
/// entry, the Flatpak and a launcher know; `NAME.p<id>` for another.
pub fn name_of(profile: &ProfileId) -> String {
    if profile.is_default() {
        NAME.to_owned()
    } else {
        format!("{NAME}.p{}", profile.as_str())
    }
}

/// Where `profile`'s window answers `org.freedesktop.Application`: [`PATH`], or below it for a
/// Profile other than the default.
pub fn path_of(profile: &ProfileId) -> String {
    if profile.is_default() {
        PATH.to_owned()
    } else {
        format!("{PATH}/p{}", profile.as_str())
    }
}

/// What a second `mailo` asks of its Profile's running window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    /// `mailo open <thread>`: open the conversation.
    Open(ThreadId),
    /// `mailo open --inbox <account>`: show the account's inbox.
    Inbox(AccountId),
    /// `mailo mailto:…`: compose on the link.
    Mailto(String),
    /// A bare `mailo`: raise the window.
    Activate,
}

impl Handoff {
    /// What a `mailo` started on `start` asks a running window, `mailto` being the `mailto:` link
    /// it was given: `None` for the command line, which no window is asked.
    pub fn of(start: Option<&crate::ui::Start>, mailto: Option<&str>) -> Option<Handoff> {
        use crate::ui::Start;
        match (start, mailto) {
            (_, Some(uri)) => Some(Handoff::Mailto(uri.to_owned())),
            (Some(Start::Thread(thread)), None) => Some(Handoff::Open(*thread)),
            (Some(Start::InboxOf(account)), None) => Some(Handoff::Inbox(account.clone())),
            (Some(Start::Inbox), None) => Some(Handoff::Activate),
            _ => None,
        }
    }

    /// Hand it to `profile`'s running window, with the click's activation `token`. True when
    /// one took it; false when none of that Profile is running, and then the caller opens one.
    pub fn deliver(&self, profile: &ProfileId, token: Option<&str>) -> bool {
        match self {
            Handoff::Open(thread) => deliver(profile, *thread, token),
            Handoff::Inbox(account) => deliver_inbox(profile, account, token),
            Handoff::Mailto(uri) => deliver_mailto(profile, uri, token),
            Handoff::Activate => deliver_activate(profile, token),
        }
    }
}

/// The activation token a launcher, a notification click or a second launch handed over: what a
/// compositor wants to see before it lets a window that exists take the keyboard. Never empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationToken(String);

impl ActivationToken {
    /// The token `text` says, or `None` for an empty one.
    pub fn new(text: impl Into<String>) -> Option<Self> {
        Some(text.into()).filter(|text| !text.is_empty()).map(Self)
    }

    /// The token as quire's `focus_with_token` takes it.
    pub fn into_string(self) -> String {
        self.0
    }
}

/// What a running window is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Raise the window itself; nothing else asked.
    Activate { token: Option<ActivationToken> },
    /// Open this conversation, and raise the window.
    Thread {
        thread: ThreadId,
        token: Option<ActivationToken>,
    },
    /// Show this account's inbox, and raise the window.
    Inbox {
        account: AccountId,
        token: Option<ActivationToken>,
    },
    /// Open the composer on what this `mailto:` link asks for, and raise the window.
    Mailto {
        link: MailtoUri,
        token: Option<ActivationToken>,
    },
}

/// The scheme and path of a conversation's URI: `mailo:thread/<id>`.
const THREAD_PREFIX: &str = "mailo:thread/";

/// The URI that asks a running window for `thread`.
pub fn uri_of(thread: ThreadId) -> String {
    format!("{THREAD_PREFIX}{thread}")
}

/// The conversation `uri` names, or `None` for one that is not a conversation of ours.
pub fn thread_of(uri: &str) -> Option<ThreadId> {
    let id = uri.strip_prefix(THREAD_PREFIX)?;
    id.parse().ok().map(ThreadId::from_uuid)
}

/// The scheme and path of an account's inbox: `mailo:inbox/<account>`.
const INBOX_PREFIX: &str = "mailo:inbox/";

/// The URI that asks a running window for `account`'s inbox.
pub fn inbox_uri(account: &AccountId) -> String {
    format!("{INBOX_PREFIX}{}", account.as_str())
}

/// The account whose inbox `uri` names, or `None` for one that is not an inbox of ours.
pub fn inbox_of(uri: &str) -> Option<AccountId> {
    AccountId::parse(uri.strip_prefix(INBOX_PREFIX)?).ok()
}

/// What `uris` ask, the token on the first request only: every one is for this window, and a
/// token is good once. A conversation of ours or a `mailto:` link is a request; none among them
/// is a request to raise the window.
pub fn requests_of(uris: &[String], token: Option<ActivationToken>) -> Vec<Request> {
    let mut requests: Vec<Request> = uris
        .iter()
        .filter_map(|uri| {
            if let Some(thread) = thread_of(uri) {
                Some(Request::Thread {
                    thread,
                    token: None,
                })
            } else if let Some(account) = inbox_of(uri) {
                Some(Request::Inbox {
                    account,
                    token: None,
                })
            } else {
                uri.parse()
                    .ok()
                    .map(|link| Request::Mailto { link, token: None })
            }
        })
        .collect();
    match requests.first_mut() {
        Some(
            Request::Thread { token: first, .. }
            | Request::Inbox { token: first, .. }
            | Request::Mailto { token: first, .. },
        ) => {
            *first = token;
        }
        _ => requests.push(Request::Activate { token }),
    }
    requests
}

#[cfg(not(any(target_os = "macos", windows)))]
mod bus;
#[cfg(not(any(target_os = "macos", windows)))]
pub use bus::{Requests, deliver, deliver_activate, deliver_inbox, deliver_mailto, serve};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{
    Requests, deliver, deliver_activate, deliver_inbox, deliver_mailto, hear_reopen, serve,
};

#[cfg(windows)]
mod none;
#[cfg(windows)]
pub use none::{Requests, deliver, deliver_activate, deliver_inbox, deliver_mailto, serve};

#[cfg(test)]
mod tests {
    use super::*;

    fn work() -> ProfileId {
        ProfileId::try_from("1a2b3c4d".to_owned()).expect("a profile id")
    }

    /// What a second `mailo` hands over, by what it was started with; and to whom, by its
    /// Profile: the default's window at the name it always had, another's at its own.
    #[test]
    fn the_handoff_is_chosen_by_the_start_and_addressed_by_the_profile() {
        use crate::ui::Start;
        let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x1234));
        let link = "mailto:a@example.test";
        assert_eq!(
            Handoff::of(Some(&Start::Thread(thread)), None),
            Some(Handoff::Open(thread))
        );
        assert_eq!(
            Handoff::of(Some(&Start::Inbox), Some(link)),
            Some(Handoff::Mailto(link.to_owned()))
        );
        assert_eq!(
            Handoff::of(Some(&Start::Inbox), None),
            Some(Handoff::Activate)
        );
        let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
        assert_eq!(
            Handoff::of(Some(&Start::InboxOf(account.clone())), None),
            Some(Handoff::Inbox(account))
        );
        assert_eq!(Handoff::of(None, None), None);

        let default = ProfileId::default();
        assert_eq!(name_of(&default), NAME);
        assert_eq!(path_of(&default), PATH);
        assert_eq!(name_of(&work()), format!("{NAME}.p1a2b3c4d"));
        assert_eq!(path_of(&work()), format!("{PATH}/p1a2b3c4d"));
        let other = ProfileId::try_from("0000beef".to_owned()).expect("a profile id");
        assert_ne!(name_of(&work()), name_of(&other));
    }

    #[test]
    fn a_thread_round_trips_through_its_uri_and_nothing_else_is_ours() {
        let thread = ThreadId::from_uuid(uuid::Uuid::from_u128(0x1234));
        assert_eq!(thread_of(&uri_of(thread)), Some(thread));
        const NOT_OURS: &[&str] = &[
            "",
            "mailo:thread/",
            "mailo:thread/not-a-uuid",
            "https://example.test/thread/00000000-0000-0000-0000-000000001234",
            "mailo:inbox",
        ];
        for uri in NOT_OURS {
            assert_eq!(thread_of(uri), None, "{uri:?}");
        }
    }

    /// A summary's click asks for its account's inbox, by a URI of its own; an inbox with no
    /// account, or one that is not an account id, is not one of ours.
    #[test]
    fn an_accounts_inbox_round_trips_through_its_uri_and_is_a_request() {
        let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
        assert_eq!(inbox_of(&inbox_uri(&account)), Some(account.clone()));
        for uri in ["mailo:inbox", "mailo:inbox/", "mailo:inbox/not an id!"] {
            assert_eq!(inbox_of(uri), None, "{uri:?}");
        }
        let token = ActivationToken::new("t");
        assert_eq!(
            requests_of(&[inbox_uri(&account)], token.clone()),
            vec![Request::Inbox { account, token }]
        );
    }

    #[test]
    fn a_mailto_link_is_a_request_and_takes_the_token_when_first() {
        let token = ActivationToken::new("t");
        let uris = ["mailto:a@example.test?subject=hi".to_owned()];
        let want: MailtoUri = uris[0].parse().expect("a link");
        assert_eq!(
            requests_of(&uris, token.clone()),
            vec![Request::Mailto { link: want, token }]
        );
    }

    #[test]
    fn an_empty_token_is_no_token() {
        assert_eq!(ActivationToken::new(""), None);
        let token = ActivationToken::new("abc").expect("a token");
        assert_eq!(token.into_string(), "abc");
    }

    #[test]
    fn the_token_goes_to_the_first_request_only() {
        let (one, two) = (
            ThreadId::from_uuid(uuid::Uuid::from_u128(1)),
            ThreadId::from_uuid(uuid::Uuid::from_u128(2)),
        );
        let token = ActivationToken::new("t");
        let uris = [uri_of(one), "mailo:inbox".to_owned(), uri_of(two)];
        assert_eq!(
            requests_of(&uris, token.clone()),
            vec![
                Request::Thread {
                    thread: one,
                    token: token.clone()
                },
                Request::Thread {
                    thread: two,
                    token: None
                },
            ]
        );
        // Nothing of ours asked: raise the window, with the token.
        assert_eq!(
            requests_of(&["mailo:inbox".to_owned()], token.clone()),
            vec![Request::Activate { token }]
        );
        assert_eq!(
            requests_of(&[], None),
            vec![Request::Activate { token: None }]
        );
    }
}

//! Handing a conversation to the mailo window that is already running.
//!
//! A click on a new-mail banner, or `mailo open <thread>` from anywhere, should open the message
//! in the window the person has, not start a second program. The running window therefore claims
//! `io.github.PoHsuanLai.mailo` on the session bus and answers `org.freedesktop.Application`, the
//! freedesktop interface a launcher or a notification server may call on an application:
//! `Open(uris, platform_data)` with `mailo:thread/<id>` for a conversation, `Activate` for the
//! window itself. The name is the app id mailo's Flatpak already has, because a D-Bus name needs a
//! dot and the desktop entry's own id (`mailo`) has none.
//!
//! Two ends. [`serve`] is the window's: it queues each request for the window to read on the
//! thread that draws ([`use_handoff`]). [`deliver`] is the client's, used by `mailo open` before it
//! opens a window of its own: it answers whether a running window took the request.
//!
//! The name is mailo's own, and an `Open` carries the click's activation token in
//! `platform_data["activation-token"]`. A request keeps it ([`ActivationToken`]); the window opens
//! the conversation where the person is already reading and raises itself with the token
//! (`ds_blitz::WindowHandle::focus_with_token`), which on Wayland is how a compositor lets a window
//! that exists take the keyboard. A token is good once, so only the first request of a call has it.
//!
//! macOS and Windows have no session bus: there `deliver` answers `false` and nothing is served.

use mail_domain::ThreadId;

/// The well-known name the running window owns.
pub const NAME: &str = "io.github.PoHsuanLai.mailo";

/// Where its `org.freedesktop.Application` object is.
pub const PATH: &str = "/io/github/PoHsuanLai/mailo";

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

/// What `uris` ask, the token on the first request only: every one is for this window, and a
/// token is good once. No conversation of ours among them is a request to raise the window.
pub fn requests_of(uris: &[String], token: Option<ActivationToken>) -> Vec<Request> {
    let mut threads = uris.iter().filter_map(|uri| thread_of(uri));
    let Some(first) = threads.next() else {
        return vec![Request::Activate { token }];
    };
    let first = Request::Thread {
        thread: first,
        token,
    };
    let rest = threads.map(|thread| Request::Thread {
        thread,
        token: None,
    });
    std::iter::once(first).chain(rest).collect()
}

#[cfg(not(any(target_os = "macos", windows)))]
mod bus;
#[cfg(not(any(target_os = "macos", windows)))]
pub(in crate::ui) use bus::use_handoff;
#[cfg(not(any(target_os = "macos", windows)))]
pub use bus::{Requests, deliver, serve};

#[cfg(any(target_os = "macos", windows))]
mod none;
#[cfg(any(target_os = "macos", windows))]
pub(in crate::ui) use none::use_handoff;
#[cfg(any(target_os = "macos", windows))]
pub use none::{Requests, deliver, serve};

#[cfg(test)]
mod tests {
    use super::*;

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

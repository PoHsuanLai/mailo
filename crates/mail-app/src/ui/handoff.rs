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
//! `platform_data["activation-token"]`. quire's window cannot yet raise itself with one
//! (`ds_blitz::WindowHandle::focus_with_token`, asked of quire), so the conversation opens in a
//! window of its own (`window::open_in_window`), which the compositor maps and focuses as it does
//! any new window; a conversation already open in one is raised.
//!
//! macOS and Windows have no session bus: there `deliver` answers `false` and nothing is served.

use mail_domain::ThreadId;

/// The well-known name the running window owns.
pub const NAME: &str = "io.github.PoHsuanLai.mailo";

/// Where its `org.freedesktop.Application` object is.
pub const PATH: &str = "/io/github/PoHsuanLai/mailo";

/// What a running window is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Show the window itself; nothing else asked.
    Activate,
    /// Open this conversation.
    Thread(ThreadId),
}

/// The scheme and path of a conversation's URI: `mailo:thread/<id>`.
const THREAD_PREFIX: &str = "mailo:thread/";

/// The URI that asks a running window for `thread`.
pub fn uri_of(thread: ThreadId) -> String {
    format!("{THREAD_PREFIX}{thread}")
}

/// What `uri` asks, or `None` for one that is not a conversation of ours.
pub fn request_of(uri: &str) -> Option<Request> {
    let id = uri.strip_prefix(THREAD_PREFIX)?;
    id.parse()
        .ok()
        .map(|uuid| Request::Thread(ThreadId::from_uuid(uuid)))
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
        assert_eq!(request_of(&uri_of(thread)), Some(Request::Thread(thread)));
        const NOT_OURS: &[&str] = &[
            "",
            "mailo:thread/",
            "mailo:thread/not-a-uuid",
            "https://example.test/thread/00000000-0000-0000-0000-000000001234",
            "mailo:inbox",
        ];
        for uri in NOT_OURS {
            assert_eq!(request_of(uri), None, "{uri:?}");
        }
    }
}

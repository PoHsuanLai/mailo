//! Handing a conversation to the mailo window that is already running.
//!
//! A click on a new-mail banner, or `mailo open <thread>` from anywhere, should open the message
//! in the window the person has, not start a second program. The running window therefore claims
//! `io.github.PoHsuanLai.mailo` on the session bus and answers `org.freedesktop.Application`, the
//! freedesktop interface a launcher or a notification server may call on an application:
//! `Open(uris, platform_data)` with `mailo:thread/<id>` for a conversation, `mailo:inbox/<account>`
//! for an account's inbox (a summary banner's click), `mailo:activate` (or `Activate`) for the
//! window itself. The name is the app id mailo's Flatpak already has, because a D-Bus name needs a
//! dot and the desktop entry's own id (`mailo`) has none.
//!
//! Two ends. [`serve`] is the window's: it queues each ask for the window to read on the thread
//! that draws (`ui::handoff::use_handoff`). [`deliver`] is the client's, used by `mailo open`
//! before it opens a window of its own: it answers whether a running window took the ask. What an
//! ask is, and how it is written as a URI, is [`crate::ui::ask`]'s.
//!
//! The name is mailo's own, and an `Open` carries the click's activation token in
//! `platform_data["activation-token"]`. An ask keeps it ([`ActivationToken`], in an [`Open`]);
//! the window opens the conversation where the person is already reading and raises itself with
//! the token (`ds_blitz::WindowHandle::focus_with_token`), which on Wayland is how a compositor
//! lets a window that exists take the keyboard. A token is good once, so only the first ask of a
//! call has it.
//!
//! macOS has no session bus and listens on a Unix socket instead (`macos.rs`); Windows has
//! neither: there `deliver` answers `false` and nothing is served.
//!
//! Each Profile's window is reached apart: the default Profile's at [`NAME`], another's at a name
//! and path of its own ([`name_of`], [`path_of`]; on macOS a socket of its own). So
//! `mailo --profile work` hands to the work window, or opens one, and never to another Profile's.

use crate::ui::ask::{self, Open};
use mail_client::profile::ProfileId;

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

/// What `uris` ask, the token on the first ask only: every one is for this window, and a token
/// is good once. A conversation of ours or a `mailto:` link is an ask; none among them is an ask
/// to raise the window.
pub fn opens_of(uris: &[String], mut token: Option<ActivationToken>) -> Vec<Open> {
    let mut opens: Vec<Open> = uris
        .iter()
        .filter_map(|uri| ask::parse_uri(uri))
        .map(|ask| Open {
            ask,
            token: token.take(),
        })
        .collect();
    if opens.is_empty() {
        opens.push(Open {
            ask: std::sync::Arc::new(ask::Raise),
            token,
        });
    }
    opens
}

#[cfg(not(any(target_os = "macos", windows)))]
mod bus;
#[cfg(not(any(target_os = "macos", windows)))]
pub use bus::{Requests, deliver, serve};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Requests, deliver, hear_reopen, serve};

#[cfg(windows)]
mod none;
#[cfg(windows)]
pub use none::{Requests, deliver, serve};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ask::{Ask as _, Inbox, Mailto, Raise, Thread};
    use mail_domain::ThreadId;

    fn work() -> ProfileId {
        ProfileId::try_from("1a2b3c4d".to_owned()).expect("a profile id")
    }

    /// Whom a second `mailo` hands to, by its Profile: the default's window at the name it always
    /// had, another's at its own.
    #[test]
    fn the_handoff_is_addressed_by_the_profile() {
        let default = ProfileId::default();
        assert_eq!(name_of(&default), NAME);
        assert_eq!(path_of(&default), PATH);
        assert_eq!(name_of(&work()), format!("{NAME}.p1a2b3c4d"));
        assert_eq!(path_of(&work()), format!("{PATH}/p1a2b3c4d"));
        let other = ProfileId::try_from("0000beef".to_owned()).expect("a profile id");
        assert_ne!(name_of(&work()), name_of(&other));
    }

    /// A summary's click asks for its account's inbox, by a URI of its own.
    #[test]
    fn an_accounts_inbox_is_an_ask() {
        let account = mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(0xacc));
        let token = ActivationToken::new("t");
        assert_eq!(
            opens_of(&[Inbox(account.clone()).uri()], token.clone()),
            vec![Open::new(Inbox(account), token)]
        );
    }

    #[test]
    fn a_mailto_link_is_an_ask_and_takes_the_token_when_first() {
        let token = ActivationToken::new("t");
        let uris = ["mailto:a@example.test?subject=hi".to_owned()];
        let want = Mailto::parse(&uris[0]).expect("a link");
        assert_eq!(opens_of(&uris, token.clone()), vec![Open::new(want, token)]);
    }

    #[test]
    fn an_empty_token_is_no_token() {
        assert_eq!(ActivationToken::new(""), None);
        let token = ActivationToken::new("abc").expect("a token");
        assert_eq!(token.into_string(), "abc");
    }

    #[test]
    fn the_token_goes_to_the_first_ask_only() {
        let (one, two) = (
            ThreadId::from_uuid(uuid::Uuid::from_u128(1)),
            ThreadId::from_uuid(uuid::Uuid::from_u128(2)),
        );
        let token = ActivationToken::new("t");
        let uris = [
            Thread(one).uri(),
            "mailo:inbox".to_owned(),
            Thread(two).uri(),
        ];
        assert_eq!(
            opens_of(&uris, token.clone()),
            vec![
                Open::new(Thread(one), token.clone()),
                Open::new(Thread(two), None),
            ]
        );
        // Nothing of ours asked: raise the window, with the token.
        assert_eq!(
            opens_of(&["mailo:inbox".to_owned()], token.clone()),
            vec![Open::new(Raise, token)]
        );
        assert_eq!(opens_of(&[], None), vec![Open::new(Raise, None)]);
    }

    /// The shell knows an app on the bus by the names its entry lists, so each entry mailo
    /// installs (the package's and `dist/install.sh`'s) names the ones the program owns, and the
    /// two say the same.
    #[test]
    fn each_entry_names_the_bus_names_the_program_owns() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let want = (
            format!("{};{};", crate::handoff::NAME, crate::APP_ID),
            crate::APP_ID.to_owned(),
        );
        for entry in [
            "packaging/org.quire.Mail.desktop",
            "dist/org.quire.Mail.desktop",
        ] {
            let text = std::fs::read_to_string(root.join(entry)).expect(entry);
            let key = |name: &str| {
                text.lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                    .unwrap_or_default()
                    .to_owned()
            };
            assert_eq!(
                (key("X-Quire-BusNames"), key("X-Quire-IntentsApp")),
                want,
                "{entry}"
            );
        }
    }
}

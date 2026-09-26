//! Notifications through the platform's own notification service, by `notify-rust`: the
//! freedesktop.org `org.freedesktop.Notifications` interface on the session bus on Linux and the
//! BSDs, the notification centre on macOS, toasts on Windows.
//!
//! Silent when there is nothing to talk to. A headless machine, an SSH session and a test run
//! have no session bus, and a watch that fetches mail must not stop, or even complain on every
//! pass, because there is nowhere to show a bubble.
//!
//! A click opens what the notification was about (`plan.md` 10.6b): `mailo open <thread>` for one
//! conversation, `mailo` for a summary's inbox. Each notification shown is waited on by the thread
//! that showed it, and what the service answers goes through [`answer`], a pure function of the
//! answer and what the notification opens.
//!
//! On Linux, what a click opens also rides along in the hints, for any other listener:
//! `x-mailo-thread` (a thread id, absent on a summary, which opens the inbox) and
//! `x-mailo-account`, beside `category` (`email.arrived`) and `desktop-entry` (`mailo`). The one
//! action offered is `default`, the key a server invokes when the bubble itself is clicked.
//!
//! What each platform does not do:
//! - Linux: a server's activation token (notification spec 1.2) is not passed on, because
//!   `notify-rust` does not surface the signal; a compositor that insists on one may open the
//!   window behind the one in focus.
//! - macOS: a click opens nothing. Hearing it needs the main run loop of an application bundle,
//!   and `mailo watch` is a command in a terminal; waiting would block that thread for good and
//!   show the notification twice. The notification names the installed `mailo.app` when there is
//!   one, else the system's default sender.
//! - Windows: toasts are shown as Windows PowerShell's. A toast names its sender by an
//!   application user model id, and one that no Start menu shortcut registers shows nothing at
//!   all, so the registered PowerShell id is the one that always works.
//!
//! Known gap: a click starts a window whether or not one is already open, so a click with a
//! window up gives a second window. Handing the thread to a running window needs the window to
//! listen for it, which it does not yet.

use super::{Notification, Notifier, Opens};
#[cfg(not(target_os = "macos"))]
use notify_rust::NotificationResponse;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The platform's notification service, if there is one to reach.
#[derive(Debug, Clone)]
pub struct Desktop {
    reach: Reach,
    /// How many shown notifications are being waited on for a click.
    waiting: Arc<Waiting>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Service,
    Nowhere,
}

impl Desktop {
    /// Find the notification service, or remember that there is none.
    pub fn connect() -> Self {
        Desktop {
            reach: reach(),
            waiting: Arc::new(Waiting::default()),
        }
    }
}

/// On Linux, whether a notification server answers on the session bus. Asked once, off any
/// runtime the caller is driving, like every blocking zbus call (`mail_runtime::secrets::off_runtime`).
#[cfg(all(unix, not(target_os = "macos")))]
fn reach() -> Reach {
    match mail_runtime::secrets::off_runtime(notify_rust::get_server_information) {
        Ok(_) => Reach::Service,
        Err(_) => Reach::Nowhere,
    }
}

/// On macOS, the notification centre is always there; which application it names is chosen once.
#[cfg(target_os = "macos")]
fn reach() -> Reach {
    let sender = notify_rust::get_bundle_identifier_or_default("mailo");
    // Already chosen, by an earlier `connect` in this process, is the same choice.
    let _ = notify_rust::set_application(&sender);
    Reach::Service
}

#[cfg(windows)]
fn reach() -> Reach {
    Reach::Service
}

/// Wait for what the user does with `$shown`, and start the window if they clicked it.
///
/// A macro rather than a function because the D-Bus handle and the toast handle share
/// `wait_for_response` but not a type, and `notify-rust` does not export the toast's by name.
#[cfg(not(target_os = "macos"))]
macro_rules! wait_on {
    ($shown:expr, $opens:expr, $waiting:expr) => {{
        // Past the bound the notification is still shown; only its click opens nothing.
        if let Some(_place) = $waiting.enter() {
            let mut clicked = None;
            let _ = $shown.wait_for_response(|response: &NotificationResponse| {
                clicked = answer(response, $opens);
            });
            if let Some(launch) = clicked {
                start(launch);
            }
        }
    }};
}

impl Notifier for Desktop {
    fn show(&self, notification: &Notification) {
        if self.reach == Reach::Nowhere {
            return;
        }
        let notification = notification.clone();
        let waiting = self.waiting.clone();
        // On a thread of its own, because a notification service that is slow to answer — or
        // registered and wedged — holds the call for its whole timeout, and this is called
        // between passes of a loop that fetches mail for every account on one thread. The same
        // thread then waits for the click, where the platform lets it.
        std::thread::spawn(move || {
            let Ok(shown) = built(&notification).show() else {
                return;
            };
            #[cfg(not(target_os = "macos"))]
            wait_on!(shown, notification.opens, waiting);
            // On macOS a click is not heard (the module's notes say why), so nothing waits.
            #[cfg(target_os = "macos")]
            let _ = (shown, waiting);
        });
    }
}

/// At most this many notifications are waited on for a click at once. A service should say when
/// each one closes, but one that never does must not make a week-long watch grow a thread and a
/// bus connection for every notification it ever showed; past the bound a notification is still
/// shown, and a click on it opens nothing until an earlier one closes.
pub const REMEMBERED: usize = 64;

/// A count of the notifications being waited on, bounded by [`REMEMBERED`].
#[derive(Debug, Default)]
pub struct Waiting(AtomicUsize);

/// One place among the [`Waiting`], given back when it is dropped.
#[derive(Debug)]
pub struct Place<'a>(&'a Waiting);

impl Waiting {
    /// A place to wait in, or `None` when [`REMEMBERED`] are already waiting.
    pub fn enter(&self) -> Option<Place<'_>> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |held| {
                (held < REMEMBERED).then_some(held + 1)
            })
            .ok()
            .map(|_| Place(self))
    }

    pub fn len(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// A window to start, and where it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub opens: Opens,
}

impl Launch {
    /// The arguments that start the window there: `open <thread>`, or none for the inbox.
    pub fn args(&self) -> Vec<String> {
        match self.opens {
            Opens::Thread(thread) => vec!["open".to_owned(), thread.to_string()],
            Opens::Inbox => Vec::new(),
        }
    }
}

/// What follows from the service's answer about a notification that opens `opens`: a window to
/// start when the notification itself was clicked, and nothing for a close or for any other
/// action (none other is offered, so a service inventing one gets nothing).
#[cfg(not(target_os = "macos"))]
pub fn answer(response: &NotificationResponse, opens: Opens) -> Option<Launch> {
    match response {
        NotificationResponse::Default => Some(Launch { opens }),
        NotificationResponse::Action(action) if action == "default" => Some(Launch { opens }),
        NotificationResponse::Action(_)
        | NotificationResponse::Reply(_)
        | NotificationResponse::Closed(_) => None,
    }
}

/// Start this same binary as the window, and leave it running.
///
/// The child is waited for on a thread of its own, so it is neither held up nor left a zombie. A
/// window that cannot be started is a click that did nothing, with nobody to tell but the
/// watch's terminal.
#[cfg(not(target_os = "macos"))]
fn start(launch: Launch) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("notification click: cannot find this program to start the window: {e}");
            return;
        }
    };
    let started = std::process::Command::new(exe)
        .args(launch.args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    match started {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("notification click: cannot start the window: {e}"),
    }
}

/// The notification as this platform's service is handed it.
fn built(n: &Notification) -> notify_rust::Notification {
    let mut out = notify_rust::Notification::new();
    out.appname("mailo").summary(&n.summary);
    platform(&mut out, n);
    out
}

/// Linux: the freedesktop hints, the icon, the bubble's own action, and a body escaped for a
/// server that reads it as markup.
#[cfg(all(unix, not(target_os = "macos")))]
fn platform(out: &mut notify_rust::Notification, n: &Notification) {
    use notify_rust::Hint;
    out.body(&escape(&n.body))
        .icon("mail-unread")
        .action("default", "Open")
        // The freedesktop category for this event, which servers use to pick a sound and to
        // group.
        .hint(Hint::Category("email.arrived".to_owned()))
        .hint(Hint::DesktopEntry("mailo".to_owned()))
        .hint(Hint::Custom(
            "x-mailo-account".to_owned(),
            n.account.to_string(),
        ));
    if let Opens::Thread(thread) = n.opens {
        out.hint(Hint::Custom(
            "x-mailo-thread".to_owned(),
            thread.to_string(),
        ));
    }
}

/// macOS and Windows show the body as text, and a click on a toast is its own activation, so
/// no action is added (on Windows one would be drawn as a button).
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn platform(out: &mut notify_rust::Notification, n: &Notification) {
    out.body(&n.body);
}

/// A body is markup to a server that advertises `body-markup`, and a subject is a stranger's text:
/// `<b>` in one must reach the screen as those three characters.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::ThreadId;

    fn thread(n: u128) -> Opens {
        Opens::Thread(ThreadId::from_uuid(uuid::Uuid::from_u128(n)))
    }

    #[test]
    fn a_subject_cannot_become_markup() {
        assert_eq!(escape("<b>R&D</b>"), "&lt;b&gt;R&amp;D&lt;/b&gt;");
        assert_eq!(escape("lunch on friday"), "lunch on friday");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_click_on_the_notification_opens_what_it_was_about_and_nothing_else_does() {
        use notify_rust::CloseReason;
        let cases: [(&str, NotificationResponse, Option<Launch>); 6] = [
            (
                "the bubble clicked",
                NotificationResponse::Default,
                Some(Launch { opens: thread(1) }),
            ),
            (
                "the default action named as an action",
                NotificationResponse::Action("default".to_owned()),
                Some(Launch { opens: thread(1) }),
            ),
            (
                "an action we never offered",
                NotificationResponse::Action("reply".to_owned()),
                None,
            ),
            (
                "an inline reply",
                NotificationResponse::Reply("ok".to_owned()),
                None,
            ),
            (
                "dismissed",
                NotificationResponse::Closed(CloseReason::Dismissed),
                None,
            ),
            (
                "expired",
                NotificationResponse::Closed(CloseReason::Expired),
                None,
            ),
        ];
        for (name, response, expect) in cases {
            assert_eq!(answer(&response, thread(1)), expect, "{name}");
        }
        assert_eq!(
            answer(&NotificationResponse::Default, Opens::Inbox),
            Some(Launch {
                opens: Opens::Inbox
            }),
            "a summary opens the inbox"
        );
    }

    #[test]
    fn a_click_starts_the_window_on_the_conversation_or_the_inbox() {
        let one = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
        let args = Launch {
            opens: Opens::Thread(one),
        }
        .args();
        assert_eq!(args, ["open".to_owned(), one.to_string()]);
        // What the window's own reading of the command line makes of them.
        assert_eq!(
            crate::ui::start_of(&args),
            Some(Ok(crate::ui::Start::Thread(one)))
        );
        let inbox = Launch {
            opens: Opens::Inbox,
        }
        .args();
        assert!(inbox.is_empty());
        assert_eq!(
            crate::ui::start_of(&inbox),
            Some(Ok(crate::ui::Start::Inbox))
        );
    }

    #[test]
    fn a_service_that_never_says_closed_cannot_grow_the_waiting_without_end() {
        let waiting = Waiting::default();
        let mut places: Vec<_> = (0..REMEMBERED).map_while(|_| waiting.enter()).collect();
        assert_eq!(places.len(), REMEMBERED);
        assert!(waiting.enter().is_none(), "one past the bound");
        assert_eq!(waiting.len(), REMEMBERED);
        // A notification that closes gives its place back, and the next one may wait.
        drop(places.pop());
        assert_eq!(waiting.len(), REMEMBERED - 1);
        let next = waiting.enter();
        assert!(next.is_some());
        assert_eq!(waiting.len(), REMEMBERED);
    }

    #[test]
    fn every_place_given_back_leaves_nothing_waiting() {
        let waiting = Waiting::default();
        {
            let _one = waiting.enter();
            let _two = waiting.enter();
            assert_eq!(waiting.len(), 2);
        }
        assert!(waiting.is_empty());
    }
}

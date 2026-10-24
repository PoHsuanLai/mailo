//! Notifications through the platform's own notification service, by `notify-rust`: the
//! freedesktop.org `org.freedesktop.Notifications` interface on the session bus on Linux and the
//! BSDs, the notification centre on macOS, toasts on Windows.
//!
//! Silent when there is nothing to talk to. A headless machine, an SSH session and a test run
//! have no session bus, and a watch that fetches mail must not stop, or even complain on every
//! pass, because there is nowhere to show a bubble.
//!
//! A click opens what the notification was about: `mailo open <thread>` for one
//! conversation, `mailo open --inbox <account>` for a summary's
//! account's inbox, started with the activation token the server minted
//! for the click so the compositor lets the window take focus. On Linux one listener for the whole
//! process hears the server's `ActivationToken`, `ActionInvoked` and `NotificationClosed`
//! ([`super::click`]); on Windows each notification shown is waited on by the thread that showed
//! it, and what the service answers goes through [`answer`], a pure function of the answer and
//! what the notification opens. `mailo open` hands the thread to a window already running, if
//! there is one, before it opens one of its own.
//!
//! On Linux, what a click opens also rides along in the hints, for any other listener:
//! `x-mailo-thread` (a thread id, absent on a summary, which opens the inbox) and
//! `x-mailo-account`, beside `category` (`email.arrived`) and `desktop-entry` (`org.quire.Mail`).
//! The one action offered is `default`, the key a server invokes when the bubble itself is clicked.
//!
//! A notification that leads with a one-time code offers Copy Code beside it where the press can
//! be heard, and a press puts the code on the clipboard ([`SystemClipboard`]): on Linux as a
//! second action, heard by the same listener; on Windows as a button on the toast, heard by the
//! thread that waits on it; on macOS as the banner's action button, only from the window, which
//! is the one process that hears a click there. Everywhere else the code is still the headline,
//! to be read off the banner.
//!
//! What each platform does not do:
//! - macOS: only the window hears a click ([`Desktop::connect_in_window`]): the click is delivered
//!   on the application's main run loop, which winit's is, and `mailo watch` is a command in a
//!   terminal with none, where waiting would block that thread for good. So a banner the watch
//!   shows opens nothing when clicked, and one the window shows (a follow-up coming back) opens its
//!   conversation through `mailo open`, which hands it to the running window. The notification
//!   names the installed `mailo.app` when there is one, else the system's default sender.
//! - Windows: toasts are shown as Windows PowerShell's. A toast names its sender by an
//!   application user model id, and one that no Start menu shortcut registers shows nothing at
//!   all, so the registered PowerShell id is the one that always works. No activation token: it
//!   is a Wayland and X11 notion.

use super::{COPY_CODE_ACTION, COPY_CODE_LABEL, Clipboard, Notification, Notifier, Opens};
#[cfg(any(windows, target_os = "macos"))]
use notify_rust::NotificationResponse;
#[cfg(not(all(unix, not(target_os = "macos"))))]
use std::sync::Arc;
#[cfg(not(all(unix, not(target_os = "macos"))))]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(all(unix, not(target_os = "macos")))]
use super::click::{Clicked, Clicks, Heard};

/// What hears a click: on Linux the one listener, elsewhere a bound on the threads that wait.
#[cfg(all(unix, not(target_os = "macos")))]
type Hearing = Clicks;
#[cfg(not(all(unix, not(target_os = "macos"))))]
type Hearing = Arc<Waiting>;

/// The platform's notification service, if there is one to reach.
#[derive(Debug, Clone)]
pub struct Desktop {
    reach: Reach,
    hearing: Hearing,
    /// macOS: whether this process runs the main run loop a click is delivered on (the window
    /// does; `mailo watch`, a command in a terminal, does not).
    #[cfg(target_os = "macos")]
    run_loop: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reach {
    Service,
    Nowhere,
}

impl Desktop {
    /// Find the notification service, or remember that there is none.
    pub fn connect() -> Self {
        let reach = reach();
        Desktop {
            reach,
            hearing: hearing(reach),
            #[cfg(target_os = "macos")]
            run_loop: false,
        }
    }

    /// As [`connect`](Self::connect), for the window: the process that runs the application's
    /// main run loop. On macOS that is what lets a click on a banner be heard; everywhere else
    /// it is the same as `connect`.
    pub fn connect_in_window() -> Self {
        #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
        let mut desktop = Self::connect();
        #[cfg(target_os = "macos")]
        {
            desktop.run_loop = true;
        }
        desktop
    }
}

/// Listen for clicks when there is a server to click on.
#[cfg(all(unix, not(target_os = "macos")))]
fn hearing(reach: Reach) -> Hearing {
    match reach {
        Reach::Service => Clicks::start(|heard| match heard {
            Heard::Open(Clicked { opens, token }) => start(Launch { opens }, token.as_deref()),
            Heard::Copy(code) => SystemClipboard.copy(&code),
        }),
        Reach::Nowhere => Clicks::default(),
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn hearing(_: Reach) -> Hearing {
    Arc::default()
}

/// On Linux, whether a notification server answers on the session bus. Asked once, off any
/// runtime the caller is driving, like every blocking zbus call (`mail_runtime::off_runtime`).
#[cfg(all(unix, not(target_os = "macos")))]
fn reach() -> Reach {
    match mail_runtime::off_runtime(notify_rust::get_server_information) {
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
#[cfg(any(windows, target_os = "macos"))]
macro_rules! wait_on {
    ($shown:expr, $notification:expr, $waiting:expr) => {{
        // Past the bound the notification is still shown; only its click opens nothing.
        if let Some(_place) = $waiting.enter() {
            let (mut clicked, mut copy) = (None, None);
            // A notification closed without an answer is a click nobody made.
            let _ = $shown.wait_for_response(|response: &NotificationResponse| {
                clicked = answer(response, &$notification.opens);
                copy = copy_asked(response, $notification.code.as_deref());
            });
            if let Some(launch) = clicked {
                start(launch, None);
            }
            if let Some(code) = copy {
                SystemClipboard.copy(&code);
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
        let hearing = self.hearing.clone();
        #[cfg(target_os = "macos")]
        let run_loop = self.run_loop;
        // On a thread of its own, because a notification service that is slow to answer — or
        // registered and wedged — holds the call for its whole timeout, and this is called
        // between passes of a loop that fetches mail for every account on one thread. The same
        // thread then waits for the click, where the platform lets it.
        // Copy Code is offered only where its press is heard: on macOS, by the window alone.
        #[cfg(not(target_os = "macos"))]
        let heard = true;
        #[cfg(target_os = "macos")]
        let heard = run_loop;
        std::thread::spawn(move || {
            let Ok(shown) = built(&notification, heard).show() else {
                return;
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            hearing.show(shown.id(), notification.opens.clone(), notification.code.clone());
            #[cfg(windows)]
            wait_on!(shown, notification, hearing);
            // On macOS only the window hears a click (the module's notes say why); `shown` sends
            // the notification when it is dropped, so there it simply goes.
            #[cfg(target_os = "macos")]
            if run_loop {
                wait_on!(shown, notification, hearing);
            }
        });
    }
}

/// At most this many notifications are waited on for a click at once (Windows: one thread each). A service should say when
/// each one closes, but one that never does must not make a week-long watch grow a thread and a
/// bus connection for every notification it ever showed; past the bound a notification is still
/// shown, and a click on it opens nothing until an earlier one closes.
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub const REMEMBERED: usize = 64;

/// A count of the notifications being waited on, bounded by [`REMEMBERED`].
#[cfg(not(all(unix, not(target_os = "macos"))))]
#[derive(Debug, Default)]
pub struct Waiting(AtomicUsize);

/// One place among the [`Waiting`], given back when it is dropped.
#[cfg(not(all(unix, not(target_os = "macos"))))]
#[derive(Debug)]
pub struct Place<'a>(&'a Waiting);

#[cfg(not(all(unix, not(target_os = "macos"))))]
impl Waiting {
    /// A place to wait in, or `None` when [`REMEMBERED`] are already waiting.
    pub fn enter(&self) -> Option<Place<'_>> {
        let mut held = self.0.load(Ordering::Acquire);
        loop {
            if held >= REMEMBERED {
                return None;
            }
            match self
                .0
                .compare_exchange_weak(held, held + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some(Place(self)),
                Err(now) => held = now,
            }
        }
    }

    pub fn len(&self) -> usize {
        self.0.load(Ordering::Acquire)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
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
    /// The arguments that start the window there: `open <thread>`, or `open --inbox <account>`
    /// for a summary's account's inbox.
    pub fn args(&self) -> Vec<String> {
        match &self.opens {
            Opens::Thread(thread) => vec!["open".to_owned(), thread.to_string()],
            Opens::Inbox(account) => vec![
                "open".to_owned(),
                "--inbox".to_owned(),
                account.as_str().to_owned(),
            ],
        }
    }
}

/// What follows from the service's answer about a notification that opens `opens`: a window to
/// start when the notification itself was clicked, and nothing for a close or for any other
/// action (none other is offered, so a service inventing one gets nothing).
#[cfg(any(windows, target_os = "macos"))]
pub fn answer(response: &NotificationResponse, opens: &Opens) -> Option<Launch> {
    let opens = opens.clone();
    match response {
        NotificationResponse::Default => Some(Launch { opens }),
        NotificationResponse::Action(action) if action == "default" => Some(Launch { opens }),
        NotificationResponse::Action(_)
        | NotificationResponse::Reply(_)
        | NotificationResponse::Closed(_) => None,
    }
}

/// The code to copy when the service's answer is a press of Copy Code on a notification that
/// offers `code`. macOS hands back the button's label rather than its name, so either is the
/// press.
#[cfg(any(windows, target_os = "macos"))]
pub fn copy_asked(response: &NotificationResponse, code: Option<&str>) -> Option<String> {
    match response {
        NotificationResponse::Action(action)
            if action == COPY_CODE_ACTION || action == COPY_CODE_LABEL =>
        {
            code.map(str::to_owned)
        }
        _ => None,
    }
}

/// The desktop's own clipboard, through `arboard`.
///
/// Each copy is made on a thread of its own. On Linux the clipboard is served by the program
/// that set it, for as long as it holds it, so the thread stays until another program takes the
/// clipboard (a later copy of ours among them); elsewhere the system keeps what was set, and the
/// thread ends at once. A clipboard that cannot be reached is said in the log, and the code is
/// still on the banner.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClipboard;

impl Clipboard for SystemClipboard {
    fn copy(&self, text: &str) {
        let text = text.to_owned();
        let spawned = std::thread::Builder::new()
            .name("mailo-clipboard".to_owned())
            .spawn(move || {
                if let Err(e) = put(text) {
                    log::warn!("a code could not be copied: {e}");
                }
            });
        if let Err(e) = spawned {
            log::warn!("a code could not be copied: {e}");
        }
    }
}

/// Put `text` on the clipboard and serve it until another program takes it.
#[cfg(all(unix, not(target_os = "macos")))]
fn put(text: String) -> Result<(), arboard::Error> {
    use arboard::SetExtLinux as _;
    arboard::Clipboard::new()?.set().wait().text(text)
}

/// Put `text` on the clipboard, which the system keeps.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn put(text: String) -> Result<(), arboard::Error> {
    arboard::Clipboard::new()?.set_text(text)
}

/// Start this same binary as the window, and leave it running.
///
/// `token` is the activation token the server minted for the click: handed on in the
/// environment under both names a toolkit reads (`XDG_ACTIVATION_TOKEN`, and the older
/// `DESKTOP_STARTUP_ID`), it is how the compositor knows the new window was asked for by a click
/// and may take focus. The child is waited for on a thread of its own, so it is neither held up
/// nor left a zombie. A window that cannot be started is a click that did nothing, with nobody to
/// tell but the watch's terminal.
fn start(launch: Launch, token: Option<&str>) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            log::warn!("notification click: cannot find this program to start the window: {e}");
            return;
        }
    };
    let mut command = std::process::Command::new(exe);
    command
        .args(launch.args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(token) = token {
        command
            .env("XDG_ACTIVATION_TOKEN", token)
            .env("DESKTOP_STARTUP_ID", token);
    }
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                // Reaped so it leaves no zombie; how the opened program exits is its own affair.
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("notification click: cannot start the window: {e}"),
    }
}

/// The notification as this platform's service is handed it, with Copy Code when it leads with
/// a code and a press of it would be `heard`.
fn built(n: &Notification, heard: bool) -> notify_rust::Notification {
    let mut out = notify_rust::Notification::new();
    out.appname("mailo").summary(&n.summary);
    platform(&mut out, n);
    if heard && n.code.is_some() {
        out.action(COPY_CODE_ACTION, COPY_CODE_LABEL);
    }
    out
}

/// Linux: the freedesktop hints, the icon, the bubble's own action, and a body escaped for a
/// server that reads it as markup.
#[cfg(all(unix, not(target_os = "macos")))]
fn platform(out: &mut notify_rust::Notification, n: &Notification) {
    use notify_rust::Hint;
    out.body(&escape(&n.body))
        .icon("mailo")
        .action("default", "Open")
        // The freedesktop category for this event, which servers use to pick a sound and to
        // group.
        .hint(Hint::Category("email.arrived".to_owned()))
        // The freedesktop sound theme's name for it, for a server that plays sounds by name.
        .sound_name("message-new-email")
        .hint(Hint::DesktopEntry("org.quire.Mail".to_owned()))
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
/// no action is added for it (on Windows one would be drawn as a button); Copy Code is, as one
/// ([`built`]).
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn platform(out: &mut notify_rust::Notification, n: &Notification) {
    out.body(&n.body);
    // The system's own sound for it: a banner with none set is silent on both.
    #[cfg(target_os = "macos")]
    out.sound_name("NSUserNotificationDefaultSoundName");
    #[cfg(windows)]
    out.sound_name("Mail");
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

    // Only Windows reads a response (Linux listens in `click`), so the click test, the one user
    // of this id, runs there.
    #[cfg(any(windows, target_os = "macos"))]
    fn thread(n: u128) -> Opens {
        Opens::Thread(mail_domain::ThreadId::from_uuid(uuid::Uuid::from_u128(n)))
    }

    fn coded() -> Notification {
        Notification {
            account: mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(1)),
            summary: "Ada".to_owned(),
            body: "Sign in".to_owned(),
            opens: Opens::Inbox(mail_domain::id::account_id_from_uuid(uuid::Uuid::from_u128(1))),
            code: None,
        }
        .with_code("482913".to_owned())
    }

    /// Copy Code is offered on a notification that leads with a code, where its press is heard,
    /// and nowhere else.
    #[test]
    fn copy_code_is_offered_only_for_a_code_and_only_where_it_is_heard() {
        let offers = |n: &Notification, heard| {
            built(n, heard)
                .actions
                .iter()
                .any(|a| a == COPY_CODE_ACTION)
        };
        assert!(offers(&coded(), true));
        assert!(!offers(&coded(), false), "nobody to hear the press");
        let plain = Notification {
            code: None,
            ..coded()
        };
        assert!(!offers(&plain, true), "no code to copy");
        assert_eq!(coded().summary, "Your code: 482913");
        assert_eq!(coded().body, "Ada \u{b7} Sign in");
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_press_of_copy_code_by_its_name_or_its_label_copies_the_code() {
        use notify_rust::CloseReason;
        let code = Some("482913");
        for action in [COPY_CODE_ACTION, COPY_CODE_LABEL] {
            let pressed = NotificationResponse::Action(action.to_owned());
            assert_eq!(copy_asked(&pressed, code).as_deref(), code, "{action}");
            assert_eq!(copy_asked(&pressed, None), None, "{action}, with no code");
            assert_eq!(
                answer(&pressed, &inbox()),
                None,
                "{action} opened something"
            );
        }
        for other in [
            NotificationResponse::Default,
            NotificationResponse::Action("default".to_owned()),
            NotificationResponse::Closed(CloseReason::Dismissed),
        ] {
            assert_eq!(copy_asked(&other, code), None, "{other:?}");
        }
    }

    #[cfg(any(windows, target_os = "macos"))]
    fn inbox() -> Opens {
        Opens::Inbox(mail_domain::id::account_id_from_uuid(
            uuid::Uuid::from_u128(0xacc),
        ))
    }

    #[test]
    fn a_subject_cannot_become_markup() {
        assert_eq!(escape("<b>R&D</b>"), "&lt;b&gt;R&amp;D&lt;/b&gt;");
        assert_eq!(escape("lunch on friday"), "lunch on friday");
    }

    #[cfg(any(windows, target_os = "macos"))]
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
            assert_eq!(answer(&response, &thread(1)), expect, "{name}");
        }
        assert_eq!(
            answer(&NotificationResponse::Default, &inbox()),
            Some(Launch { opens: inbox() }),
            "a summary opens the inbox"
        );
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
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

    #[cfg(not(all(unix, not(target_os = "macos"))))]
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

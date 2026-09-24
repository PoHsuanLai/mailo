//! Notifications through the desktop's notification server: the freedesktop.org
//! `org.freedesktop.Notifications` interface on the session bus.
//!
//! Silent when there is nothing to talk to. A headless machine, an SSH session and a test run
//! have no session bus, and a watch that fetches mail must not stop, or even complain on every
//! pass, because there is nowhere to show a bubble.
//!
//! A click opens what the notification was about (`plan.md` 10.6b). The server hands back an id
//! for every `Notify`; [`Shown`] remembers which of those ids are ours and what each opens, and
//! one listener per [`Desktop`] hears the server's signals and asks [`hear`] — a pure function of
//! the signal and [`Shown`] — whether to start the window: `mailo open <thread>` for one
//! conversation, `mailo` for a summary's inbox. A server that supports activation tokens
//! (notification spec 1.2) sends one just before the click, and it is passed on so the compositor
//! lets the new window take focus.
//!
//! What a click opens also rides along in the hints, for any other listener: `x-mailo-thread` (a
//! thread id, absent on a summary, which opens the inbox) and `x-mailo-account`. The one action
//! offered is `default`, the key a server invokes when the bubble itself is clicked.
//!
//! Known gap: a click starts a window whether or not one is already open, so a click with a
//! window up gives a second window. Handing the thread to a running window needs the window to
//! listen for it, which it does not yet.

use super::{Notification, Notifier, Opens};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use zbus::zvariant::Value;

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// Every signal the server sends on its interface; one match rule hears all three we read.
const SIGNALS: &str = "type='signal',interface='org.freedesktop.Notifications',\
                       path='/org/freedesktop/Notifications'";

/// The session bus's notification server, if there is a session bus.
#[derive(Debug, Clone)]
pub struct Desktop {
    bus: Option<Bus>,
}

#[derive(Debug, Clone)]
struct Bus {
    connection: zbus::blocking::Connection,
    /// Written by `show`'s threads, read by the listener.
    shown: Arc<Mutex<Shown>>,
}

impl Desktop {
    /// Reach the session bus and start listening for clicks, or remember that there is no bus.
    ///
    /// The subscription is made here, before anything is shown, so no click can come before
    /// anyone is listening for it. A bus that refuses the subscription still gets notifications;
    /// their clicks just open nothing, as before.
    pub fn connect() -> Self {
        let Ok(connection) = zbus::blocking::Connection::session() else {
            return Desktop { bus: None };
        };
        let shown = Arc::new(Mutex::new(Shown::default()));
        if let Ok(signals) =
            zbus::blocking::MessageIterator::for_match_rule(SIGNALS, &connection, None)
        {
            let shown = shown.clone();
            // Blocks on the bus for as long as the process lives; `mailo watch` is that process.
            std::thread::spawn(move || {
                let signals = signals.filter_map(|message| signal(&message.ok()?));
                listen(signals, &shown, launch);
            });
        }
        Desktop {
            bus: Some(Bus { connection, shown }),
        }
    }
}

impl Notifier for Desktop {
    fn show(&self, notification: &Notification) {
        let Some(bus) = self.bus.clone() else {
            return;
        };
        let notification = notification.clone();
        // On a thread of its own, because a notification server that is slow to answer — or
        // registered and wedged — holds a D-Bus call for its whole timeout, and this is called
        // between passes of a loop that fetches mail for every account on one thread.
        std::thread::spawn(move || {
            if let Ok(id) = send(&bus.connection, &notification)
                && let Ok(mut shown) = bus.shown.lock()
            {
                shown.remember(id, notification.opens);
            }
        });
    }
}

/// At most this many notifications are remembered. A server should say when each one closes,
/// but one that never does must not make a week-long watch grow without end; the oldest are
/// forgotten first, and a click on a bubble that old opens nothing.
pub const REMEMBERED: usize = 64;

/// Our notifications the server may still be showing, by the id it gave each, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shown(VecDeque<(u32, Held)>);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    opens: Opens,
    /// Sent just before the click, by a server that supports activation tokens.
    token: Option<String>,
}

impl Shown {
    /// Remember that the server showed notification `id`, which opens `opens`.
    pub fn remember(&mut self, id: u32, opens: Opens) {
        // A restarted server counts from the start again; the newer notification wins.
        self.forget(id);
        if self.0.len() == REMEMBERED {
            self.0.pop_front();
        }
        self.0.push_back((id, Held { opens, token: None }));
    }

    fn forget(&mut self, id: u32) -> Option<Held> {
        let at = self.0.iter().position(|(held, _)| *held == id)?;
        self.0.remove(at).map(|(_, held)| held)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut Held> {
        self.0
            .iter_mut()
            .find(|(held, _)| *held == id)
            .map(|(_, held)| held)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A signal from the notification server, as far as a click is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// `ActionInvoked`: the user chose `action` on notification `id`; `default` is the bubble.
    Invoked { id: u32, action: String },
    /// `ActivationToken`: what the window started for `id`'s click should hand the compositor.
    Token { id: u32, token: String },
    /// `NotificationClosed`, for any reason.
    Closed { id: u32 },
}

/// A window to start: where it opens, and the activation token to start it with, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub opens: Opens,
    pub token: Option<String>,
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

/// What follows from one signal: a window to start, or nothing.
///
/// Signals about ids that are not ours — every other application's notifications come past on
/// the same interface — change nothing. A click forgets its notification, so a server that
/// reports the click twice starts one window.
pub fn hear(shown: &mut Shown, signal: Signal) -> Option<Launch> {
    match signal {
        Signal::Invoked { id, action } if action == "default" => {
            shown.forget(id).map(|held| Launch {
                opens: held.opens,
                token: held.token,
            })
        }
        // No other action is offered; a server inventing one gets nothing.
        Signal::Invoked { .. } => None,
        Signal::Token { id, token } => {
            if let Some(held) = shown.get_mut(id) {
                held.token = Some(token);
            }
            None
        }
        Signal::Closed { id } => {
            shown.forget(id);
            None
        }
    }
}

/// Hear `signals` until they end, starting a window with `start` for each click on one of ours.
pub fn listen(
    signals: impl IntoIterator<Item = Signal>,
    shown: &Mutex<Shown>,
    mut start: impl FnMut(Launch),
) {
    for signal in signals {
        let launch = match shown.lock() {
            Ok(mut shown) => hear(&mut shown, signal),
            // A `show` thread panicked holding it; nothing it holds can be trusted now.
            Err(_) => return,
        };
        if let Some(launch) = launch {
            start(launch);
        }
    }
}

/// The D-Bus message as a [`Signal`], or `None` for anything else on the interface.
fn signal(message: &zbus::message::Message) -> Option<Signal> {
    let header = message.header();
    let body = message.body();
    match header.member()?.as_str() {
        "ActionInvoked" => {
            let (id, action) = body.deserialize::<(u32, String)>().ok()?;
            Some(Signal::Invoked { id, action })
        }
        "ActivationToken" => {
            let (id, token) = body.deserialize::<(u32, String)>().ok()?;
            Some(Signal::Token { id, token })
        }
        "NotificationClosed" => {
            let (id, _reason) = body.deserialize::<(u32, u32)>().ok()?;
            Some(Signal::Closed { id })
        }
        _ => None,
    }
}

/// Start this same binary as the window, and leave it running.
///
/// The token goes in both variables a toolkit looks for: `XDG_ACTIVATION_TOKEN` on Wayland,
/// `DESKTOP_STARTUP_ID` on X11. The child is waited for on a thread of its own, so it is neither
/// held up nor left a zombie. A window that cannot be started is a click that did nothing, with
/// nobody to tell but the watch's terminal.
fn launch(launch: Launch) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("notification click: cannot find this program to start the window: {e}");
            return;
        }
    };
    let mut command = std::process::Command::new(exe);
    command
        .args(launch.args())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(token) = &launch.token {
        command
            .env("XDG_ACTIVATION_TOKEN", token)
            .env("DESKTOP_STARTUP_ID", token);
    }
    match command.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => eprintln!("notification click: cannot start the window: {e}"),
    }
}

/// One `Notify` call. The id the server returns is what `ActionInvoked` will name.
fn send(bus: &zbus::blocking::Connection, n: &Notification) -> zbus::Result<u32> {
    let account = n.account.to_string();
    let mut hints: HashMap<&str, Value<'_>> = HashMap::new();
    // The freedesktop category for this event, which servers use to pick a sound and to group.
    hints.insert("category", Value::from("email.arrived"));
    hints.insert("desktop-entry", Value::from("mailo"));
    hints.insert("x-mailo-account", Value::from(account.as_str()));
    let thread = match n.opens {
        Opens::Thread(thread) => Some(thread.to_string()),
        Opens::Inbox => None,
    };
    if let Some(thread) = &thread {
        hints.insert("x-mailo-thread", Value::from(thread.as_str()));
    }
    let body = escape(&n.body);
    let reply = bus.call_method(
        Some(DESTINATION),
        PATH,
        Some(DESTINATION),
        "Notify",
        &(
            "mailo",
            0u32,
            "mail-unread",
            n.summary.as_str(),
            body.as_str(),
            vec!["default", "Open"],
            hints,
            -1i32,
        ),
    )?;
    reply.body().deserialize::<u32>()
}

/// A body is markup to a server that advertises `body-markup`, and a subject is a stranger's text:
/// `<b>` in one must reach the screen as those three characters.
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

    fn click(id: u32) -> Signal {
        Signal::Invoked {
            id,
            action: "default".to_owned(),
        }
    }

    fn token(id: u32, token: &str) -> Signal {
        Signal::Token {
            id,
            token: token.to_owned(),
        }
    }

    #[test]
    fn a_subject_cannot_become_markup() {
        assert_eq!(escape("<b>R&D</b>"), "&lt;b&gt;R&amp;D&lt;/b&gt;");
        assert_eq!(escape("lunch on friday"), "lunch on friday");
    }

    #[test]
    fn a_click_on_one_of_ours_opens_what_it_was_about_and_nothing_else_does() {
        let launch = |opens, token: Option<&str>| {
            Some(Launch {
                opens,
                token: token.map(str::to_owned),
            })
        };
        let cases: [(&str, Vec<Signal>, Option<Launch>); 9] = [
            (
                "a click on a conversation",
                vec![click(7)],
                launch(thread(1), None),
            ),
            (
                "a click on the summary",
                vec![click(8)],
                launch(Opens::Inbox, None),
            ),
            (
                "a click after its activation token",
                vec![token(7, "tok-7"), click(7)],
                launch(thread(1), Some("tok-7")),
            ),
            (
                "another notification's token is not this one's",
                vec![token(8, "tok-8"), click(7)],
                launch(thread(1), None),
            ),
            ("another application's notification", vec![click(99)], None),
            (
                "another application's token and click",
                vec![token(99, "theirs"), click(99)],
                None,
            ),
            (
                "an action we never offered",
                vec![Signal::Invoked {
                    id: 7,
                    action: "reply".to_owned(),
                }],
                None,
            ),
            (
                "a click after the bubble closed",
                vec![Signal::Closed { id: 7 }, click(7)],
                None,
            ),
            (
                "the same click reported twice",
                vec![click(7), click(7)],
                None,
            ),
        ];
        for (name, signals, expect) in cases {
            let mut shown = Shown::default();
            shown.remember(7, thread(1));
            shown.remember(8, Opens::Inbox);
            let mut last = None;
            for signal in signals {
                last = hear(&mut shown, signal);
            }
            assert_eq!(last, expect, "{name}");
        }
    }

    #[test]
    fn a_click_starts_the_window_on_the_conversation_or_the_inbox() {
        let one = ThreadId::from_uuid(uuid::Uuid::from_u128(0x7001));
        let args = Launch {
            opens: Opens::Thread(one),
            token: None,
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
            token: None,
        }
        .args();
        assert!(inbox.is_empty());
        assert_eq!(
            crate::ui::start_of(&inbox),
            Some(Ok(crate::ui::Start::Inbox))
        );
    }

    #[test]
    fn a_server_that_never_says_closed_cannot_grow_the_memory_without_end() {
        let mut shown = Shown::default();
        let all = REMEMBERED as u32 * 3;
        for id in 0..all {
            shown.remember(id, thread(u128::from(id)));
        }
        assert_eq!(shown.len(), REMEMBERED);
        assert_eq!(hear(&mut shown, click(0)), None, "the oldest are forgotten");
        let newest = all - 1;
        assert_eq!(
            hear(&mut shown, click(newest)),
            Some(Launch {
                opens: thread(u128::from(newest)),
                token: None,
            })
        );
        // Every close forgets, so a server that does say leaves nothing behind.
        for id in 0..all {
            hear(&mut shown, Signal::Closed { id });
        }
        assert!(shown.is_empty());
    }

    #[test]
    fn an_id_the_server_hands_out_again_opens_the_newer_notification() {
        let mut shown = Shown::default();
        shown.remember(1, thread(1));
        shown.remember(1, thread(2));
        assert_eq!(shown.len(), 1);
        assert_eq!(hear(&mut shown, click(1)).map(|l| l.opens), Some(thread(2)));
    }

    #[test]
    fn the_listener_starts_one_window_per_click_on_ours() {
        let shown = Mutex::new(Shown::default());
        shown.lock().unwrap().remember(3, thread(3));
        shown.lock().unwrap().remember(4, Opens::Inbox);
        let mut started = Vec::new();
        listen(
            [
                click(50),
                token(3, "t"),
                click(3),
                Signal::Closed { id: 3 },
                Signal::Closed { id: 4 },
                click(4),
            ],
            &shown,
            |launch| started.push(launch),
        );
        assert_eq!(
            started,
            [Launch {
                opens: thread(3),
                token: Some("t".to_owned()),
            }]
        );
        assert!(shown.lock().unwrap().is_empty());
    }
}

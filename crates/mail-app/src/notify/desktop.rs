//! Notifications through the desktop's notification server: the freedesktop.org
//! `org.freedesktop.Notifications` interface on the session bus.
//!
//! Silent when there is nothing to talk to. A headless machine, an SSH session and a test run
//! have no session bus, and a watch that fetches mail must not stop, or even complain on every
//! pass, because there is nowhere to show a bubble.
//!
//! What a click opens rides along in the hints, for whoever listens for `ActionInvoked` on the
//! id the server hands back: `x-mailo-thread` (a thread id, absent on a summary, which opens the
//! inbox) and `x-mailo-account`. The one action offered is `default`, the key a server invokes
//! when the bubble itself is clicked.

use super::{Notification, Notifier, Opens};
use std::collections::HashMap;
use zbus::zvariant::Value;

const DESTINATION: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// The session bus's notification server, if there is a session bus.
#[derive(Debug, Clone)]
pub struct Desktop {
    bus: Option<zbus::blocking::Connection>,
}

impl Desktop {
    /// Reach the session bus, or remember that there is none.
    pub fn connect() -> Self {
        Desktop {
            bus: zbus::blocking::Connection::session().ok(),
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
            let _ = send(&bus, &notification);
        });
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
    use super::escape;

    #[test]
    fn a_subject_cannot_become_markup() {
        assert_eq!(escape("<b>R&D</b>"), "&lt;b&gt;R&amp;D&lt;/b&gt;");
        assert_eq!(escape("lunch on friday"), "lunch on friday");
    }
}

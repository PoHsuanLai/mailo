//! Hearing what the notification server says about our notifications: which one was clicked, and
//! the activation token the server minted for the click.
//!
//! One listener for the whole process, on a session-bus connection of its own, instead of a
//! thread parked on each notification shown. The server (sill's, or any that speaks version 1.2 of
//! the specification) emits `ActivationToken(id, token)` and then `ActionInvoked(id, action)`
//! when the bubble is clicked, and `NotificationClosed(id, reason)` when it goes. `notify-rust`
//! does not surface the first, and a window opened without the token may open behind the one in
//! focus, so the token is the reason this is here.
//!
//! What each signal means is [`Held::hear`], a pure function over what was shown; the bus is the
//! seam around it ([`Clicks::start`]). Bounded by [`REMEMBERED`]: a server that never says closed
//! must not make a week-long watch grow without end, so the oldest notification is forgotten
//! first and a click on it opens nothing.

use super::Opens;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

/// At most this many notifications are remembered for a click at once.
pub const REMEMBERED: usize = 64;

/// What the server said, after the bus has decoded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// `ActivationToken(id, token)`, sent before the action it belongs to.
    Token { id: u32, token: String },
    /// `ActionInvoked(id, action)`.
    Invoked { id: u32, action: String },
    /// `NotificationClosed(id, reason)`.
    Closed { id: u32 },
}

/// A click on a notification of ours: what to open, and the token to open it with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clicked {
    pub opens: Opens,
    pub token: Option<String>,
}

/// The notifications shown and not yet closed, oldest first.
#[derive(Debug, Default)]
pub struct Held {
    order: VecDeque<u32>,
    opens: HashMap<u32, Opens>,
    tokens: HashMap<u32, String>,
}

impl Held {
    /// Remember that notification `id` opens `opens`, forgetting the oldest past [`REMEMBERED`].
    pub fn show(&mut self, id: u32, opens: Opens) {
        if self.opens.insert(id, opens).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > REMEMBERED {
            if let Some(old) = self.order.pop_front() {
                self.forget(old);
            }
        }
    }

    fn forget(&mut self, id: u32) {
        self.opens.remove(&id);
        self.tokens.remove(&id);
        self.order.retain(|held| *held != id);
    }

    /// How many are remembered.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// What follows from `signal`: a click when the bubble itself was clicked on a notification
    /// of ours. The action offered is `default`, the key a server invokes for a click on the
    /// bubble; any other action is nothing we offered, and a close is only forgotten. Ids that
    /// are not ours (another program's notifications pass this bus too) are ignored.
    pub fn hear(&mut self, signal: Signal) -> Option<Clicked> {
        match signal {
            Signal::Token { id, token } => {
                if self.opens.contains_key(&id) {
                    self.tokens.insert(id, token);
                }
                None
            }
            Signal::Invoked { id, action } => {
                let opens = *self.opens.get(&id)?;
                (action == "default").then(|| Clicked {
                    opens,
                    token: self.tokens.get(&id).cloned().filter(|t| !t.is_empty()),
                })
            }
            Signal::Closed { id } => {
                self.forget(id);
                None
            }
        }
    }
}

/// The listener's handle: what [`Desktop`](super::desktop::Desktop) tells it about each
/// notification it shows.
#[derive(Debug, Clone, Default)]
pub struct Clicks {
    held: Arc<Mutex<Held>>,
}

impl Clicks {
    /// Remember what notification `id` opens. Called right after the server answered `Notify`.
    pub fn show(&self, id: u32, opens: Opens) {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .show(id, opens);
    }

    /// Feed one signal through the held notifications.
    pub fn hear(&self, signal: Signal) -> Option<Clicked> {
        self.held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .hear(signal)
    }

    /// Listen on the session bus, on a thread of its own, calling `on_click` for each click.
    /// A desktop with no bus listens to nothing and says so once, as every notification does.
    pub fn start(on_click: impl Fn(Clicked) + Send + 'static) -> Clicks {
        let clicks = Clicks::default();
        let heard = clicks.clone();
        let spawned = std::thread::Builder::new()
            .name("mailo-notify-clicks".to_owned())
            .spawn(move || {
                if let Err(e) = bus::listen(|signal| {
                    if let Some(click) = heard.hear(signal) {
                        on_click(click);
                    }
                }) {
                    eprintln!("notification clicks are not heard: {e}");
                }
            });
        if let Err(e) = spawned {
            eprintln!("notification clicks are not heard: {e}");
        }
        clicks
    }
}

/// The session bus half: match the three signals, decode them, hand them over.
mod bus {
    use super::Signal;
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::message::Type;
    use zbus::{MatchRule, Message};

    const INTERFACE: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";

    /// Block, handing each of our three signals to `each`, until the bus goes away.
    pub(super) fn listen(mut each: impl FnMut(Signal)) -> zbus::Result<()> {
        let connection = Connection::session()?;
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(INTERFACE)?
            .path(PATH)?
            .build();
        for message in MessageIterator::for_match_rule(rule, &connection, Some(64))? {
            if let Some(signal) = message.ok().and_then(|m| decode(&m)) {
                each(signal);
            }
        }
        Ok(())
    }

    fn decode(message: &Message) -> Option<Signal> {
        let header = message.header();
        let member = header.member()?;
        let body = message.body();
        match member.as_str() {
            "ActivationToken" => {
                let (id, token): (u32, String) = body.deserialize().ok()?;
                Some(Signal::Token { id, token })
            }
            "ActionInvoked" => {
                let (id, action): (u32, String) = body.deserialize().ok()?;
                Some(Signal::Invoked { id, action })
            }
            "NotificationClosed" => {
                let (id, _reason): (u32, u32) = body.deserialize().ok()?;
                Some(Signal::Closed { id })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::ThreadId;

    fn thread(n: u128) -> Opens {
        Opens::Thread(ThreadId::from_uuid(uuid::Uuid::from_u128(n)))
    }

    fn token(id: u32, token: &str) -> Signal {
        Signal::Token {
            id,
            token: token.to_owned(),
        }
    }

    fn invoked(id: u32, action: &str) -> Signal {
        Signal::Invoked {
            id,
            action: action.to_owned(),
        }
    }

    #[test]
    fn a_click_opens_what_the_notification_was_about_with_the_token_the_server_sent_first() {
        let mut held = Held::default();
        held.show(7, thread(1));
        assert_eq!(
            held.hear(token(7, "tok-7")),
            None,
            "a token alone opens nothing"
        );
        assert_eq!(
            held.hear(invoked(7, "default")),
            Some(Clicked {
                opens: thread(1),
                token: Some("tok-7".to_owned())
            })
        );
    }

    #[test]
    fn a_server_that_sends_no_token_still_opens_the_message() {
        let mut held = Held::default();
        held.show(1, Opens::Inbox);
        assert_eq!(
            held.hear(invoked(1, "default")),
            Some(Clicked {
                opens: Opens::Inbox,
                token: None
            })
        );
        held.show(2, Opens::Inbox);
        held.hear(token(2, ""));
        assert_eq!(
            held.hear(invoked(2, "default")).map(|c| c.token),
            Some(None)
        );
    }

    #[test]
    fn nothing_but_the_default_action_on_our_own_notification_opens_anything() {
        let mut held = Held::default();
        held.show(3, thread(3));
        let cases = [
            ("an action we never offered", invoked(3, "reply")),
            ("another program's notification", invoked(99, "default")),
            ("a close", Signal::Closed { id: 3 }),
            ("a click after the close", invoked(3, "default")),
        ];
        for (name, signal) in cases {
            assert_eq!(held.hear(signal), None, "{name}");
        }
        assert!(held.is_empty(), "the close forgot it");
    }

    #[test]
    fn another_programs_token_is_not_kept() {
        let mut held = Held::default();
        held.hear(token(50, "theirs"));
        assert!(held.tokens.is_empty());
    }

    #[test]
    fn a_server_that_never_says_closed_cannot_grow_the_memory_without_end() {
        let mut held = Held::default();
        for id in 0..(REMEMBERED as u32 + 10) {
            held.show(id, thread(u128::from(id)));
            held.hear(token(id, "t"));
        }
        assert_eq!(held.len(), REMEMBERED);
        assert_eq!(
            held.tokens.len(),
            REMEMBERED,
            "the tokens are forgotten with them"
        );
        assert_eq!(held.hear(invoked(0, "default")), None, "the oldest is gone");
        assert!(
            held.hear(invoked(REMEMBERED as u32 + 9, "default"))
                .is_some()
        );
    }
}

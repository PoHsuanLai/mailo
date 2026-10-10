//! The session-bus half of [`super`]: serving `org.freedesktop.Application` from the window, and
//! calling it from `mailo open`.

use super::{ActivationToken, NAME, PATH, Request, requests_of, uri_of};
use mail_domain::ThreadId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use zbus::blocking::{Connection, connection::Builder};
use zbus::zvariant::{OwnedValue, Value};

/// What the window reads requests from, as a root context. The bus connection is held with it, so
/// the name is owned for as long as the window's contexts are.
#[derive(Clone)]
pub struct Requests {
    queue: Arc<Mutex<Option<UnboundedReceiver<Request>>>>,
    _connection: Option<Connection>,
}

impl std::fmt::Debug for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Requests(..)")
    }
}

impl Requests {
    /// A queue with nobody serving it, for a test to push into.
    pub fn local() -> (Requests, UnboundedSender<Request>) {
        let (sender, queue) = unbounded_channel();
        let requests = Requests {
            queue: Arc::new(Mutex::new(Some(queue))),
            _connection: None,
        };
        (requests, sender)
    }

    /// The queue, once: the window reads it on the thread that draws.
    pub fn take(&self) -> Option<UnboundedReceiver<Request>> {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// The token a call's `platform_data` carries: `activation-token`, a string (the key is
/// `desktop-startup-id` on X11, which no compositor here consumes).
fn token_of(mut platform_data: HashMap<String, OwnedValue>) -> Option<ActivationToken> {
    platform_data
        .remove("activation-token")
        .and_then(|value| String::try_from(value).ok())
        .and_then(ActivationToken::new)
}

/// The object a caller's `org.freedesktop.Application` calls land on.
struct Application(UnboundedSender<Request>);

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        let token = token_of(platform_data);
        let _ = self.0.send(Request::Activate { token });
    }

    fn open(&self, uris: Vec<String>, platform_data: HashMap<String, OwnedValue>) {
        for request in requests_of(&uris, token_of(platform_data)) {
            let _ = self.0.send(request);
        }
    }

    fn activate_action(
        &self,
        _name: String,
        _parameter: Vec<OwnedValue>,
        platform_data: HashMap<String, OwnedValue>,
    ) {
        let token = token_of(platform_data);
        let _ = self.0.send(Request::Activate { token });
    }
}

/// Claim the name and start answering, or `None` when the bus is out of reach or another window
/// already owns the name (this one then simply is not handed to).
pub fn serve() -> Option<Requests> {
    let (requests, sender) = Requests::local();
    let connection = Builder::session()
        .and_then(|builder| builder.name(NAME))
        .and_then(|builder| {
            builder
                .allow_name_replacements(false)
                .replace_existing_names(false)
                .serve_at(PATH, Application(sender))
        })
        .and_then(Builder::build)
        .map_err(|e| eprintln!("opening a conversation from outside is off: {e}"))
        .ok()?;
    Some(Requests {
        _connection: Some(connection),
        ..requests
    })
}

/// Ask the running window to open `thread`, handing it the activation `token` of the click. True
/// when a window took it; false when none is running (or there is no bus), and then the caller
/// opens a window of its own.
pub fn deliver(thread: ThreadId, token: Option<&str>) -> bool {
    let Ok(connection) = Connection::session() else {
        return false;
    };
    let mut platform_data: HashMap<&str, Value<'_>> = HashMap::new();
    if let Some(token) = token {
        platform_data.insert("activation-token", Value::from(token));
    }
    connection
        .call_method(
            Some(NAME),
            PATH,
            Some("org.freedesktop.Application"),
            "Open",
            &(vec![uri_of(thread)], platform_data),
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    #[test]
    fn the_token_is_the_activation_token_string_and_nothing_empty() {
        let data = |key: &str, value: Value<'_>| {
            HashMap::from([(key.to_owned(), OwnedValue::try_from(value).unwrap())])
        };
        assert_eq!(
            token_of(data("activation-token", Value::from("xdg-1"))),
            ActivationToken::new("xdg-1")
        );
        assert_eq!(token_of(data("activation-token", Value::from(""))), None);
        assert_eq!(token_of(data("activation-token", Value::from(7u32))), None);
        assert_eq!(token_of(data("other", Value::from("xdg-1"))), None);
        assert_eq!(token_of(HashMap::new()), None);
    }
}

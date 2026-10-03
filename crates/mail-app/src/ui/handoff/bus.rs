//! The session-bus half of [`super`]: serving `org.freedesktop.Application` from the window, and
//! calling it from `mailo open`.

use super::{NAME, PATH, Request, request_of, uri_of};
use crate::ui::window::open_in_window;
use dioxus::prelude::*;
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

    fn take(&self) -> Option<UnboundedReceiver<Request>> {
        self.queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// The object a caller's `org.freedesktop.Application` calls land on.
struct Application(UnboundedSender<Request>);

#[zbus::interface(name = "org.freedesktop.Application")]
impl Application {
    fn activate(&self, _platform_data: HashMap<String, OwnedValue>) {
        let _ = self.0.send(Request::Activate);
    }

    fn open(&self, uris: Vec<String>, _platform_data: HashMap<String, OwnedValue>) {
        let asked: Vec<Request> = uris.iter().filter_map(|uri| request_of(uri)).collect();
        for request in if asked.is_empty() {
            vec![Request::Activate]
        } else {
            asked
        } {
            let _ = self.0.send(request);
        }
    }

    fn activate_action(
        &self,
        _name: String,
        _parameter: Vec<OwnedValue>,
        _platform_data: HashMap<String, OwnedValue>,
    ) {
        let _ = self.0.send(Request::Activate);
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

/// Read what the running window is asked, on the thread that draws, and open each conversation in
/// a window of its own. Called once, from `App`; a window with no [`Requests`] reads nothing.
pub(in crate::ui) fn use_handoff() {
    let Some(requests) = try_consume_context::<Requests>() else {
        return;
    };
    use_future(move || {
        let requests = requests.clone();
        async move {
            let Some(mut queue) = requests.take() else {
                return;
            };
            while let Some(request) = queue.recv().await {
                match request {
                    Request::Thread(thread) => {
                        // Said on stderr, which the session's journal keeps: the one trace of what
                        // a click did to a window with no other way to ask it.
                        eprintln!("opened conversation {thread}");
                        open_in_window(thread);
                    }
                    Request::Activate => {}
                }
            }
        }
    });
}

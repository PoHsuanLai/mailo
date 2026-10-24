//! The session-bus half of [`super`]: serving `org.freedesktop.Application` from the window, and
//! calling it from `mailo open`.

use super::{ActivationToken, Request, inbox_uri, name_of, path_of, requests_of, uri_of};
use mail_core::profile::ProfileId;
use mail_domain::ThreadId;
use porter_core::AccountId;
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

/// Claim `profile`'s name and start answering, or `None` when the bus is out of reach or another
/// window already owns the name (this one then simply is not handed to).
pub fn serve(profile: &ProfileId) -> Option<Requests> {
    let (requests, sender) = Requests::local();
    let (name, path) = (name_of(profile), path_of(profile));
    let connection = Builder::session()
        .and_then(|builder| builder.name(name))
        .and_then(|builder| {
            builder
                .allow_name_replacements(false)
                .replace_existing_names(false)
                .serve_at(path, Application(sender))
        })
        .and_then(Builder::build)
        .map_err(|e| eprintln!("opening a conversation from outside is off: {e}"))
        .ok()?;
    Some(Requests {
        _connection: Some(connection),
        ..requests
    })
}

/// Ask `profile`'s running window to open `thread`, handing it the activation `token` of the
/// click. True when a window took it; false when none is running (or there is no bus), and then
/// the caller opens a window of its own.
pub fn deliver(profile: &ProfileId, thread: ThreadId, token: Option<&str>) -> bool {
    open(profile, &uri_of(thread), token)
}

/// Ask `profile`'s running window to open a composer on the `mailto:` link `uri`. True when one
/// took it.
pub fn deliver_mailto(profile: &ProfileId, uri: &str, token: Option<&str>) -> bool {
    open(profile, uri, token)
}

/// Ask `profile`'s running window to show `account`'s inbox. True when one took it.
pub fn deliver_inbox(profile: &ProfileId, account: &AccountId, token: Option<&str>) -> bool {
    open(profile, &inbox_uri(account), token)
}

/// Ask `profile`'s running window to raise itself. True when one did; false when none is running.
pub fn deliver_activate(profile: &ProfileId, token: Option<&str>) -> bool {
    call(profile, "Activate", &(platform_data(token),))
}

/// `Open` of the one `uri` on the window's object.
fn open(profile: &ProfileId, uri: &str, token: Option<&str>) -> bool {
    call(profile, "Open", &(vec![uri], platform_data(token)))
}

/// The `platform_data` a call carries: the activation `token`, when there is one.
fn platform_data(token: Option<&str>) -> HashMap<&str, Value<'_>> {
    let mut data = HashMap::new();
    if let Some(token) = token {
        data.insert("activation-token", Value::from(token));
    }
    data
}

/// Call `method` of `org.freedesktop.Application` on `profile`'s running window. False when there
/// is no bus or no window owns the name.
fn call<B>(profile: &ProfileId, method: &str, body: &B) -> bool
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    let Ok(connection) = Connection::session() else {
        return false;
    };
    connection
        .call_method(
            Some(name_of(profile).as_str()),
            path_of(profile).as_str(),
            Some("org.freedesktop.Application"),
            method,
            body,
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::Value;

    /// Every Profile's name and path are ones the bus takes.
    #[test]
    fn every_profiles_name_and_path_are_valid_on_the_bus() {
        let work = ProfileId::try_from("1a2b3c4d".to_owned()).expect("a profile id");
        for profile in [ProfileId::default(), work] {
            let name = name_of(&profile);
            assert!(
                zbus::names::WellKnownName::try_from(name.as_str()).is_ok(),
                "{name}"
            );
            let path = path_of(&profile);
            assert!(
                zbus::zvariant::ObjectPath::try_from(path.as_str()).is_ok(),
                "{path}"
            );
        }
    }

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

    #[test]
    fn a_call_carries_the_activation_token_only_when_there_is_one() {
        let data = platform_data(Some("xdg-1"));
        assert_eq!(data.len(), 1);
        assert_eq!(data.get("activation-token"), Some(&Value::from("xdg-1")));
        assert!(platform_data(None).is_empty());
    }
}

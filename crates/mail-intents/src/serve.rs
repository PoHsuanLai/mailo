//! The bus side: `org.quire.IntentProvider1` at [`PATH`], on the name [`APP`].
//!
//! Each member decodes the JSON text of its argument, calls the [`Provider`] and answers JSON
//! text. Only the router calls a provider: a method call from any other connection is refused,
//! so a process that knows mailo's bus name cannot get round the gate the router keeps. `Summon`
//! is the one member the shell calls, and a process with no window declines it.
//!
//! The members and their signatures are held to `IntentProvider1.xml`, the interface as the
//! router declares it, by a test.

use super::wire::{Invocation, SuggestAsk, answer_of};
use super::{APP, PATH, Provider, ROUTER};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use zbus::fdo;
use zbus::message::Header;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::OwnedValue;

/// Why the provider is not serving.
#[derive(Debug)]
pub enum ServeError {
    /// The bus could not be reached, or refused something.
    Bus(zbus::Error),
    /// Another process already answers for [`APP`].
    Taken,
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Bus(why) => write!(f, "the session bus: {why}"),
            ServeError::Taken => write!(f, "another process already answers for {APP}"),
        }
    }
}

impl std::error::Error for ServeError {}

impl From<zbus::Error> for ServeError {
    fn from(why: zbus::Error) -> Self {
        ServeError::Bus(why)
    }
}

fn parse<T: DeserializeOwned>(text: &str) -> fdo::Result<T> {
    serde_json::from_str(text).map_err(|e| fdo::Error::InvalidArgs(e.to_string()))
}

fn render<T: Serialize>(value: &T) -> fdo::Result<String> {
    serde_json::to_string(value).map_err(|e| fdo::Error::Failed(e.to_string()))
}

/// Whether the caller is the router: the owner of its well-known name.
async fn from_router(connection: &zbus::Connection, header: &Header<'_>) -> fdo::Result<()> {
    let bus = fdo::DBusProxy::new(connection).await?;
    let name =
        zbus::names::BusName::try_from(ROUTER).map_err(|e| fdo::Error::Failed(e.to_string()))?;
    match (bus.get_name_owner(name).await, header.sender()) {
        (Ok(owner), Some(sender)) if owner.as_str() == sender.as_str() => Ok(()),
        _ => Err(fdo::Error::AccessDenied(
            "only the intent router calls a provider".into(),
        )),
    }
}

pub(super) struct Object {
    pub(super) provider: Provider,
}

#[zbus::interface(name = "org.quire.IntentProvider1")]
impl Object {
    async fn perform(
        &self,
        invocation: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        let _ = options;
        from_router(connection, &header).await?;
        let invocation: Invocation = parse(&invocation)?;
        Ok(answer_of(&self.provider.perform(&invocation)))
    }

    async fn dry_run(
        &self,
        invocation: String,
        options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        let _ = options;
        from_router(connection, &header).await?;
        let invocation: Invocation = parse(&invocation)?;
        Ok(answer_of(&self.provider.dry_run(&invocation)))
    }

    async fn undo(
        &self,
        token: String,
        actor: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        from_router(connection, &header).await?;
        let token: String = parse(&token)?;
        // Who undoes it is the router's to record; mailo only checks that it is well formed.
        let _: serde_json::Value = parse(&actor)?;
        Ok(answer_of(&self.provider.undo(&token)))
    }

    async fn context(
        &self,
        scope: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        from_router(connection, &header).await?;
        let _: serde_json::Value = parse(&scope)?;
        render(&self.provider.context())
    }

    async fn summon(&self, serial: u64, origin: String) -> fdo::Result<String> {
        let _ = (serial, origin);
        render(&"declined")
    }

    async fn search(
        &self,
        text: String,
        generation: u64,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        let _ = generation;
        from_router(connection, &header).await?;
        render(&self.provider.search(&text))
    }

    async fn preview(
        &self,
        entity: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        from_router(connection, &header).await?;
        render(&self.provider.preview(&parse(&entity)?))
    }

    async fn suggest(
        &self,
        ask: String,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> fdo::Result<String> {
        from_router(connection, &header).await?;
        let ask: SuggestAsk = parse(&ask)?;
        render(&self.provider.suggest(&ask))
    }

    // Nothing resolves keys yet. (A plain comment: a doc comment lands in the introspection.)
    async fn resolve(&self, keys: String) -> fdo::Result<String> {
        let _ = keys;
        Err(fdo::Error::NotSupported("mailo resolves no keys".into()))
    }

    #[zbus(signal)]
    async fn undo_changed(
        emitter: &SignalEmitter<'_>,
        token: &str,
        state: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn index_stale(emitter: &SignalEmitter<'_>, epoch: u64) -> zbus::Result<()>;
}

/// Export `provider` at [`PATH`] on `connection` and claim [`APP`]. Returns once the name is
/// ours; the connection keeps serving. A name that is taken is not queued for: a second
/// `mailo intents` has nothing to do.
pub async fn serve_on(connection: &zbus::Connection, provider: Provider) -> Result<(), ServeError> {
    connection
        .object_server()
        .at(PATH, Object { provider })
        .await?;
    let reply = connection
        .request_name_with_flags(APP, fdo::RequestNameFlags::DoNotQueue.into())
        .await
        .map_err(|why| match why {
            zbus::Error::NameTaken => ServeError::Taken,
            other => ServeError::Bus(other),
        })?;
    match reply {
        fdo::RequestNameReply::PrimaryOwner | fdo::RequestNameReply::AlreadyOwner => Ok(()),
        fdo::RequestNameReply::InQueue | fdo::RequestNameReply::Exists => Err(ServeError::Taken),
    }
}

/// Serve on the session bus until the bus goes away, which is when the session ends.
pub fn serve(provider: Provider) -> Result<(), ServeError> {
    use futures_util::StreamExt as _;
    zbus::block_on(async {
        let connection = zbus::connection::Builder::session()?.build().await?;
        serve_on(&connection, provider).await?;
        let mut messages = zbus::MessageStream::from(&connection);
        while messages.next().await.is_some() {}
        Ok(())
    })
}

//! The D-Bus link to accountd (feature `quire-desktop`, Linux): porter-client's `DbusTransport`
//! for the questions, porter-dbus's proxy for the signals.
//!
//! Desktop extras are lit by capability (quire design/36 rule 2): nothing here runs unless the
//! capability probe said accountd is here, and a failure to connect after that is the in-process
//! path, not an error ([`super::start`]).

use super::client::Client;
use super::{Accountd, Change, Changes};
use futures_util::StreamExt;
use porter_client::{Accounts, DbusTransport};
use porter_core::consent::Usage;
use porter_dbus::{BusConnection, ManagerProxy};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// A connection to the session bus, and the link over it, or `None` when the bus is not there.
///
/// Opened on whichever runtime this is awaited on, which for the process's link is the long-lived
/// one ([`super::start`]).
pub async fn session(usage: Usage) -> Option<Arc<dyn Accountd>> {
    let connection = BusConnection::session().await.ok()?;
    Some(over(connection, usage))
}

/// The link over `connection`: a private bus in a test, the session bus in the desktop.
pub fn over(connection: BusConnection, usage: Usage) -> Arc<dyn Accountd> {
    let accounts = Accounts::over(DbusTransport::over(connection.clone()));
    let client = Client::new(accounts, usage).with_changes(move || {
        let connection = connection.clone();
        Box::pin(async move { Ok(follow(&connection).await) })
    });
    Arc::new(client)
}

/// `AccountAdded`, `AccountRemoved` and `GrantChanged` on `connection`, as [`Change`]s.
async fn follow(connection: &BusConnection) -> Option<Box<dyn Changes>> {
    let manager = ManagerProxy::new(connection).await.ok()?;
    let added = manager.receive_account_added().await.ok()?;
    let removed = manager.receive_account_removed().await.ok()?;
    let granted = manager.receive_grant_changed().await.ok()?;
    let streams: Vec<Feed> = vec![
        Box::pin(added.map(|_| Change::Added)),
        Box::pin(removed.map(|signal| {
            // All the signal carries is the account's object path; its last segment is
            // `porter_core::object_segment` of the id.
            let segment = signal
                .args()
                .ok()
                .and_then(|args| args.account().rsplit('/').next().map(str::to_owned))
                .unwrap_or_default();
            Change::Removed(segment)
        })),
        Box::pin(granted.map(|_| Change::Granted)),
    ];
    Some(Box::new(Signals(futures_util::stream::select_all(streams))))
}

type Feed = Pin<Box<dyn futures_util::Stream<Item = Change> + Send>>;

struct Signals(futures_util::stream::SelectAll<Feed>);

impl Changes for Signals {
    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Change>> + Send + '_>> {
        Box::pin(self.0.next())
    }
}

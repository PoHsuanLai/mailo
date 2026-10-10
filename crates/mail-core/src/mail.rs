//! The one handle on mailo's logic: [`Mail`].
//!
//! A front-end builds a `Mail` once, at its edge, from the pieces only it can choose: the store it
//! opened, the link to accountd it settled on, the runtime it owns, the environment it was started
//! with, and (in a test) a clock. Everything this crate does that touches a server, the keyring or
//! the environment is then a method on it, grouped by what it is for:
//!
//! - [`Mail::accounts`]: adding, listing and removing accounts;
//! - [`Mail::sync`]: passes, watching, fetching a body or an attachment, a folder, a server search;
//! - [`Mail::contacts`]: the address book commands;
//! - [`Mail::rules`]: pushing rules and the vacation reply to a server;
//! - [`Mail::crypto`]: finding other people's keys;
//! - [`Mail::discover`]: finding an address's servers.
//!
//! Every operation that waits on a network is `async`. This crate starts no runtime: the
//! front-end awaits the call on the one it owns, and the [`Handle`] it gave `Mail` is where the
//! secret store keeps its connection.
//!
//! The pieces of the old free functions that a test wants to vary (the secret store, the OAuth
//! registry, the time) are still parameters of their `_with` forms, for tests that run one
//! function without a `Mail`.

use crate::environment::Environment;
use chrono::{DateTime, Utc};
use mail_runtime::{AccountSecrets, ClientRegistry, Link, platform_secrets};
use mail_store::SqliteStore;
use std::sync::Arc;
use tokio::runtime::Handle;

/// What time it is. The clock a [`Mail`] reads, so a test can stand still.
pub trait Clock: Send + Sync {
    /// Now.
    fn now(&self) -> DateTime<Utc>;
}

/// The wall clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// A clock that says one time until told another. For tests.
#[derive(Debug)]
pub struct FixedClock(std::sync::Mutex<DateTime<Utc>>);

impl FixedClock {
    /// A clock stopped at `at`.
    pub fn at(at: DateTime<Utc>) -> Self {
        Self(std::sync::Mutex::new(at))
    }

    /// Moves the clock to `to`.
    pub fn set(&self, to: DateTime<Utc>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = to;
    }
}

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The store, the link, the secrets, the environment, the clock and the runtime handle that
/// mailo's operations run over. See the module documentation.
#[derive(Clone)]
pub struct Mail {
    store: Arc<SqliteStore>,
    link: Link,
    secrets: Arc<dyn AccountSecrets>,
    environment: Environment,
    clients: Option<ClientRegistry>,
    clock: Arc<dyn Clock>,
    runtime: Handle,
}

impl std::fmt::Debug for Mail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mail")
            .field("link", &self.link.name())
            .finish_non_exhaustive()
    }
}

impl Mail {
    /// A handle over `store`, signed in through `link`, with `runtime` as the one the
    /// application owns. The secrets are the platform's (or accountd's, when linked), the
    /// environment is empty and the clock is the wall's; change any with the `with_` methods.
    pub fn new(store: Arc<SqliteStore>, link: Link, runtime: Handle) -> Self {
        let secrets = platform_secrets(&link, &runtime);
        Self {
            store,
            link,
            secrets,
            environment: Environment::default(),
            clients: None,
            clock: Arc::new(SystemClock),
            runtime,
        }
    }

    /// With the environment the program was started with.
    #[must_use]
    pub fn with_environment(mut self, environment: Environment) -> Self {
        self.environment = environment;
        self
    }

    /// With these OAuth clients, in place of the ones recorded in the config directory: a test's.
    #[must_use]
    pub fn with_clients(mut self, clients: ClientRegistry) -> Self {
        self.clients = Some(clients);
        self
    }

    /// With another clock.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// With another store of account secrets, in place of the platform's: a test's.
    #[must_use]
    pub fn with_secrets(mut self, secrets: Arc<dyn AccountSecrets>) -> Self {
        self.secrets = secrets;
        self
    }

    /// The store.
    pub fn store(&self) -> &Arc<SqliteStore> {
        &self.store
    }

    /// The link to accountd this handle was made with.
    pub fn link(&self) -> &Link {
        &self.link
    }

    /// Where account secrets are kept, or asked for: the link's, when there is one.
    pub fn secrets(&self) -> Arc<dyn AccountSecrets> {
        self.secrets.clone()
    }

    /// What the program was started with.
    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    /// The runtime the application owns.
    pub fn runtime(&self) -> &Handle {
        &self.runtime
    }

    /// Now, by this handle's clock.
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The OAuth clients earlier sign-ins recorded, read from the config directory (or the ones
    /// given with [`Mail::with_clients`]). A registry that cannot be read is an error.
    pub fn clients(&self) -> Result<ClientRegistry, crate::CoreError> {
        match &self.clients {
            Some(clients) => Ok(clients.clone()),
            None => Ok(mail_runtime::clients::load_default()?),
        }
    }

    /// The same, for the places that sign in and can do without: a registry that cannot be read is
    /// logged and empty.
    pub fn saved_clients(&self) -> ClientRegistry {
        match &self.clients {
            Some(clients) => clients.clone(),
            None => crate::account::saved_clients(),
        }
    }

    /// Adding, listing and removing accounts.
    pub fn accounts(&self) -> AccountOps<'_> {
        AccountOps(self)
    }

    /// Passes, watching, and fetching on demand.
    pub fn sync(&self) -> SyncOps<'_> {
        SyncOps(self)
    }

    /// The address book commands.
    pub fn contacts(&self) -> ContactOps<'_> {
        ContactOps(self)
    }

    /// Rules and the vacation reply, on a server.
    pub fn rules(&self) -> RuleOps<'_> {
        RuleOps(self)
    }

    /// Other people's keys.
    pub fn crypto(&self) -> CryptoOps<'_> {
        CryptoOps(self)
    }

    /// Finding an address's servers.
    pub fn discover(&self) -> DiscoverOps<'_> {
        DiscoverOps(self)
    }
}

/// [`Mail::accounts`]. The methods are in [`crate::account`].
#[derive(Debug, Clone, Copy)]
pub struct AccountOps<'a>(pub(crate) &'a Mail);

/// [`Mail::sync`]. The methods are in [`crate::sync`].
#[derive(Debug, Clone, Copy)]
pub struct SyncOps<'a>(pub(crate) &'a Mail);

/// [`Mail::contacts`]. The methods are in [`crate::contacts`].
#[derive(Debug, Clone, Copy)]
pub struct ContactOps<'a>(pub(crate) &'a Mail);

/// [`Mail::rules`]. The methods are in [`crate::rules::server`].
#[derive(Debug, Clone, Copy)]
pub struct RuleOps<'a>(pub(crate) &'a Mail);

/// [`Mail::crypto`]. The methods are in [`crate::pgp`].
#[derive(Debug, Clone, Copy)]
pub struct CryptoOps<'a>(pub(crate) &'a Mail);

/// [`Mail::discover`]. The methods are in [`crate::discover`].
#[derive(Debug, Clone, Copy)]
pub struct DiscoverOps<'a>(pub(crate) &'a Mail);

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use porter_secrets::MemorySecrets;

    fn handle() -> (Mail, tempfile::TempDir, tokio::runtime::Runtime) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SqliteStore::in_memory(dir.path()).unwrap());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mail = Mail::new(store, Link::Local, runtime.handle().clone())
            .with_secrets(Arc::new(MemorySecrets::default()));
        (mail, dir, runtime)
    }

    #[test]
    fn a_handle_carries_what_it_was_built_with_and_nothing_global() {
        let (mail, _dir, _runtime) = handle();
        assert!(!mail.link().is_linked());
        assert_eq!(*mail.environment(), Environment::default());
        let given = Environment {
            password: Some("hunter2".to_owned()),
            ..Environment::default()
        };
        let other = mail.clone().with_environment(given.clone());
        assert_eq!(*other.environment(), given);
        assert_eq!(
            *mail.environment(),
            Environment::default(),
            "a second handle in the same process has its own environment"
        );
    }

    #[test]
    fn the_clock_is_the_handles_own() {
        let (mail, _dir, _runtime) = handle();
        let noon = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap();
        let clock = Arc::new(FixedClock::at(noon));
        let mail = mail.with_clock(clock.clone());
        assert_eq!(mail.now(), noon);
        let later = noon + chrono::TimeDelta::hours(3);
        clock.set(later);
        assert_eq!(mail.now(), later);
    }

    #[test]
    fn the_secrets_are_the_ones_given_and_an_unlinked_store_has_no_link() {
        let (mail, _dir, _runtime) = handle();
        assert!(mail.secrets().link().is_none());
    }

    #[test]
    fn the_oauth_clients_can_be_handed_in_rather_than_read_from_the_config_directory() {
        let (mail, _dir, _runtime) = handle();
        let none = ClientRegistry::default();
        let mail = mail.with_clients(none.clone());
        assert_eq!(mail.clients().unwrap(), none);
        assert_eq!(mail.saved_clients(), none);
    }
}

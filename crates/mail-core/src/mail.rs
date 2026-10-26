//! The one handle on mailo's logic: [`Mail`].
//!
//! A front-end builds a `Mail` once, at its edge, from the pieces only it can choose: the store it
//! opened, the runtime it owns, the environment it was started with, the OAuth clients it signs in
//! as ([`OAuthClients`]), and (in a test) a clock. Everything this crate does that touches a server, the keyring or
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
use crate::error::Logged;
use chrono::{DateTime, Utc};
use mail_runtime::{
    AccountSecrets, ClientRegistry, KeyringSigningStore, SigningStore, platform_secrets,
};
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

/// Where a [`Mail`]'s OAuth clients come from: chosen by the front-end at its edge, never by this
/// crate.
///
/// A handle starts with none at hand ([`OAuthClients::default`]): every OAuth sign-in it starts
/// stops at "needs a client id", and nothing reaches an issuer. The program hands it
/// [`OAuthClients::here`], this installation's; a test hands it the clients it means to have
/// (`mail_runtime::testing::fake_clients`), or none on purpose.
#[derive(Debug, Clone)]
pub enum OAuthClients {
    /// Exactly these.
    Given(ClientRegistry),
    /// Read from these files at each use, so a client a sign-in records, or one set up in porter's
    /// Settings while the program runs, is the next sign-in's and the next renewal's.
    Files(mail_runtime::clients::Files),
}

impl Default for OAuthClients {
    /// None at hand.
    fn default() -> Self {
        OAuthClients::Given(ClientRegistry::default())
    }
}

impl OAuthClients {
    /// This installation's: porter's shipped clients, the person's own, and mailo's earlier
    /// `oauth.json`. For the program's edge alone: a test that called it
    /// would sign in as mailo with whatever the machine it runs on has set up.
    pub fn here() -> Self {
        OAuthClients::Files(mail_runtime::clients::Files::here())
    }

    /// The registry now. One that cannot be read is an error, never read as empty (that would
    /// report a configured client as missing).
    pub fn load(&self) -> Result<ClientRegistry, crate::CoreError> {
        match self {
            OAuthClients::Given(clients) => Ok(clients.clone()),
            OAuthClients::Files(files) => Ok(mail_runtime::clients::load(files)?),
        }
    }

    /// Whether these clients can sign a personal Microsoft account in
    /// (`mail_runtime::clients::personal_microsoft_ready`).
    pub fn takes_personal_microsoft(&self) -> bool {
        mail_runtime::clients::personal_microsoft_ready(&self.saved())
    }

    /// The same, for the places that sign in and can do without: a registry that cannot be read is
    /// logged and empty.
    pub fn saved(&self) -> ClientRegistry {
        self.load()
            .or_log_default("the recorded OAuth clients could not be read")
    }
}

/// The store, the secrets, the environment, the OAuth clients, the clock and the runtime handle
/// that mailo's operations run over. See the module documentation.
#[derive(Clone)]
pub struct Mail {
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    signing: Arc<dyn SigningStore>,
    environment: Environment,
    clients: OAuthClients,
    clock: Arc<dyn Clock>,
    runtime: Handle,
}

impl std::fmt::Debug for Mail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mail").finish_non_exhaustive()
    }
}

impl Mail {
    /// A handle over `store`, with `runtime` as the one the application owns. The secrets are the
    /// platform's, the environment is empty, no OAuth client is at hand and the clock is the
    /// wall's; change any with the `with_` methods.
    pub fn new(store: Arc<SqliteStore>, runtime: Handle) -> Self {
        let secrets = platform_secrets(&runtime);
        Self {
            store,
            secrets,
            signing: Arc::new(KeyringSigningStore::default()),
            environment: Environment::default(),
            clients: OAuthClients::default(),
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

    /// With exactly these OAuth clients: a test's, or a caller's that has them in hand.
    #[must_use]
    pub fn with_clients(mut self, clients: ClientRegistry) -> Self {
        self.clients = OAuthClients::Given(clients);
        self
    }

    /// With the OAuth clients from `clients`: the program's ([`OAuthClients::here`]).
    #[must_use]
    pub fn with_oauth_clients(mut self, clients: OAuthClients) -> Self {
        self.clients = clients;
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

    /// With another store of signing keys, in place of the platform's under the service `mailo`: a
    /// Profile's own (`KeyringSigningStore::for_service`, with `Profile::signing_service`), or a
    /// test's.
    #[must_use]
    pub fn with_signing(mut self, signing: Arc<dyn SigningStore>) -> Self {
        self.signing = signing;
        self
    }

    /// The store.
    pub fn store(&self) -> &Arc<SqliteStore> {
        &self.store
    }

    /// Where account secrets are kept.
    pub fn secrets(&self) -> Arc<dyn AccountSecrets> {
        self.secrets.clone()
    }

    /// Where signing keys are kept: OpenPGP secret keys and S/MIME private keys, which are mailo's
    /// own and not an account's.
    pub fn signing(&self) -> Arc<dyn SigningStore> {
        self.signing.clone()
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

    /// The OAuth clients this handle was given, as a registry now. A registry that cannot be read
    /// is an error.
    pub fn clients(&self) -> Result<ClientRegistry, crate::CoreError> {
        self.clients.load()
    }

    /// The same, for the places that sign in and can do without: a registry that cannot be read is
    /// logged and empty.
    pub fn saved_clients(&self) -> ClientRegistry {
        self.clients.saved()
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

    fn handle() -> (Mail, tokio::runtime::Runtime) {
        let store = Arc::new(mail_store::testing::in_memory());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mail = Mail::new(store, runtime.handle().clone())
            .with_secrets(Arc::new(MemorySecrets::default()));
        (mail, runtime)
    }

    #[test]
    fn a_handle_carries_what_it_was_built_with_and_nothing_global() {
        let (mail, _runtime) = handle();
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
        let (mail, _runtime) = handle();
        let noon = Utc.with_ymd_and_hms(2026, 1, 2, 12, 0, 0).unwrap();
        let clock = Arc::new(FixedClock::at(noon));
        let mail = mail.with_clock(clock.clone());
        assert_eq!(mail.now(), noon);
        let later = noon + chrono::TimeDelta::hours(3);
        clock.set(later);
        assert_eq!(mail.now(), later);
    }

    #[test]
    fn a_handle_has_the_oauth_clients_it_is_handed_and_none_until_it_is() {
        let (mail, _runtime) = handle();
        // Nothing handed: no client at hand, whatever this machine has set up.
        let none = ClientRegistry::default();
        assert_eq!(mail.clients().unwrap(), none);
        assert_eq!(mail.saved_clients(), none);
        let fake = mail_runtime::testing::fake_clients();
        let given = mail.clone().with_clients(fake.clone());
        assert_eq!(given.clients().unwrap(), fake);
        assert_eq!(given.saved_clients(), fake);
        // Files are read at each use, and a file that is not there is empty.
        let dir = tempfile::tempdir().unwrap();
        let files = mail.with_oauth_clients(OAuthClients::Files(mail_runtime::clients::Files {
            own: Some(dir.path().join("clients.toml")),
            ..mail_runtime::clients::Files::default()
        }));
        assert_eq!(files.clients().unwrap(), none);
    }
}

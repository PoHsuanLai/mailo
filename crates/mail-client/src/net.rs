//! What the client asks of servers for a caller: a folder fetched now, a body, a key looked up, an
//! account added. Each waits on the program's runtime, so a caller makes it from a thread that may
//! wait (a blocking task), never from the runtime's own.
//!
//! How servers are reached is the program's to say ([`Reach`], [`Client::reach`]): its environment,
//! its OAuth clients, its keyring and its runtime. A client never told reaches them with none of
//! those: no keyring, so nothing signs in, which is what a test wants of a call it did not fake.

use crate::Client;
use chrono::{DateTime, Utc};
use mail_core::account::{Added, Credentials, RemoveError, Removed, Setup};
use mail_core::sync::Configured;
use mail_core::sync::report::PassEnd;
use mail_core::{AccountSecrets, CoreError, Mail, NoKeyring, Pushed, Searched};
use mail_domain::{MessageId, PgpKey, Retry, Section};
use mail_store::SqliteStore;
use porter_core::AccountId;
use std::future::Future;
use std::sync::{Arc, OnceLock};

/// How a client reaches servers: the [`Mail`] the program makes over a store, with what it was
/// started with.
#[derive(Clone)]
pub struct Reach(Arc<dyn Fn(Arc<SqliteStore>) -> Mail + Send + Sync>);

impl Reach {
    pub fn new(mail: impl Fn(Arc<SqliteStore>) -> Mail + Send + Sync + 'static) -> Reach {
        Reach(Arc::new(mail))
    }
}

impl std::fmt::Debug for Reach {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Reach")
    }
}

impl Client {
    /// Reach servers as `reach` says from now on. Only the first call counts.
    pub fn reach(&self, reach: Reach) {
        // A second program is not a thing: the first one told stays.
        let _ = self.reached().set(reach);
    }

    /// What the client asks of servers.
    pub fn net(&self) -> Net<'_> {
        Net(self)
    }
}

/// What the client asks of servers ([`Client::net`]).
#[derive(Clone, Copy, Debug)]
pub struct Net<'a>(&'a Client);

/// The runtime a client never told how to reach servers waits on.
static UNREACHED: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

/// The [`Mail`] over `store`, as `reach` says, or with no keyring when nothing does.
fn mail_over(reach: Option<&Reach>, store: Arc<SqliteStore>) -> Mail {
    match reach {
        Some(reach) => (reach.0)(store),
        None => {
            let runtime = UNREACHED.get_or_init(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(1)
                    .thread_name("mailo-unreached")
                    .enable_all()
                    .build()
                    .expect("a runtime for a client that reaches no server")
            });
            Mail::new(store, runtime.handle().clone())
                .with_secrets(Arc::new(NoKeyring))
                .with_signing(Arc::new(NoKeyring))
        }
    }
}

impl Net<'_> {
    fn mail(&self) -> Mail {
        mail_over(self.0.reached().get(), self.0.store_arc())
    }

    fn wait<T>(mail: &Mail, work: impl Future<Output = T>) -> T {
        mail.runtime().block_on(work)
    }

    /// The servers' passes, for fetching ([`crate::fetch::Seams::pass`]).
    pub fn pass(&self) -> crate::fetch::Pass {
        let reach = self.0.reached().get().cloned();
        Arc::new(move |store, _now, account, hooks| {
            let mail = mail_over(reach.as_ref(), store);
            Self::wait(&mail, mail.sync().run_due(&[account], hooks)).map_err(String::from)
        })
    }

    /// The servers' watches, for fetching ([`crate::fetch::Seams::listen`]).
    pub fn listen(&self) -> crate::fetch::Listen {
        let reach = self.0.reached().get().cloned();
        Arc::new(move |store, account, stop, hold, heard| {
            let mail = mail_over(reach.as_ref(), store);
            Self::wait(
                &mail,
                mail.sync().listen(
                    account,
                    stop,
                    Some(hold),
                    mail_core::sync::live::GRACE,
                    heard,
                ),
            )
        })
    }

    /// Fetch one folder now: one no pass fetches, opened.
    pub fn folder_now(&self, account: AccountId, path: &str) -> Result<PassEnd, CoreError> {
        let mail = self.mail();
        Self::wait(&mail, mail.sync().folder_now(account, path))
    }

    /// Download one message's body.
    pub fn fetch_body(&self, message: MessageId) -> Result<(), (Retry, String)> {
        let mail = self.mail();
        Self::wait(&mail, mail.sync().fetch_body(message))
    }

    /// Download one attachment left on the server.
    pub fn fetch_part(&self, message: MessageId, section: &Section) -> Result<(), CoreError> {
        let mail = self.mail();
        Self::wait(&mail, mail.sync().fetch_part(message, section))
    }

    /// Send what an account has queued to go up (an import's messages).
    pub fn drain(&self, account: AccountId) -> Result<mail_runtime::SyncReport, CoreError> {
        let mail = self.mail();
        Self::wait(&mail, mail.sync().drain(account))
    }

    /// Put an account's rules on its server, never over a script someone else made.
    pub fn push_rules(&self, account: &Configured) -> Result<Pushed, CoreError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail.rules()
                .push(account, mail_proto::sieve::Takeover::Refuse),
        )
    }

    /// Set an account's kept vacation reply where its JMAP or Graph server keeps one.
    pub fn put_away(&self, account: &Configured) -> mail_core::rules::server::PushDone {
        let mail = self.mail();
        Self::wait(&mail, mail.rules().put_away(account))
    }

    /// How much of an account's storage its server says is used.
    pub fn quota(&self, account: AccountId) -> Result<Option<mail_core::Storage>, CoreError> {
        let mail = self.mail();
        Self::wait(&mail, mail.sync().quota(account))
    }

    /// Ask `address`'s domain for its key (Web Key Directory), keeping what it gives.
    pub fn lookup_key(
        &self,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<PgpKey>, mail_core::pgp::PgpError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::pgp::lookup_address(self.0.store(), address, now),
        )
    }

    /// Search one account's server for `input`.
    pub fn search_server(&self, account: AccountId, input: &str) -> Result<Searched, CoreError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::server_search::search(&mail, account, input),
        )
    }

    /// Leave a mailing list the way `found` says.
    pub fn unsubscribe(
        &self,
        found: &mail_core::unsubscribe::Found,
        http: &reqwest::Client,
        now: DateTime<Utc>,
    ) -> Result<mail_core::unsubscribe::Outcome, CoreError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::unsubscribe::perform(self.0.store(), found, http, now),
        )
    }

    /// Find how `address`'s provider is reached.
    pub fn discover(
        &self,
        address: &str,
        now: DateTime<Utc>,
    ) -> Result<mail_core::discover::Found, mail_core::discover::Failed> {
        let mail = self.mail();
        Self::wait(&mail, mail_core::discover::search(address, now))
    }

    /// Follow a JMAP session `url`, without a credential.
    pub fn find_jmap(&self, url: &str) -> Result<String, CoreError> {
        let mail = self.mail();
        Self::wait(&mail, mail_core::discover::find_jmap(url))
    }

    /// Add an account, or set up again one already here.
    pub fn add_with_password(
        &self,
        address: &str,
        manual: Option<&Setup>,
        microsoft: bool,
        graph: bool,
        now: DateTime<Utc>,
        credentials: Credentials<'_>,
    ) -> Result<Added, CoreError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::account::add_with_password(
                self.0.store(),
                address,
                manual,
                microsoft,
                graph,
                now,
                credentials,
            ),
        )
    }

    /// Move a Microsoft account onto Graph.
    pub fn switch_to_graph(
        &self,
        account: AccountId,
        now: DateTime<Utc>,
        credentials: Credentials<'_>,
    ) -> Result<Added, CoreError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::account::switch_to_graph(self.0.store(), account, now, credentials),
        )
    }

    /// Sign in to `address`'s server once, to see that its password is right.
    pub fn check_sign_in(
        &self,
        address: &str,
        secrets: Arc<dyn AccountSecrets>,
        now: DateTime<Utc>,
    ) -> Result<(), CoreError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::sync::check_sign_in(&self.0.store_arc(), address, secrets, now),
        )
    }

    /// Remove an account and what it kept here, and its secrets.
    pub fn remove_account(
        &self,
        secrets: &dyn AccountSecrets,
        account: AccountId,
    ) -> Result<Removed, RemoveError> {
        let mail = self.mail();
        Self::wait(
            &mail,
            mail_core::account::remove(self.0.store(), secrets, account),
        )
    }
}

/// What a sign-in in the browser is given: [`mail_core::account::Direct`] but the store, which is
/// the client's.
#[derive(Clone, Copy)]
pub struct SignIn<'a> {
    pub secrets: &'a dyn AccountSecrets,
    pub saved: &'a mail_core::ClientRegistry,
    pub environment: &'a mail_core::Environment,
    pub browse: &'a dyn Fn(&str) -> std::io::Result<()>,
    pub profiles: &'a mail_core::account::direct::Profiles,
    pub wait: std::time::Duration,
    pub now: DateTime<Utc>,
}

impl std::fmt::Debug for SignIn<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignIn").finish_non_exhaustive()
    }
}

impl Net<'_> {
    /// Add an account by signing in to `issuer` in the browser, until `cancel` ends it.
    pub fn sign_in(
        &self,
        issuer: porter_provider::Issuer,
        mode: mail_core::account::Mode,
        given: SignIn<'_>,
        cancel: impl Future<Output = ()>,
    ) -> Result<Added, mail_core::account::SignInError> {
        let mail = self.mail();
        let SignIn {
            secrets,
            saved,
            environment,
            browse,
            profiles,
            wait,
            now,
        } = given;
        Self::wait(
            &mail,
            mail_core::account::sign_in_with(
                issuer,
                mode,
                mail_core::account::Direct {
                    store: self.0.store(),
                    secrets,
                    saved,
                    environment,
                    browse,
                    profiles,
                    wait,
                    now,
                },
                cancel,
            ),
        )
    }
}

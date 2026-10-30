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
use mail_core::{
    AccountSecrets, CoreError, Environment, Mail, NoKeyring, OAuthClients, Pushed, Searched,
    SigningStore,
};
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

/// Said once, by the first call made on a client never told how to reach servers.
static UNREACHED_SAID: std::sync::Once = std::sync::Once::new();

/// The [`Mail`] over `store`, as `reach` says, or with no keyring when nothing does.
///
/// A program always says ([`Client::reach`]), so a client that was not told is a program that
/// forgot to, and every sign-in it attempts will fail for want of a keyring: said once in the
/// log, and a debug build stops. A test, which wants exactly that of a call it did not fake, is
/// built with the `testing` feature and is neither.
fn mail_over(reach: Option<&Reach>, store: Arc<SqliteStore>) -> Mail {
    match reach {
        Some(reach) => (reach.0)(store),
        None => {
            UNREACHED_SAID.call_once(|| {
                log::warn!(
                    "a client that was not told how to reach servers was asked to: it has no keyring"
                );
            });
            #[cfg(not(feature = "testing"))]
            debug_assert!(
                false,
                "a client was asked to reach servers before the program said how (Client::reach)"
            );
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

/// How long a [`Mail`] made for [`Net`] is reused. Making one reads `settings.toml` twice and
/// opens a keyring handle, and a caller asks for several in a row. What it holds from the
/// settings (the junk filter's policy) is only for a sync pass, which makes its own, so this is
/// the longest a change to them waits to be seen by the rest.
const MAIL_KEPT: std::time::Duration = std::time::Duration::from_secs(5);

impl Net<'_> {
    fn mail(&self) -> Mail {
        // A client not yet told how to reach servers makes one each time, so it keeps nothing
        // that a later `Client::reach` would have to replace.
        if self.0.reached().get().is_none() {
            return mail_over(None, self.0.store_arc());
        }
        let mut kept = self.0.mail_kept().lock();
        if let Some((made, mail)) = kept.as_ref()
            && made.elapsed() < MAIL_KEPT
        {
            return mail.clone();
        }
        let mail = mail_over(self.0.reached().get(), self.0.store_arc());
        *kept = Some((std::time::Instant::now(), mail.clone()));
        mail
    }

    /// Where account secrets are kept: the program's keyring, or none for a client never told.
    pub fn secrets(&self) -> Arc<dyn AccountSecrets> {
        self.mail().secrets()
    }

    /// Where signing keys are kept, as [`Net::secrets`].
    pub fn signing(&self) -> Arc<dyn SigningStore> {
        self.mail().signing()
    }

    /// The privacy relay's key or token, kept in the keyring: `None` while none is kept. Blocks on
    /// the keyring, so a caller reads it off the thread that draws.
    pub fn relay_key(&self) -> Result<Option<String>, String> {
        use mail_domain::signing::{SigningKeyRef, SigningSecret};
        match self.signing().get(&SigningKeyRef::privacy_relay()) {
            Ok(SigningSecret::Relay(key)) => Ok(Some(key).filter(|key| !key.is_empty())),
            Ok(_) => Ok(None),
            Err(why) => match mail_secrets::SecretTrouble::of(&why) {
                mail_secrets::SecretTrouble::Absent => Ok(None),
                mail_secrets::SecretTrouble::Unreachable(said) => Err(said.to_owned()),
            },
        }
    }

    /// Keep `key` as the privacy relay's, in the keyring; an empty one forgets it. Blocks.
    pub fn set_relay_key(&self, key: &str) -> Result<(), String> {
        use mail_domain::signing::{SigningKeyRef, SigningSecret};
        let at = SigningKeyRef::privacy_relay();
        let key = key.trim();
        let done = if key.is_empty() {
            self.signing().forget(&at)
        } else {
            self.signing()
                .put(&at, &SigningSecret::Relay(key.to_owned()))
        };
        done.map_err(|why| why.to_string())
    }

    /// What the program was started with.
    pub fn environment(&self) -> Environment {
        self.mail().environment().clone()
    }

    /// The OAuth clients the program signs in as.
    pub fn oauth_clients(&self) -> OAuthClients {
        self.mail().oauth_clients().clone()
    }

    /// The runtime servers are reached on.
    pub fn runtime(&self) -> tokio::runtime::Handle {
        self.mail().runtime().clone()
    }

    /// Run the future `work` makes on a thread of its own, waiting on [`Net::runtime`] there, and
    /// wait for its output: for work that borrows something not `Send` (a sign-in's browser
    /// callback) across a wait, so cannot be a task. `None` when the thread could not start or
    /// ended without an answer.
    pub async fn on_thread<T, F>(
        &self,
        name: &str,
        work: impl FnOnce() -> F + Send + 'static,
    ) -> Option<T>
    where
        F: Future<Output = T>,
        T: Send + 'static,
    {
        let runtime = self.runtime();
        let (done, answer) = tokio::sync::oneshot::channel();
        std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                // The future that asked may have been dropped; then nobody is left to tell.
                let _ = done.send(runtime.block_on(work()));
            })
            .inspect_err(|why| log::warn!("the thread {name} could not be started: {why}"))
            // A thread that could not start is an answer that never comes, and said above.
            .ok()?;
        // A sender dropped without a send is a thread that panicked.
        answer.await.ok()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_never_told_how_to_reach_servers_says_so_once_and_still_answers() {
        let store = Arc::new(mail_store::testing::in_memory());
        let client = Client::inline(store, Arc::new(mail_core::mail::SystemClock));
        let _ = client.net().secrets();
        let _ = client.net().secrets();
        assert!(UNREACHED_SAID.is_completed());
    }
}

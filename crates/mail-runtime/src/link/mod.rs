//! The link to the desktop's accountd (porter, step E6): one decision at start, then the same
//! four questions asked of whoever answers them.
//!
//! mailo's accounts are of one of two kinds. Most are mailo's own: the secrets are in the
//! platform keyring, and the engines connect to the servers themselves, which is the only thing a
//! build for another desktop has. When our desktop's accountd is there, an account can instead be
//! accountd's and mailo holds a grant on it ([`mail_domain::AuthPlan::Granted`]): its password or
//! token never reaches mailo, an IMAP, SMTP, POP3 or ManageSieve engine is handed a stream the
//! daemon already authenticated (`Accounts::open_authenticated`), and the engines that speak HTTP
//! with a bearer (JMAP, Graph) ask for a short-lived token (`Accounts::token`).
//!
//! **The choice** is [`start`]: one function, called once at the start of the process, which asks
//! nothing of the operating system. It is told whether accountd is here (the desktop capability
//! probe, `ds_desktop`'s `Capability::Accounts`: the bus name has an owner or the bus can start
//! it), and links to it when it is and when the build has the `quire-desktop` feature; every other
//! case is [`Link::Local`] and the engines go on as they always have. A build without the feature
//! has no D-Bus transport in it to choose. The caller says which was chosen ([`Link::name`]).
//!
//! **What stays in process.** porter-client's `InProcess` transport hosts the account service in
//! this process, and mailo does that for the Add Account window (`mail-app`'s `add_account::host`:
//! the sheet's machine over mailo's own sign-in). It does not host mailo's accounts: those live in
//! mailo's store and keyring, and a service whose registry held them would be a second copy. So the
//! in-process path of the engines is the direct one, and says so ([`Link::Local`]).
//!
//! The link is a trait object ([`Accountd`]) so the engines, the sync and the window ask it
//! without naming a transport, and a test stands in for the daemon.

mod client;
#[cfg(all(feature = "quire-desktop", target_os = "linux"))]
pub mod dbus;
mod tokens;

pub use client::{Client, Listed};
pub use tokens::LinkedTokens;

use crate::AccountSecrets;
use porter_core::SecretKey;

use crate::{RuntimeError, Transport};
use mail_domain::Retry;
use porter_core::wire::Refusal;
use porter_core::{
    AccountId, Audience, Candidate, GrantId, IssuedToken, ServiceEndpoint, UnixSeconds,
};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// A boxed answer, so [`Accountd`] can be a trait object.
pub type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, LinkError>> + Send + 'a>>;

/// Why accountd did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// Nobody answered: not running, or the bus went away.
    #[error("the desktop's account service did not answer")]
    Unreachable,
    /// accountd answered no.
    #[error("the desktop's account service refused: {}", words(.0))]
    Refused(Refusal),
    /// Anything else (a reply that is not porter's, a daemon of another version).
    #[error("the desktop's account service: {0}")]
    Other(String),
}

fn words(refusal: &Refusal) -> &'static str {
    match refusal {
        Refusal::Dismissed => "the sheet was closed",
        Refusal::Denied => "the person said no",
        Refusal::NoFittingAccount => "no account fits",
        Refusal::UnknownGrant => "Mail has no grant on that account (any more)",
        Refusal::AudienceNotGranted => "the grant does not cover that service",
        Refusal::NeedsReauth => "the account needs signing in again",
        Refusal::Unavailable => "the account or its secret store cannot be reached",
        Refusal::EndpointNotGranted => "that server is not one of the account's",
    }
}

impl LinkError {
    /// What to do about it: the same words as every other failure's.
    pub fn retry(&self) -> Retry {
        match self {
            // The daemon is not there now; it may be, and a pass tries again on its schedule.
            LinkError::Unreachable | LinkError::Refused(Refusal::Unavailable) => {
                Retry::After(Duration::from_secs(30))
            }
            // What is wanted of the person: sign in again, or allow Mail to use the account.
            LinkError::Refused(
                Refusal::NeedsReauth
                | Refusal::UnknownGrant
                | Refusal::AudienceNotGranted
                | Refusal::Denied
                | Refusal::Dismissed
                | Refusal::NoFittingAccount,
            ) => Retry::NeedsReauth,
            LinkError::Refused(Refusal::EndpointNotGranted) | LinkError::Other(_) => {
                Retry::Fatal(self.to_string())
            }
        }
    }

    /// Whether the person has something to do (sign in again, allow Mail again) rather than wait.
    pub fn needs_person(&self) -> bool {
        matches!(self.retry(), Retry::NeedsReauth)
    }
}

impl From<LinkError> for RuntimeError {
    fn from(error: LinkError) -> Self {
        RuntimeError::Link {
            retry: error.retry(),
            why: error.to_string(),
        }
    }
}

/// What changed at accountd that a mailo that has read its accounts should read again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// An account was added (and Mail holds a grant on it: accountd tells holders only).
    Added,
    /// An account was removed. The name is the object path's last segment
    /// (`porter_core::object_segment` of its id), which is all the signal carries.
    Removed(String),
    /// A grant of Mail's was given, changed or revoked.
    Granted,
}

/// A feed of [`Change`]s, until the link goes.
pub trait Changes: Send {
    /// The next change, or `None` when no more will come.
    fn next(&mut self) -> Pin<Box<dyn Future<Output = Option<Change>> + Send + '_>>;
}

/// What mailo asks of accountd. [`Client`] answers over a porter-client transport; a test answers
/// from a table.
pub trait Accountd: Send + Sync + fmt::Debug {
    /// The accounts Mail holds a grant on that can carry its mail: what to read accounts from.
    fn candidates(&self) -> Answer<'_, Vec<Candidate>>;

    /// A short-lived bearer for `audience`, on the grant (`Accounts::token`).
    fn token<'a>(&'a self, grant: &'a GrantId, audience: &'a Audience) -> Answer<'a, IssuedToken>;

    /// A connection to one of the account's servers that the daemon has already signed in to
    /// (`Accounts::open_authenticated`).
    fn open<'a>(
        &'a self,
        grant: &'a GrantId,
        endpoint: &'a ServiceEndpoint,
    ) -> Answer<'a, Transport>;

    /// Opens the add-account sheet, drawn by the desktop's own shell (mailo opens no window), and
    /// answers with the account added.
    fn add_account(&self) -> Answer<'_, AccountId>;

    /// Opens the sheet that signs `account` in again.
    fn reauthenticate<'a>(&'a self, account: &'a AccountId) -> Answer<'a, ()>;

    /// Asks the person to allow Mail to use one of their accounts (the chooser and the consent
    /// sheet), and answers with the one they picked, granted.
    fn request_grant(&self) -> Answer<'_, Candidate>;

    /// Withdraws Mail's grant: Mail stops using the account, which stays accountd's.
    fn revoke<'a>(&'a self, grant: &'a GrantId) -> Answer<'a, ()>;

    /// The changes to follow, if this link has any: `AccountAdded`, `AccountRemoved` and
    /// `GrantChanged`.
    fn changes(&self) -> Answer<'_, Option<Box<dyn Changes>>>;
}

/// How this process reaches accounts: decided once, by [`start`].
#[derive(Debug, Clone)]
pub enum Link {
    /// Mailo's own: its store, its keyring, its sockets. Everything a build for another desktop
    /// does, and what ours does when accountd is not there.
    Local,
    /// The desktop's accountd.
    Accountd(Arc<dyn Accountd>),
}

impl Link {
    /// The link to accountd, if that is what was chosen.
    pub fn accountd(&self) -> Option<&Arc<dyn Accountd>> {
        match self {
            Link::Local => None,
            Link::Accountd(link) => Some(link),
        }
    }

    /// Whether accounts can be accountd's.
    pub fn is_linked(&self) -> bool {
        matches!(self, Link::Accountd(_))
    }

    /// Which was chosen, in a word for the log.
    pub fn name(&self) -> &'static str {
        match self {
            Link::Local => "in process",
            Link::Accountd(_) => "accountd over D-Bus",
        }
    }
}

/// Whether the desktop's account service is here, as the capability probe found it
/// (`ds_desktop::Presence`, which this crate does not name: it has no window and no quire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Here {
    /// accountd owns its bus name, or the bus would start it.
    Yes,
    /// Nothing answers.
    No,
}

/// The one decision: link to accountd when it is here and this build can, else stay in process.
///
/// `usage` is how this process uses its grants: `Interactive` for the window and the commands
/// someone is waiting on, `Background` for `mailo watch`.
pub async fn start(accountd: Here, usage: porter_core::consent::Usage) -> Link {
    let connect = connect_session(usage);
    start_with(accountd, connect).await
}

/// [`start`] over a connection a test supplies. The probe's answer is still the first word: a
/// connection is not attempted when accountd is not here.
pub async fn start_with<F>(accountd: Here, connect: F) -> Link
where
    F: Future<Output = Option<Arc<dyn Accountd>>>,
{
    match accountd {
        Here::No => Link::Local,
        Here::Yes => match connect.await {
            Some(link) => Link::Accountd(link),
            // The probe said it is here (activatable, say) and it could not be reached: the
            // process goes on in its own way rather than without accounts.
            None => Link::Local,
        },
    }
}

#[cfg(all(feature = "quire-desktop", target_os = "linux"))]
async fn connect_session(usage: porter_core::consent::Usage) -> Option<Arc<dyn Accountd>> {
    // On the long-lived runtime: zbus's connection keeps its tasks on the runtime it was made
    // under, and the runtimes of a sync or a command are short.
    let work = async move { dbus::session(usage).await };
    crate::account_secrets::long_lived()
        .spawn(work)
        .await
        .ok()?
}

#[cfg(not(all(feature = "quire-desktop", target_os = "linux")))]
async fn connect_session(_usage: porter_core::consent::Usage) -> Option<Arc<dyn Accountd>> {
    None
}

/// The link of this process, once [`install`]ed: `platform_secrets()` hands it to every engine
/// (it is where an account's credentials come from, and for an accountd account that is here).
static CURRENT: OnceLock<Link> = OnceLock::new();

/// Records the process's link. Only the first call counts; a second is ignored, like
/// `OnceLock::set`.
pub fn install(link: Link) {
    let _ = CURRENT.set(link);
}

/// The process's link: [`Link::Local`] until [`install`] says otherwise.
pub fn current() -> Link {
    CURRENT.get().cloned().unwrap_or(Link::Local)
}

/// When a token must be asked for again: this far ahead of its expiry, so an operation that
/// starts now does not present one that dies on the wire.
pub(crate) const REFRESH_MARGIN: i64 = 60;

/// Whether `token` is good for an operation that starts at `now`.
pub(crate) fn fresh(token: &IssuedToken, now: UnixSeconds) -> bool {
    token.expires.0 - now.0 > REFRESH_MARGIN
}

/// The account secrets of a process linked to accountd: none. accountd holds every account's
/// credentials ([`link`] is where an account that is accountd's gets them), so this store keeps
/// nothing, gives nothing and takes nothing, and in particular never reaches mailo's own keyring
/// items: those belong to a start that is not linked, and stay exactly as they are.
///
/// [`link`]: AccountSecrets::link
pub struct LinkedSecrets {
    link: Arc<dyn Accountd>,
}

// By hand: a link is a trait object with no `Debug` of its own to rely on.
impl fmt::Debug for LinkedSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkedSecrets")
            .field("link", &self.link)
            .finish_non_exhaustive()
    }
}

impl LinkedSecrets {
    /// The secrets of a process linked to `link`.
    pub fn new(link: Arc<dyn Accountd>) -> Self {
        Self { link }
    }
}

/// What a linked process says to a secret it is asked to read or write.
fn not_here() -> RuntimeError {
    RuntimeError::Secrets(
        "linked to the desktop's accounts: Mail keeps no secret of its own".to_owned(),
    )
}

impl AccountSecrets for LinkedSecrets {
    fn get<'a>(
        &'a self,
        _key: &'a SecretKey,
    ) -> Pin<Box<dyn Future<Output = Result<porter_core::Credential, RuntimeError>> + Send + 'a>>
    {
        Box::pin(async { Err(not_here()) })
    }

    fn put<'a>(
        &'a self,
        _key: &'a SecretKey,
        _value: &'a porter_core::Credential,
    ) -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>> {
        Box::pin(async { Err(not_here()) })
    }

    // Nothing is held, so nothing is forgotten: and an item of mailo's own is never deleted from
    // here.
    fn forget<'a>(
        &'a self,
        _key: &'a SecretKey,
    ) -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn forget_account<'a>(
        &'a self,
        _account: &'a AccountId,
    ) -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn link(&self) -> Option<Arc<dyn Accountd>> {
        Some(self.link.clone())
    }
}

#[cfg(test)]
mod tests;

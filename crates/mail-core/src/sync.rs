//! One sync pass, from stored configuration to a real connection.
//!
//! This is where everything meets: the preset that configured the account, the credential in the
//! keyring, the backend that speaks the protocol, and the store that keeps the result. It is
//! also where the application's async work starts: every entry point is `async`, awaited on the
//! runtime the front-end owns.

pub use crate::notify::Announce;
pub mod due;
pub mod fresh;
pub mod graph_push;
pub mod live;
pub mod report;
pub mod watcher;

mod body;
pub use body::fetch_body_with;
mod jmap;
mod quota;
mod search;

use crate::error::{CoreError, Logged};
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend, Pop3Backend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, Pop3Command, Pop3Session};
use mail_runtime::graph::read::{OverHttp, Reader};
use mail_runtime::tokens::{self, Now};
use mail_runtime::{
    AccountEngine, AccountSecrets, ClientRegistry, Held, OAuthTokens, SyncReport, TokenSource,
    clients,
};
use mail_store::SqliteStore;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
pub use quota::has_quota;
use report::{Done, Emit, Failure, Hooks, PassEnd, Progress};
pub use search::search_server_with;
use std::sync::Arc;
use tokio::sync::watch;

/// Everything stored about one account.
#[derive(Debug)]
pub struct Configured {
    pub id: AccountId,
    pub address: String,
    pub plan: AccountPlan,
    pub caps: AccountCaps,
    /// Whether a pass fetches the attachments a sync leaves on the server too. The window's
    /// setting, not the store's ([`crate::offline`]): [`Keep::Bodies`] until something reads it.
    ///
    /// [`Keep::Bodies`]: crate::offline::Keep::Bodies
    pub keep: crate::offline::Keep,
    /// How often a timed pass checks it, from the same setting: [`Every::Automatic`] until
    /// something reads it.
    ///
    /// [`Every::Automatic`]: crate::offline::Every::Automatic
    pub every: crate::offline::Every,
    /// How far back a pass downloads its bodies, from the same setting.
    pub window: crate::offline::Window,
    /// When a POP3 pass removes the server's copies, from the same setting.
    pub remove: crate::offline::Remove,
    /// How the junk filter judges what a pass brings ([`crate::junk`]): the window's setting,
    /// as `keep` is.
    pub junk: crate::junk::Policy,
}

/// What the server was last observed to support, for one account.
///
/// `None` when nothing has connected yet, which the caller must treat as "assume nothing" rather
/// than as an error: the window can be opened before the first sync, and refusing to act at all
/// would be worse than acting locally.
pub fn caps_of(store: &SqliteStore, account: AccountId) -> Option<AccountCaps> {
    store
        .account_caps(account)
        .or_log("the account's capabilities could not be read")
        .flatten()
}

/// Whether the person paused `account`'s syncing in Settings. A paused account is in every list
/// of accounts but none of the lists of what to sync: no timed pass, no pass asked for, no
/// connection held open for it. One whose preferences cannot be read syncs, as it did before
/// there were any.
pub fn paused(store: &SqliteStore, account: AccountId) -> bool {
    store
        .account_prefs(account)
        .or_log("the account's preferences could not be read")
        .is_some_and(|prefs| prefs.syncing == mail_store::Syncing::Paused)
}

/// Read the accounts back out of the store.
pub(crate) fn configured(store: &SqliteStore) -> Result<Vec<Configured>, CoreError> {
    let mut out = Vec::new();
    for stored in store.list_accounts()? {
        let address = stored.address;
        let plan: AccountPlan = stored.plan.map_err(|e| CoreError::PlanUnreadable {
            address: address.clone(),
            why: why(&e),
        })?;
        let caps: AccountCaps = match stored.caps {
            Some(caps) => caps.map_err(|e| CoreError::CapsUnreadable {
                address: address.clone(),
                why: why(&e),
            })?,
            // No capabilities yet means nothing has connected. The expected ones from the
            // preset are a starting point, not a claim about the server.
            None => return Err(CoreError::NoCaps { address }),
        };
        out.push(Configured {
            id: stored.id,
            address,
            plan,
            caps,
            keep: crate::offline::Keep::default(),
            every: crate::offline::Every::default(),
            window: crate::offline::Window::default(),
            remove: crate::offline::Remove::default(),
            junk: crate::junk::Policy::default(),
        });
    }
    Ok(out)
}

/// A pass's classified failure as the error of a call that has no use for the classification.
fn pass_error(failure: Failure) -> CoreError {
    CoreError::PassFailed {
        retry: failure.retry,
        why: failure.why,
    }
}

/// Why a stored value would not read, without the store's own preamble.
pub(crate) fn why(error: &mail_store::StoreError) -> String {
    match error {
        mail_store::StoreError::Decode { why, .. } => why.clone(),
        other => other.to_string(),
    }
}

/// How each configured account signs in, by address.
///
/// Beside `mailboxes_by_account` and for the same reason: `account list` should be able to say
/// what an account is waiting for without deciding it for itself. Two surfaces answering that
/// question separately is how `sync` came to tell a Google account to find a password.
pub fn auth_by_account(
    store: &SqliteStore,
) -> Result<std::collections::HashMap<String, AuthPlan>, CoreError> {
    Ok(configured(store)?
        .into_iter()
        .map(|account| (account.address, account.plan.auth))
        .collect())
}

/// Every configured account's id and address, in the order they were added.
pub fn addresses(store: &SqliteStore) -> Vec<(AccountId, String)> {
    configured(store)
        .or_log_default("the accounts could not be read")
        .into_iter()
        .map(|account| (account.id, account.address))
        .collect()
}

/// The remedy for an account that must sign in, again or for the first time: a pass that ended in
/// [`Retry::NeedsReauth`] says so, and this says how that account signs in. An address the store
/// does not know, or a plan it cannot read, is asked to sign in without saying how.
pub fn sign_in_remedy(store: &SqliteStore, address: &str) -> crate::Remedy {
    let with = configured(store)
        .or_log("the accounts could not be read to say how to sign in")
        .and_then(|all| all.into_iter().find(|account| account.address == address))
        .map_or(crate::SignInWith::Unknown, |account| {
            crate::SignInWith::of(&account.plan.auth)
        });
    crate::Remedy::SignIn {
        address: address.to_owned(),
        with,
    }
}

/// The accounts that keep their mail on this computer ([`Incoming::Local`]).
pub fn local_accounts(store: &SqliteStore) -> Vec<AccountId> {
    configured(store)
        .or_log_default("the accounts could not be read")
        .into_iter()
        .filter(|account| matches!(account.plan.incoming, Incoming::Local))
        .map(|account| account.id)
        .collect()
}

/// How often a background sync should run, from the accounts' own capabilities.
///
/// The shortest interval any account asks for, so an account that wants IDLE-like freshness is
/// not held to another's hourly poll. Five minutes when nothing says otherwise, which is what
/// every preset in the table already carries.
pub fn poll_interval(store: &SqliteStore) -> std::time::Duration {
    let default = std::time::Duration::from_secs(300);
    configured(store)
        .or_log_default("the accounts could not be read")
        .into_iter()
        // IDLE is a long-lived connection this loop does not hold. Until it does, an account that
        // supports it is polled like any other, at `due::IDLE`.
        .filter(|account| !paused(store, account.id.clone()))
        .map(|a| due::every(&a.caps.watch))
        .min()
        .unwrap_or(default)
}

impl crate::mail::SyncOps<'_> {
    /// Sync every configured account that has a credential, how each ended kept as data.
    ///
    /// Accounts without one end as [`PassEnd::Failed`] rather than failing the run: having one
    /// account that needs attention should not stop the others from fetching mail. An account with
    /// nothing to sync has no entry, so a run over no accounts is an empty list, and what to say of
    /// that is for the caller.
    pub async fn run(&self, hooks: Hooks<'_>) -> Result<Vec<PassEnd>, CoreError> {
        let mail = self.0;
        // The registry is read once per run rather than once per account: it is deployment
        // configuration, and an edit halfway through a run producing two different client ids is
        // not a behaviour worth having.
        let registry = mail.clients()?;
        run_all(
            mail.store().clone(),
            mail.secrets(),
            &registry,
            mail.now(),
            Mode::Once,
            &Scope {
                due: &|_| true,
                kept: &crate::offline::load_default(),
                junk: mail.junk(),
            },
            hooks,
        )
        .await
    }

    /// Download one attachment a sync left on the server, and record it as held.
    pub async fn fetch_part(
        &self,
        message: mail_domain::MessageId,
        section: &mail_domain::Section,
    ) -> Result<(), CoreError> {
        let mail = self.0;
        let registry = mail.clients()?;
        fetch_part_with(
            mail.store(),
            mail.secrets(),
            &registry,
            message,
            section,
            mail.now(),
        )
        .await
    }

    /// Run one IMAP account's outbox now, and nothing else: no fetch, no flag sweep.
    ///
    /// What `mailo import --to-mailbox` runs after queueing its uploads, so they go while the user
    /// is watching rather than at the next sync.
    pub async fn drain(&self, account: AccountId) -> Result<SyncReport, CoreError> {
        let mail = self.0;
        let registry = mail.clients()?;
        drain_with(mail.store(), mail.secrets(), &registry, account, mail.now()).await
    }

    /// Fetch one folder now.
    ///
    /// For a folder the window opens that no pass fetches — one the user does not follow — so its
    /// mail is there to list without waiting for the next pass. A followed folder is fetched by
    /// every pass anyway, and asking again here is harmless: what is already held is not fetched
    /// twice. List it afterwards with `Filter::InFolder`.
    ///
    /// How the fetch ended is the return, as one account's pass does ([`PassEnd`]); it is never
    /// `Cancelled`, because nothing holds a signal to cancel it with. An `Err` is a request there
    /// is nothing to fetch for: POP3, which has one mailbox and fetches it every pass, and an
    /// account whose folders are labels, where a folder's mail is the mail with that label and
    /// arrives with it.
    pub async fn folder_now(&self, account: AccountId, path: &str) -> Result<PassEnd, CoreError> {
        let mail = self.0;
        let registry = mail.clients()?;
        folder_now_with(
            mail.store().clone(),
            mail.secrets(),
            &registry,
            account,
            path,
            mail.now(),
        )
        .await
    }
}

/// [`SyncOps::run`](crate::SyncOps), with the secret store named.
///
/// The keyring is the only thing in this function that cannot exist in a test, and it was
/// reached for directly — so the one function that ties configuration, credentials, the backend,
/// the engine and the store together was the one function nothing could run. Every piece below
/// it had tests; the assembly had none.
///
/// Injected rather than faked at a lower level, because what is worth testing here *is* the
/// assembly: which credential is fetched for which purpose, what happens to an account that has
/// none, and that a failure in one account does not stop the next.
pub async fn run_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, CoreError> {
    run_all(
        store,
        secrets,
        registry,
        now,
        Mode::Once,
        &Scope {
            due: &|_| true,
            kept: &crate::offline::Kept::default(),
            junk: crate::junk::Policy::default(),
        },
        hooks,
    )
    .await
}

/// Which accounts a run syncs, and how much of each it keeps.
struct Scope<'a> {
    /// Whether an account is due a pass ([`due`]).
    due: &'a dyn Fn(AccountId) -> bool,
    /// How each account is synced ([`crate::offline`]).
    kept: &'a crate::offline::Kept,
    /// How the junk filter judges new mail ([`crate::junk`]).
    junk: crate::junk::Policy,
}

/// A run over the accounts in `scope`, told whether to stop after one pass.
#[allow(clippy::too_many_arguments)]
async fn run_all(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    scope: &Scope<'_>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, CoreError> {
    let accounts = pick(&store, scope)?;
    pass_over(&store, secrets, registry, now, mode, accounts, hooks).await
}

/// The accounts a run is to sync.
fn pick(store: &SqliteStore, scope: &Scope<'_>) -> Result<Vec<Configured>, CoreError> {
    // An account that keeps its mail here has no server: nothing to fetch, nothing to drain,
    // and no credential to ask the keyring for. Left out rather than reported, because a line
    // saying so on every pass would be noise about something that is working as intended.
    Ok(configured(store)?
        .into_iter()
        .filter(|account| !matches!(account.plan.incoming, Incoming::Local))
        .filter(|account| !paused(store, account.id.clone()))
        .filter(|a| (scope.due)(a.id.clone()))
        .map(|a| Configured {
            junk: scope.junk,
            ..kept_as(a, scope.kept)
        })
        .collect())
}

/// `account`, synced as `kept` says.
pub(crate) fn kept_as(account: Configured, kept: &crate::offline::Kept) -> Configured {
    Configured {
        keep: kept.of(account.id.clone()),
        every: kept.every_of(&account.id),
        window: kept.window_of(&account.id),
        remove: kept.remove_of(&account.id),
        ..account
    }
}

/// One pass over `accounts`, concurrently, each ended as a [`PassEnd`].
#[allow(clippy::too_many_arguments)]
async fn pass_over(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    accounts: Vec<Configured>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, CoreError> {
    // All the accounts at once.
    //
    // Accounts are independent by construction: `AccountId` partitions every table, and two
    // accounts are two conversations with two different servers. Run one after another, a pass
    // spends the *sum* of their waiting, and almost all of a pass is waiting — so a slow Gmail
    // backfill would hold up another account's poll that had nothing to do with it.
    //
    // Concurrent rather than parallel, and deliberately: these futures are joined, not spawned,
    // so they share one thread and interleave at their `await` points. What overlaps is the
    // network, which is where the time goes. Spawning would need `'static` and would buy only
    // the few milliseconds of SQLite in between — and the store is one connection behind a
    // mutex, so those milliseconds cannot overlap anyway.
    //
    // Nothing here needs a rate limiter. Throttling is a within-account question — one server,
    // several connections — and this is one connection each to servers that have never heard of
    // each other.
    //
    // Ended in the order the accounts are configured, which `join_all` preserves. A pass that
    // listed its accounts in whatever order they happened to finish would read differently
    // between runs for no reason the user could see.
    Ok(
        futures_util::future::join_all(accounts.iter().map(|account| async {
            let cancel = hooks.cancel.get(&account.id).cloned();
            let emit = |progress| {
                if let Some(sink) = hooks.progress {
                    sink(account.id.clone(), progress);
                }
            };
            let cancelled = cancel.clone();
            let result = one(
                store,
                account,
                secrets.clone(),
                registry,
                now,
                mode,
                cancel,
                Some(&emit),
            )
            .await;
            let end = ended(account, result, cancelled);
            remember(store, &end, now);
            end
        }))
        .await,
    )
}

/// Keep how `end` went, so a window opened later can tell an account that is quiet from one
/// that has not been fetched in days ([`fresh`]). `at` is when the pass began: its mail is as
/// current as that. A cancelled pass says nothing either way and is not kept.
fn remember(store: &SqliteStore, end: &PassEnd, at: chrono::DateTime<chrono::Utc>) {
    let kept = match end {
        PassEnd::Finished(report) => store.pass_finished(report.account.clone(), at),
        PassEnd::Failed { account, why, .. } => store.pass_failed(account.clone(), at, why),
        PassEnd::Cancelled { .. } => return,
    };
    // Logged and no more: the pass has ended either way, and only the record of it is lost.
    let _ = kept.or_log("how a pass ended could not be kept");
}

/// How an account's pass ended, given what it returned and the signal it was given.
fn ended(
    account: &Configured,
    result: Result<Done, Failure>,
    cancel: Option<watch::Receiver<bool>>,
) -> PassEnd {
    match result {
        Ok(done) => PassEnd::Finished(done.of(account)),
        Err(Failure {
            retry: Retry::Fatal(_),
            ..
        }) if cancel.is_some_and(|signal| *signal.borrow()) => PassEnd::Cancelled {
            account: account.id.clone(),
            address: account.address.clone(),
        },
        Err(Failure {
            retry,
            why,
            pause,
            refused,
        }) => PassEnd::Failed {
            account: account.id.clone(),
            address: account.address.clone(),
            retry,
            why,
            pause,
            // Refused the protocol, an account set up another way may be served.
            offer: refused.and_then(|_| crate::account::graph_offer(&account.plan)),
        },
    }
}

/// The credential to authenticate with, renewed first if it is an OAuth one that has expired.
///
/// An access token lasts about an hour. Nothing did this: the stored value went straight to the
/// backend, so an OAuth account fetched mail until its first token expired and then failed on
/// every pass afterwards with an authentication error — while the refresh token that would have
/// fixed it sat unused in the same keyring entry.
pub(crate) async fn signed_in(
    account: &Configured,
    credential: Credential,
    secrets: &dyn AccountSecrets,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, CoreError> {
    signed_in_typed(account, credential, secrets, registry, now)
        .await
        .map_err(pass_error)
}

/// [`signed_in`], with what to do about a failure kept beside what to say about it.
async fn signed_in_typed(
    account: &Configured,
    credential: Credential,
    secrets: &dyn AccountSecrets,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, Failure> {
    let AuthPlan::OAuth { issuer, scopes } = &account.plan.auth else {
        return Ok(credential);
    };
    // Checked before the client is built and before anything is sent: a password account and a
    // still-valid token both leave here without touching the network.
    if tokens::is_fresh(&credential, now) {
        return Ok(credential);
    }
    let Some(client) = clients::client(registry, *issuer) else {
        // The account was added with a client id that this installation no longer has, so the
        // token cannot be renewed and saying "authentication failed" would point at the wrong
        // thing entirely.
        //
        // The remedy is the user signing in again, so it is classified as one.
        return Err(Failure::reauth(crate::account::no_client_id(*issuer)));
    };
    tokens::renew(account.id.clone(), client, scopes, credential, secrets, now)
        .await
        .map_err(|e| Failure::of("cannot renew the sign-in: ", &e))
}

/// The credential stored for `account`'s sign-in: what the keyring holds for it.
pub(crate) async fn stored_credential(
    account: &Configured,
    secrets: &dyn AccountSecrets,
) -> Result<Credential, Failure> {
    secrets
        .get(&SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::IncomingPassword,
        })
        .await
        // A credential that is not there is one to be asked for again; a keyring that did not
        // answer is waited for, and says so.
        .map_err(|error| match mail_runtime::SecretTrouble::of(&error) {
            mail_runtime::SecretTrouble::Absent => {
                Failure::reauth(crate::account::no_credential(&account.plan.auth))
            }
            trouble @ mail_runtime::SecretTrouble::Unreachable(why) => Failure {
                retry: trouble.retry(),
                why: why.to_owned(),
                pause: crate::fetch::Pause::ServerBusy,
                refused: None,
            },
        })
}

/// What keeps an OAuth account's tokens fresh for as long as its engine runs.
///
/// `None` for a password account and for one this installation has no client id for: the first
/// has nothing to renew, and [`signed_in`] has already said what is wrong with the second.
///
/// `clock` is where it reads the time. A watch reads the wall clock, because it outlives any
/// `now` it could be handed; a single pass uses the instant it was given, like everything else
/// in it.
fn renewal_for(
    account: &Configured,
    held: &Held,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    clock: Now,
) -> Option<Arc<dyn TokenSource>> {
    let AuthPlan::OAuth { issuer, .. } = &account.plan.auth else {
        return None;
    };
    let client = clients::client(registry, *issuer)?.clone();
    let source = OAuthTokens::new(
        account.id.clone(),
        &account.plan,
        client,
        secrets,
        held.clone(),
    )
    .with_clock(clock);
    Some(Arc::new(source))
}

/// The clock a renewal reads in `mode`. See [`renewal_for`].
fn clock_for(mode: Mode, now: chrono::DateTime<chrono::Utc>) -> Now {
    match mode {
        Mode::Once => Arc::new(move || now),
        Mode::Watch => Arc::new(chrono::Utc::now),
    }
}

#[allow(clippy::too_many_arguments)]
async fn one(
    store: &Arc<SqliteStore>,
    account: &Configured,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    cancel: Option<watch::Receiver<bool>>,
    emit: Emit<'_>,
) -> Result<Done, Failure> {
    say(emit, Progress::Connecting);
    let stored = stored_credential(account, secrets.as_ref()).await?;
    let credential = signed_in_typed(account, stored, secrets.as_ref(), registry, now).await?;
    // Not fatal to the pass: an account that cannot send can still receive. And a token for
    // sending alone is sending's (`mail_runtime::sending`): what it could not have is said on
    // the outbox, when there is something to send, and never asks this account to sign in or
    // wait. The one an account reading through Graph receives with is this pass's to report.
    let sending = sending_token_typed(account, secrets.as_ref(), registry, now)
        .await
        .err()
        .filter(|_| !mail_runtime::sending::sending_alone(&account.plan));
    // The credential goes into a cell rather than into the session factory, so a token renewed
    // an hour into a watch reaches the next connection.
    let held = Held::new(credential);
    let renewal = renewal_for(
        account,
        &held,
        secrets.clone(),
        registry,
        clock_for(mode, now),
    );

    let folders = {
        use mail_store::Store as _;
        store
            .folders(account.id.clone())
            .map_err(|e| Failure::of("", &e))?
    };
    let mailboxes = to_sync(account, &folders);
    // Nothing cancels a one-shot CLI sync, but the loop requires a receiver. The window hands in
    // the real one, which is what lets the same engine serve it unchanged.
    let (_tx, quiet) = watch::channel(false);
    let mut cancel = cancel.unwrap_or(quiet);

    let report = match &account.plan.incoming {
        // `pick` leaves these out before asking for a credential; nothing to do if one
        // arrives here anyway.
        Incoming::Local => Ok(Done::default()),
        Incoming::Pop3 { .. } => {
            let mut engine = pop3_engine(store, account, &held, secrets)?
                .downloading_since(account.window.since(now));
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            let passed = pass(&mut engine, &mailboxes, account, &mut cancel, now, emit).await;
            match passed {
                Ok(mut done) => {
                    let removing = Removing {
                        store,
                        account,
                        now,
                    };
                    remove_copies(&mut engine, &removing, &mut cancel, &mut done).await;
                    Ok(done)
                }
                failed => failed,
            }
        }
        Incoming::Imap { .. } => {
            let mut engine = imap_engine(store, account, held, secrets)
                .downloading_since(account.window.since(now));
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            pass(&mut engine, &mailboxes, account, &mut cancel, now, emit).await
        }
        Incoming::Graph => {
            let mut engine = graph_engine(store, account, secrets)
                .map_err(|e| Failure::fatal(e.to_string()))?
                .downloading_since(account.window.since(now));
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            pass(&mut engine, &mailboxes, account, &mut cancel, now, emit).await
        }
        // HTTPS rather than a socket, so an engine of its own; the same pass, the same wait.
        Incoming::Jmap { .. } => {
            let mut engine = mail_runtime::JmapEngine::new(
                account.id.clone(),
                account.plan.clone(),
                store.clone(),
                secrets,
            )
            .map_err(|e| Failure::of("", &e))?;
            jmap::drive(&mut engine, account, &mut cancel, now, emit).await
        }
    };
    report.map(|mut done| {
        if let Some(failure) = sending {
            done.trouble(None, failure.retry, failure.why);
        }
        done
    })
}

/// Most server copies one POP3 pass removes. The rest wait for the next pass.
const REMOVED_PER_PASS: u32 = 100;

/// Whose copies [`remove_copies`] removes, from what store, and when.
#[derive(Clone, Copy)]
struct Removing<'a> {
    store: &'a SqliteStore,
    account: &'a Configured,
    now: chrono::DateTime<chrono::Utc>,
}

/// After a POP3 pass, remove the server's copies the account no longer wants there
/// ([`crate::offline::Remove`]): those held here in full and old enough, or those deleted here.
/// A failure is the pass's to say, beside what it fetched; the copies stay on the server, and the
/// next pass asks again.
async fn remove_copies(
    engine: &mut AccountEngine<Pop3Backend>,
    removing: &Removing<'_>,
    cancel: &mut mail_runtime::Cancel,
    done: &mut Done,
) {
    use crate::offline::Remove;
    let Removing {
        store,
        account,
        now,
    } = *removing;
    let inbox = MailboxRef {
        account: account.id.clone(),
        path: "INBOX".to_owned(),
    };
    let chosen = match account.remove {
        Remove::Never => return,
        Remove::AfterDays(days) => store.copies_held_before(
            &inbox,
            now - chrono::TimeDelta::days(i64::from(days.max(1))),
            REMOVED_PER_PASS,
        ),
        Remove::WhenDeleted => store.copies_deleted_here(&inbox, REMOVED_PER_PASS),
    };
    let removed = match chosen {
        Ok(remotes) => engine.remove_copies(remotes, cancel).await,
        Err(e) => Err(e.into()),
    };
    if let Err(e) = removed {
        done.trouble(
            Some("INBOX"),
            e.retry(),
            format!("removing mail from the server: {e}"),
        );
    }
}

/// Tell the sink, if there is one, where the pass has got to.
fn say(emit: Emit<'_>, progress: Progress) {
    if let Some(sink) = emit {
        sink(progress);
    }
}

/// For an account that sends or reads through Graph, a Graph token valid for this pass.
///
/// Its own token beside the IMAP one: Microsoft issues each access token for one resource. An
/// account that reads through Graph has no IMAP one, and reads with this.
async fn sending_token(
    account: &Configured,
    secrets: &dyn AccountSecrets,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    sending_token_typed(account, secrets, registry, now)
        .await
        .map_err(pass_error)
}

/// [`sending_token`], with what to do about a failure kept beside what to say about it.
async fn sending_token_typed(
    account: &Configured,
    secrets: &dyn AccountSecrets,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), Failure> {
    let AuthPlan::OAuth { issuer, .. } = &account.plan.auth else {
        return Ok(());
    };
    if account.plan.outgoing != Outgoing::Graph && account.plan.incoming != Incoming::Graph {
        return Ok(());
    }
    let client = clients::client(registry, *issuer).ok_or_else(|| {
        Failure::fatal("no OAuth client id is configured, so nothing can be sent")
    })?;
    let reach = tokens::GraphReach::of(&account.plan);
    tokens::graph_token(account.id.clone(), client, reach, secrets, now)
        .await
        .map(|_| ())
        .map_err(|e| Failure::of("cannot sign in to Microsoft Graph for sending: ", &e))
}

/// Sign in to `address`'s incoming server once, as a pass would, and say what it answered.
///
/// Adding an account only files its password, so a wrong user name or password was first met
/// at the first sync, and arrived as "sign in again" with nothing to say what the server had
/// refused. This is run when the account is added, while the person still has the form in front
/// of them. It reads POP3's `USER`/`PASS` and IMAP's `LOGIN`, and nothing else: an account that
/// signs in in a browser has just done so, and the rest reach their server some other way.
///
/// What it learns of the server (its capabilities and folders) is kept, as a first sync would.
pub async fn check_sign_in(
    store: &Arc<SqliteStore>,
    address: &str,
    secrets: Arc<dyn AccountSecrets>,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    let address = address.to_lowercase();
    let account = configured(store)?
        .into_iter()
        .find(|account| account.address == address)
        .ok_or_else(|| CoreError::UnknownAccount(address.clone()))?;
    let (host, login) = match (&account.plan.incoming, &account.plan.auth) {
        (Incoming::Pop3 { host, .. } | Incoming::Imap { host, .. }, AuthPlan::Password { .. }) => {
            (host.clone(), username_for(&account.plan))
        }
        _ => return Ok(()),
    };
    let held = Held::new(
        stored_credential(&account, secrets.as_ref())
            .await
            .map_err(pass_error)?,
    );
    let (_tx, mut cancel) = watch::channel(false);
    let asked = match &account.plan.incoming {
        Incoming::Pop3 { .. } => {
            let mut engine = pop3_engine(store, &account, &held, secrets).map_err(pass_error)?;
            engine.refresh_caps(&mut cancel, now).await
        }
        _ => {
            let mut engine = imap_engine(store, &account, held, secrets);
            engine.refresh_caps(&mut cancel, now).await
        }
    };
    match asked {
        Ok(_) => Ok(()),
        Err(mail_runtime::RuntimeError::Proto(mail_proto::ProtoError::AuthRejected(said))) => {
            let hint = if login.contains('@') {
                " Some servers want the user name without the part after the @.".to_owned()
            } else {
                String::new()
            };
            Err(CoreError::SignInRefused {
                host,
                login,
                said,
                hint,
            })
        }
        Err(other) => Err(CoreError::ServerUnreachable {
            host,
            why: other.to_string(),
        }),
    }
}

/// The incoming server's name and port when its connections are to be upgraded with STARTTLS
/// (STLS for POP3) before anything is signed in; `None` for implicit TLS or no server.
fn starttls_for(plan: &AccountPlan) -> Option<(String, u16)> {
    match &plan.incoming {
        Incoming::Imap {
            host,
            port,
            tls: Tls::StartTlsRequired,
        }
        | Incoming::Pop3 {
            host,
            port,
            tls: Tls::StartTlsRequired,
            ..
        } => Some((host.clone(), *port)),
        _ => None,
    }
}

/// An engine for a POP3 account, signed in with the password `held` holds.
fn pop3_engine(
    store: &Arc<SqliteStore>,
    account: &Configured,
    held: &Held,
    secrets: Arc<dyn AccountSecrets>,
) -> Result<AccountEngine<Pop3Backend>, Failure> {
    let username = username_for(&account.plan);
    let Credential::Password(password) = held.current() else {
        return Err(Failure::fatal("POP3 needs a password credential"));
    };
    let sasl = sasl_for(&account.plan);
    let starttls = starttls_for(&account.plan);
    let copies = match account.remove {
        crate::offline::Remove::Never => mail_proto::backend::ServerCopies::Kept,
        crate::offline::Remove::AfterDays(_) | crate::offline::Remove::WhenDeleted => {
            mail_proto::backend::ServerCopies::Removable
        }
    };
    let backend = Pop3Backend::new(
        account.id.clone(),
        account.caps.clone(),
        Box::new(move |auth, commands| {
            // The factory owns authentication, because it is the only thing holding the
            // credential and the only thing that knows the mechanism.
            let mut all = Vec::new();
            if auth == Authenticate::First {
                all.extend(auth_prelude(&sasl));
            }
            all.extend(commands);
            let session = Pop3Session::new(username.clone(), password.expose().to_owned(), all)?;
            Ok(match &starttls {
                Some((host, port)) => session.with_starttls(host.clone(), *port),
                None => session,
            })
        }),
    )
    .with_copies(copies);
    Ok(AccountEngine::new(
        account.id.clone(),
        account.plan.clone(),
        backend,
        store.clone(),
        secrets,
    ))
}

/// An engine for an IMAP account, signed in with whatever `held` holds when it connects.
fn imap_engine(
    store: &Arc<SqliteStore>,
    account: &Configured,
    held: Held,
    secrets: Arc<dyn AccountSecrets>,
) -> AccountEngine<ImapBackend> {
    // Password *and* bearer token. Only Gmail needs OAuth; every other IMAP server this client
    // will meet authenticates with `LOGIN`, and refusing one meant the IMAP path could not be
    // used at all without registering an OAuth client.
    let mechanism = match held.current() {
        Credential::OAuth { .. } => ImapCommand::AuthenticateXoauth2,
        // A bearer token (JMAP's) has no IMAP mechanism: `LOGIN` is named, and the session
        // refuses the credential before anything is sent (`credential_forbidden`).
        Credential::Password(_)
        | Credential::ApiKey(_)
        | Credential::Bearer(_)
        | Credential::KeyPair { .. } => ImapCommand::Login,
    };
    let (username, sasl) = (username_for(&account.plan), sasl_for(&account.plan));
    let starttls = starttls_for(&account.plan);
    let backend = ImapBackend::new(
        account.id.clone(),
        account.caps.clone(),
        // The factory owns authentication, so the backend never names a mechanism and never
        // holds the credential that would decide one. It reads the credential per session, not
        // once: a renewal replaces it in `held`, and a copy taken here would be the token the
        // process started with, for as long as it runs.
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            if authenticate == Authenticate::First {
                all.push(mechanism.clone());
            }
            all.extend(commands);
            let auth = ImapAuth {
                username: username.clone(),
                credential: held.current(),
                sasl: sasl.clone(),
            };
            // A session resumed on a connection already open drops this
            // (`ImapSession::resuming`): that connection was upgraded when it was opened.
            let session = ImapSession::new(auth, all)?;
            Ok(match &starttls {
                Some((host, port)) => session.with_starttls(host.clone(), *port),
                None => session,
            })
        }),
    );
    AccountEngine::new(
        account.id.clone(),
        account.plan.clone(),
        backend,
        store.clone(),
        secrets,
    )
}

/// An engine for an account that reads through Microsoft Graph.
///
/// It presents the Graph token kept as the account's outgoing credential, which the pass has
/// just made sure of ([`sending_token`]); the incoming credential `one` read is not used.
fn graph_engine(
    store: &Arc<SqliteStore>,
    account: &Configured,
    secrets: Arc<dyn AccountSecrets>,
) -> Result<AccountEngine<OverHttp>, CoreError> {
    let reader = Reader::new(account.id.clone(), account.caps.clone())?;
    Ok(AccountEngine::new(
        account.id.clone(),
        account.plan.clone(),
        OverHttp::new(account.caps.clone()),
        store.clone(),
        secrets,
    )
    .with_graph_reader(reader))
}

/// [`SyncOps::fetch_part`](crate::SyncOps), with the secret store named, so a test can run it.
pub async fn fetch_part_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    message: mail_domain::MessageId,
    section: &mail_domain::Section,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    use mail_store::Store as _;
    let owner = store.message(message)?.account;
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == owner)
        .ok_or(CoreError::MessageAccountGone)?;
    if !matches!(account.plan.incoming, Incoming::Imap { .. }) {
        return Err(CoreError::OnlyImapLeavesAttachments);
    }
    let (_tx, mut cancel) = watch::channel(false);
    let mut engine = signed_in_imap(store, &account, secrets, registry, now).await?;
    engine
        .fetch_part(message, section, &mut cancel)
        .await
        .map(|_| ())
        .map_err(|e| CoreError::cannot("download the attachment", e))
}

/// An IMAP engine for `account`, signed in with a fresh credential and kept fresh.
async fn signed_in_imap(
    store: &Arc<SqliteStore>,
    account: &Configured,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<AccountEngine<ImapBackend>, CoreError> {
    let stored = stored_credential(account, secrets.as_ref())
        .await
        .map_err(pass_error)?;
    let credential = signed_in(account, stored, secrets.as_ref(), registry, now).await?;
    let held = Held::new(credential);
    let renewal = renewal_for(
        account,
        &held,
        secrets.clone(),
        registry,
        clock_for(Mode::Once, now),
    );
    let mut engine = imap_engine(store, account, held, secrets);
    if let Some(source) = renewal {
        engine = engine.with_tokens(source);
    }
    Ok(engine)
}

/// [`SyncOps::drain`](crate::SyncOps), with the secret store named, so a test can run it.
pub async fn drain_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, CoreError> {
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or(CoreError::AccountGone)?;
    if !matches!(account.plan.incoming, Incoming::Imap { .. }) {
        return Err(CoreError::OnlyImapUploads);
    }
    let (_tx, mut cancel) = watch::channel(false);
    let mut engine = signed_in_imap(store, &account, secrets, registry, now).await?;
    Ok(engine.drain_outbox(&mut cancel, now).await?)
}

/// How long the caller's `now` is good for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `mailo sync`, and each pass of the scheduler: one pass, so `now` is the whole of it.
    Once,
    /// A connection held for as long as the process lives ([`live`]): it outlives any `now` it
    /// could be handed, so it reads the wall clock.
    Watch,
}

/// Drive one account through a single pass.
///
/// How the pass ended is the return. Passes over time are the scheduler's
/// (`mail_runtime::schedule`), which `mailo watch` and the window share; what a watch says
/// between them is [`watcher::told`].
pub async fn drive<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    account: &Configured,
    mailboxes: &[MailboxRef],
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
) -> PassEnd {
    let result = pass(engine, mailboxes, account, cancel, now, None).await;
    ended(account, result, None)
}

/// How long a live watch sleeps between waits on a server that cannot be waited on, and the
/// floor under one that can ([`live`]).
fn poll_floor(watch: &WatchMode) -> std::time::Duration {
    match watch {
        WatchMode::Poll { every } => *every,
        // An IDLE account still needs a floor: `watch` returns as soon as the server says
        // anything, and a mailbox that is busy would otherwise pass in a tight loop.
        WatchMode::Idle => std::time::Duration::from_secs(30),
    }
}

/// One whole sync pass, whatever the protocol underneath.
///
/// Shared rather than written once per backend, so POP3 and IMAP cannot drift: both fetch
/// headers, then bodies, then drain the outbox. The surest way to make two paths the same is for
/// there to be one.
///
/// The pass's connection is closed as it ends, however it ends ([`AccountEngine::let_go`]): the
/// next pass is minutes away, and the connection would only sit open until then.
async fn pass<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailboxes: &[MailboxRef],
    account: &Configured,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    emit: Emit<'_>,
) -> Result<Done, Failure> {
    let passed = pass_on(engine, mailboxes, account, cancel, now, emit).await;
    engine.let_go().await;
    passed
}

/// [`pass`], on whatever connection it is given.
async fn pass_on<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailboxes: &[MailboxRef],
    account: &Configured,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    emit: Emit<'_>,
) -> Result<Done, Failure> {
    // Reachability first, so an unreachable server reports once rather than three times as each
    // pass opens its own connection.
    say(emit, Progress::Connecting);
    engine.reachable().await.map_err(|e| Failure::of("", &e))?;

    // Ask what the server supports before deciding how to talk to it. Stored capabilities start
    // as the preset's expectation, and an expectation that is never checked is a guess the
    // client acts on for ever — CONDSTORE, MOVE and the special-use folder roles were all
    // undiscoverable until this call existed.
    if engine.caps_are_stale(now) {
        match engine.refresh_caps(cancel, now).await {
            Ok(_) => {}
            // A refused sign-in is reported as one, so the watch stops and says so. As prose it
            // read as an ordinary failure, and a watch retries those every minute — against a
            // credential already refused.
            Err(e) if e.retry().needs_person() => {
                let mut done = Done::default();
                done.trouble(None, e.retry(), e.to_string());
                return Ok(done);
            }
            // Not fatal. A server that refuses CAPABILITY still delivers mail, and the stored
            // expectation is a worse answer than the truth but a better one than stopping.
            Err(e) => return Err(Failure::of("could not read capabilities: ", &e)),
        }
    }

    // The inbox first and in full, then the other folders: a first sync of a large Archive must
    // not be what stands between the user and their new mail. `to_sync` puts them in that order
    // and a failure in a later one does not lose what the earlier ones fetched.
    //
    // Every folder has its own budget, so a pass is bounded by the number of folders times
    // it, and the inbox's share is spent before any folder's: however large a folder's first
    // sync, each pass brings the inbox up to date before it moves on.
    let mut done = Done::default();
    for mailbox in mailboxes {
        one_mailbox(engine, mailbox, account.keep, cancel, now, &mut done, emit).await?;
    }

    // Rules, over what arrived in any folder this pass, before the drain so that what they
    // queue leaves in the same pass. After the bodies, so a rule about words in the body finds
    // them for every message the pass had room to fetch. The junk filter before them or after,
    // as the person chose: whichever goes second sees only what the first left in the inbox.
    let junk = account.junk;
    if junk.order == crate::junk::Order::BeforeRules {
        sift(engine, account.id.clone(), &junk, &mut done, now);
    }
    match engine.run_rules(&done.counts.arrived, now) {
        Ok(ran) => {
            done.counts.ruled.extend(ran.acted);
            // Forwards and replies are queued before the drain, so they leave in this pass.
            let noted = crate::rules::speak::carry_out(
                engine.store(),
                &ran.left,
                crate::rules::guard::Occasion::Arrival,
                now,
            );
            done.counts.noted.extend(noted);
        }
        Err(e) => done.trouble(None, e.retry(), format!("rules: {e}")),
    }
    if junk.order == crate::junk::Order::AfterRules {
        sift(engine, account.id.clone(), &junk, &mut done, now);
    }

    say(emit, Progress::Sending);
    let drained = engine
        .drain_outbox(cancel, now)
        .await
        .map_err(|e| Failure::of("", &e))?;
    done.counts.outbox_settled += drained.outbox_settled;
    done.counts.submitted += drained.submitted;
    done.counts.appended += drained.appended;
    done.counts.still_queued = drained.still_queued;
    // The drain classified its own failures and they were dropped here, so a credential the
    // outbox found rejected did not stop the poll loop the way one found by a fetch does.
    done.flags(&drained);
    done.notes(None, drained.needs_attention);

    // The folder list, on an account that has never had one: otherwise it arrives with the
    // daily capability refresh, and an account added this morning could not list, rename or
    // delete a folder until tomorrow. After the drain, so it shows any folder work just sent.
    //
    // Not after a refused sign-in or a rate limit, which would only add one more failed login
    // or one more request the server asked not to receive.
    if engine.folders_unlisted()
        && !report::needs_person(&done.trouble)
        && report::hold(&done.trouble).is_none()
    {
        say(emit, Progress::Folders);
        if let Err(e) = engine.refresh_folders(cancel, now).await {
            done.trouble(None, e.retry(), format!("folders: {e}"));
        }
    }
    Ok(done)
}

/// Run the junk filter over what the pass brought. A filter that cannot read its counts says
/// so and the pass goes on: unjudged mail in the inbox is what there was before the filter.
fn sift<B: mail_proto::Backend>(
    engine: &AccountEngine<B>,
    account: AccountId,
    policy: &crate::junk::Policy,
    done: &mut Done,
    now: chrono::DateTime<chrono::Utc>,
) {
    if let Err(e) = crate::junk::sift(
        engine.store(),
        account,
        engine.caps(),
        &done.counts.arrived,
        policy,
        now,
    ) {
        junk_failed(done, e);
    }
}

/// Say that the junk filter failed, without failing the pass.
fn junk_failed(done: &mut Done, why: CoreError) {
    done.trouble(None, mail_domain::Retry::Now, format!("junk filter: {why}"));
}

/// Most headers one mailbox takes in one pass. The rest wait for the next pass, behind the
/// inbox's.
const HEADERS_PER_PASS: usize = 200;

/// Most bodies one mailbox takes in one pass.
const BODIES_PER_PASS: usize = 100;

/// Most of what a sync left on the server one mailbox fetches in one pass, for an account kept
/// offline in full: twenty attachments or 64 MiB, whichever comes first, and always at least
/// one. A pass is what stands between the user and their next new mail, so a mailbox of large
/// attachments is brought here over many passes rather than in one that does not end.
const PARTS_PER_PASS: mail_runtime::PartBudget = mail_runtime::PartBudget {
    parts: 20,
    bytes: 64 * 1024 * 1024,
};

/// One mailbox's share of a pass: headers, then the server's view of what is held, then bodies.
///
/// Failures are folded into `report` rather than returned: a folder the server will not select
/// is worth seeing and is not a reason to abandon the mail already in hand. The one returned is
/// a refusal of the protocol itself ([`report::ProtocolRefused`]), which is the account's and not
/// the folder's: every other folder would meet it too, and the pass ends on it so the account
/// says so once.
async fn one_mailbox<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailbox: &MailboxRef,
    keep: crate::offline::Keep,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    done: &mut Done,
    emit: Emit<'_>,
) -> Result<(), Failure> {
    let path = mailbox.path.as_str();
    let headers = |done: u32| Progress::Headers {
        mailbox: path.to_owned(),
        done,
        of: None,
    };
    say(emit, headers(0));
    let one = match engine.sync(mailbox, cancel, now, HEADERS_PER_PASS).await {
        Ok(report) => report,
        // Named, because "cannot select" on a folder the server listed is worth seeing and
        // is not a reason to abandon the mail already in hand.
        Err(e) => {
            // Classified while the error is still typed. By the time it reaches the user it
            // is prose, and prose is not something a loop can safely decide on.
            let failure = Failure::of("", &e);
            if failure.refused.is_some() {
                return Err(failure);
            }
            done.trouble(Some(path), e.retry(), format!("{path}: {e}"));
            return Ok(());
        }
    };
    done.counts.headers_fetched += one.headers_fetched;
    done.counts.arrived.extend(one.arrived);
    done.notes(Some(path), one.needs_attention);
    say(emit, headers(done_count(one.headers_fetched)));

    // What the server knows and a header fetch does not carry: flags, Gmail's labels, and
    // messages that have gone. Without `AccountEngine::sweep`, mail read on a phone would stay
    // bold, unread counts would be the size of the mailbox, and labels would never arrive,
    // since they ride the same survey.
    //
    // After the header fetch, because a sweep is about messages already held, and before the
    // bodies, so the list is right as soon as it is populated. Failures are reported and do
    // not abandon the mail already in hand, like every other step here.
    say(
        emit,
        Progress::Flags {
            mailbox: path.to_owned(),
        },
    );
    match engine.sweep(mailbox, cancel, now).await {
        Ok(swept) => done.notes(Some(path), swept.needs_attention),
        Err(e) => done.trouble(Some(path), e.retry(), format!("{path}: {e}")),
    }

    // Headers first, then bodies smallest-band-first behind them, so the inbox is usable
    // long before the hundred large attachments finish.
    say(emit, Progress::Bodies { done: 0, of: None });
    match engine
        .fetch_bodies(mailbox, cancel, now, BODIES_PER_PASS)
        .await
    {
        Ok(bodies) => {
            done.counts.bodies_fetched += bodies.bodies_fetched;
            done.notes(Some(path), bodies.needs_attention);
            say(
                emit,
                Progress::Bodies {
                    done: done_count(bodies.bodies_fetched),
                    of: None,
                },
            );
            // Then, for an account kept offline in full, what the bodies left on the server,
            // largest last. Only once this pass had room to spare for bodies: while a first
            // sync still has messages to fetch, reading them comes before their attachments.
            if keep == crate::offline::Keep::Everything && bodies.bodies_fetched < BODIES_PER_PASS {
                remote_parts(engine, mailbox, cancel, done).await;
            }
        }
        Err(e) => done.trouble(Some(path), e.retry(), format!("{path}: {e}")),
    }
    Ok(())
}

/// A count of messages, as the `u32` a [`Progress`] carries.
fn done_count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// Fetch a pass's share of the attachments earlier passes left on the server in `mailbox`.
async fn remote_parts<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailbox: &MailboxRef,
    cancel: &mut mail_runtime::Cancel,
    done: &mut Done,
) {
    let path = mailbox.path.as_str();
    match engine
        .fetch_remote_parts(mailbox, cancel, PARTS_PER_PASS)
        .await
    {
        Ok(parts) => {
            done.counts.parts_fetched += parts.parts_fetched;
            done.flags(&parts);
            done.notes(Some(path), parts.needs_attention);
        }
        Err(e) => done.trouble(Some(path), e.retry(), format!("{path}: {e}")),
    }
}

/// Fetch one folder now ([`SyncOps::folder_now`](crate::SyncOps)): its first page of headers, what
/// the server says of them, and bodies. The secret store is named, so a test can run it.
pub async fn folder_now_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    path: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PassEnd, CoreError> {
    let account = configured(&store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or(CoreError::NoSuchAccount)?;
    match account.plan.incoming {
        Incoming::Pop3 { .. } => return Err(CoreError::Pop3OneMailbox),
        Incoming::Local => {
            return Err(CoreError::LocalNoServerToFetch {
                address: account.address,
            });
        }
        Incoming::Imap { .. } | Incoming::Graph => {}
        // Every pass fetches the whole account, and a folder's mail arrives with its label.
        Incoming::Jmap { .. } => {
            return Err(CoreError::FoldersAreLabelsWhole {
                address: account.address,
            });
        }
    }
    if account.caps.labels == ServerLabels::Supported {
        return Err(CoreError::FoldersAreLabels {
            address: account.address,
        });
    }
    let mailbox = MailboxRef {
        account: account.id.clone(),
        path: path.to_owned(),
    };
    let done = async {
        let stored = stored_credential(&account, secrets.as_ref()).await?;
        let credential = signed_in_typed(&account, stored, secrets.as_ref(), registry, now).await?;
        let (_tx, mut cancel) = watch::channel(false);
        let held = Held::new(credential);
        let renewal = renewal_for(
            &account,
            &held,
            secrets.clone(),
            registry,
            clock_for(Mode::Once, now),
        );
        let mut done = Done::default();
        if account.plan.incoming == Incoming::Graph {
            sending_token_typed(&account, secrets.as_ref(), registry, now).await?;
            let mut engine = graph_engine(&store, &account, secrets.clone())
                .map_err(|e| Failure::fatal(e.to_string()))?;
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            one_mailbox(
                &mut engine,
                &mailbox,
                account.keep,
                &mut cancel,
                now,
                &mut done,
                None,
            )
            .await?;
            return Ok(done);
        }
        let mut engine = imap_engine(&store, &account, held, secrets.clone());
        if let Some(source) = renewal {
            engine = engine.with_tokens(source);
        }
        engine.reachable().await.map_err(|e| Failure::of("", &e))?;
        one_mailbox(
            &mut engine,
            &mailbox,
            account.keep,
            &mut cancel,
            now,
            &mut done,
            None,
        )
        .await?;
        Ok(done)
    }
    .await;
    Ok(ended(&account, done, None))
}

/// The mailboxes the next pass would fetch, per account, by address.
///
/// Reported by `mailo account list`, because "why is my Sent folder empty" is a question a user
/// asks and the answer is a fact this client knows: only these folders are fetched. Exposed
/// rather than duplicated so a test can ask the code rather than restate its rules.
pub fn mailboxes_by_account(store: &SqliteStore) -> Result<Vec<(String, Vec<String>)>, CoreError> {
    use mail_store::Store as _;
    configured(store)?
        .into_iter()
        .map(|account| {
            let folders = store.folders(account.id.clone())?;
            let paths = to_sync(&account, &folders)
                .into_iter()
                .map(|m| m.path)
                .collect();
            Ok((account.address, paths))
        })
        .collect()
}

/// Which mailboxes a pass should fetch, in the order it should fetch them.
///
/// The inbox first and always, then `Sent` where the server named one.
///
/// Only the inbox was ever fetched, which nothing said and nothing tested: a user who sends mail
/// from their phone opened Sent and found only what this client had sent, and mail archived
/// elsewhere vanished from the inbox without appearing in Archive. `Spam` is deliberately not
/// here — downloading the spam folder to populate a place nobody opens costs a first sync twice
/// over — and neither is `Drafts`, whose server copies are other clients' half-written mail and
/// would collide with this one's outbox.
///
/// POP3 has one mailbox by construction, and a server that answered no roles gets the inbox
/// alone, which is what happened before this existed.
///
/// Then, on an IMAP account whose folders are folders, every other folder the user follows
/// (see [`followed`]), in the order the store lists them. Not on one whose folders are labels:
/// there a folder is a label on mail already fetched from the inbox and Sent, and fetching it
/// again as a mailbox would fetch the account's mail once per label.
fn to_sync(account: &Configured, folders: &[Folder]) -> Vec<MailboxRef> {
    match account.plan.incoming {
        Incoming::Local => return Vec::new(),
        // One mailbox, the whole account: JMAP reports changes across every mailbox at once.
        Incoming::Jmap { .. } => {
            return vec![MailboxRef {
                account: account.id.clone(),
                path: JMAP_ALL.to_owned(),
            }];
        }
        Incoming::Imap { .. } | Incoming::Pop3 { .. } | Incoming::Graph => {}
    }
    let mut out = vec![MailboxRef {
        account: account.id.clone(),
        path: "INBOX".to_owned(),
    }];
    // `Sent` and not `Archive`, which is not safe yet. On Gmail the same message is in INBOX
    // *and* in All Mail, and `Message.mailbox` is one role: a pass over All Mail would mark a
    // message that is in the inbox as archived and it would leave the list. That needs the
    // message to carry a set of mailboxes rather than one, which is a domain change and not a
    // sync one. A message in Sent is not also in the inbox, so Sent is safe today.
    if let Some(path) = account.caps.folders.path(MailboxRole::Sent)
        && !path.eq_ignore_ascii_case("INBOX")
    {
        out.push(MailboxRef {
            account: account.id.clone(),
            path: path.to_owned(),
        });
    }
    let folders_are_folders = matches!(
        account.plan.incoming,
        Incoming::Imap { .. } | Incoming::Graph
    ) && account.caps.labels != ServerLabels::Supported;
    if folders_are_folders {
        for folder in folders
            .iter()
            .filter(|f| followed(f, &account.caps.folders))
        {
            if !out.iter().any(|m| m.path == folder.path) {
                out.push(MailboxRef {
                    account: account.id.clone(),
                    path: folder.path.clone(),
                });
            }
        }
    }
    out
}

/// The mailboxes besides the inbox a push would hear of: the ones a pass fetches that mail
/// arrives in, so not Sent, which only this client fills.
fn pushed_folders(account: &Configured, folders: &[Folder]) -> Vec<MailboxRef> {
    let sent = account.caps.folders.path(MailboxRole::Sent);
    to_sync(account, folders)
        .into_iter()
        .skip(1)
        .filter(|m| sent != Some(m.path.as_str()))
        .take(PUSHED_FOLDERS)
        .collect()
}

/// At most this many folders beside the inbox are named to a server's `NOTIFY`: a server that
/// lists hundreds would be sent a command it may refuse for its length.
const PUSHED_FOLDERS: usize = 50;

/// Whether a pass fetches this folder, beside the inbox and Sent.
///
/// One the user follows (`LSUB`) that can hold mail. An unfollowed one is fetched when it is
/// opened, by [`SyncOps::folder_now`](crate::SyncOps), and not on every pass: a server can list hundreds of them.
///
/// Trash, Junk and Archive are fetched like any folder, and what arrives from them is filed
/// under their roles (`FolderRoles::filed_as`). Not these:
///
/// - Drafts. Its server copies are this client's own uploaded drafts and other clients'
///   half-written mail; fetched as messages they would stand beside the drafts this client
///   already holds, and collide with its outbox.
/// - `\All`, `\Flagged` and `\Important`. They are views of mail held in other folders, and
///   fetching one fetches that mail a second time.
fn followed(folder: &Folder, roles: &FolderRoles) -> bool {
    folder.subscription == Subscription::Subscribed
        && folder.holds == Holds::Mail
        && !folder.path.eq_ignore_ascii_case("INBOX")
        && !matches!(
            folder.special,
            Some(
                SpecialUse::Drafts | SpecialUse::All | SpecialUse::Flagged | SpecialUse::Important
            )
        )
        && roles.role(&folder.path) != Some(MailboxRole::Drafts)
}

/// The commands that authenticate a POP3 session, given the mechanisms on offer.
fn auth_prelude(sasl: &[SaslMech]) -> Vec<Pop3Command> {
    if sasl.contains(&SaslMech::Plain) {
        vec![Pop3Command::AuthPlain]
    } else {
        // USER/PASS is the universal fallback and the only thing some servers implement.
        vec![Pop3Command::User, Pop3Command::Pass]
    }
}

// Both live on `AccountPlan` now, because `AccountEngine` needs the same answers to
// authenticate a submission and two derivations of "what do I log in as" is how one of them
// keeps working while the other quietly cannot. The wrappers stay so the tests below keep
// naming what they are testing.
fn username_for(plan: &AccountPlan) -> String {
    plan.username()
}

fn sasl_for(plan: &AccountPlan) -> Vec<SaslMech> {
    plan.sasl()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_auth_prelude_prefers_sasl_but_falls_back() {
        // Many servers offer PLAIN only; some older servers offer neither and only understand USER/PASS.
        assert_eq!(
            auth_prelude(&[SaslMech::Plain]),
            vec![Pop3Command::AuthPlain]
        );
        assert_eq!(
            auth_prelude(&[]),
            vec![Pop3Command::User, Pop3Command::Pass]
        );
    }

    #[test]
    fn a_password_login_resolves_to_the_local_part_where_the_preset_says_so() {
        // Getting this wrong is an authentication failure with no explanation.
        let preset = mail_domain::presets::manual_pop3(
            "s1234567@example.edu",
            &mail_domain::presets::ManualPop3 {
                pop3_host: "pop.example.edu".to_owned(),
                pop3_port: 995,
                smtp_host: "smtp.example.edu".to_owned(),
                smtp_port: 465,
                login: Some("s1234567".to_owned()),
            },
            crate::support_tests::now(),
        );
        assert_eq!(username_for(&preset.plan), "s1234567");
    }
}

//! One sync pass, from stored configuration to a real connection.
//!
//! This is where everything meets: the preset that configured the account, the credential in the
//! keyring, the backend that speaks the protocol, and the store that keeps the result. It is
//! also the only place in the application that needs an async runtime.

pub use crate::notify::Announce;
pub mod due;
pub mod live;
pub mod report;

mod body;
pub use body::{fetch_body, fetch_body_with};
mod jmap;
mod search;

use mail_domain::id::account_id_from_uuid;
use mail_domain::*;
use mail_proto::backend::{Authenticate, ImapBackend, Pop3Backend};
use mail_proto::{ImapAuth, ImapCommand, ImapSession, Pop3Command, Pop3Session};
use mail_runtime::graph::read::{OverHttp, Reader};
use mail_runtime::link::LinkedTokens;
use mail_runtime::tokens::{self, Now};
use mail_runtime::{
    AccountEngine, AccountSecrets, ClientRegistry, Held, OAuthTokens, SyncReport, TokenSource,
    clients, platform_secrets,
};
use mail_store::SqliteStore;
use porter_core::{AccountId, Credential, Family, SecretKey, SecretPurpose, SecretText};
use report::{Done, Emit, Failure, Hooks, PassEnd, Progress, Told, Watched};
pub use search::{search_server, search_server_with};
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
}

/// What the server was last observed to support, for one account.
///
/// `None` when nothing has connected yet, which the caller must treat as "assume nothing" rather
/// than as an error: the window can be opened before the first sync, and refusing to act at all
/// would be worse than acting locally.
pub fn caps_of(store: &SqliteStore, account: AccountId) -> Option<AccountCaps> {
    let db = store.connection();
    let stored: Option<String> = db
        .query_row(
            "SELECT caps FROM account_caps WHERE account = ?1",
            [account.to_string()],
            |r| r.get(0),
        )
        .ok();
    serde_json::from_str(&stored?).ok()
}

/// Read the accounts back out of the store.
pub(crate) fn configured(store: &SqliteStore) -> Result<Vec<Configured>, String> {
    let db = store.connection();
    let mut stmt = db
        .prepare(&format!(
            "SELECT a.id, a.address, a.plan, c.caps
             FROM {} a LEFT JOIN account_caps c ON c.account = a.id
             ORDER BY a.created_at",
            store.accounts()
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for row in rows {
        let (id, address, plan, caps) = row.map_err(|e| e.to_string())?;
        let Ok(uuid) = id.parse() else { continue };
        let plan: AccountPlan = serde_json::from_str(&plan)
            .map_err(|e| format!("{address}: stored plan is unreadable: {e}"))?;
        let caps: AccountCaps = match caps {
            Some(text) => serde_json::from_str(&text)
                .map_err(|e| format!("{address}: stored capabilities are unreadable: {e}"))?,
            // No capabilities yet means nothing has connected. The expected ones from the
            // preset are a starting point, not a claim about the server.
            None => {
                return Err(format!(
                    "{address}: no capabilities recorded. Something has gone wrong with setup."
                ));
            }
        };
        out.push(Configured {
            id: account_id_from_uuid(uuid),
            address,
            plan,
            caps,
            keep: crate::offline::Keep::default(),
        });
    }
    Ok(out)
}

/// How each configured account signs in, by address.
///
/// Beside `mailboxes_by_account` and for the same reason: `account list` should be able to say
/// what an account is waiting for without deciding it for itself. Two surfaces answering that
/// question separately is how `sync` came to tell a Google account to find a password.
pub fn auth_by_account(
    store: &SqliteStore,
) -> Result<std::collections::HashMap<String, AuthPlan>, String> {
    Ok(configured(store)?
        .into_iter()
        .map(|account| (account.address, account.plan.auth))
        .collect())
}

/// Every configured account's id and address, in the order they were added.
pub fn addresses(store: &SqliteStore) -> Vec<(AccountId, String)> {
    configured(store)
        .unwrap_or_default()
        .into_iter()
        .map(|account| (account.id, account.address))
        .collect()
}

/// The accounts that keep their mail on this computer ([`Incoming::Local`]).
pub fn local_accounts(store: &SqliteStore) -> Vec<AccountId> {
    configured(store)
        .unwrap_or_default()
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
        .ok()
        .into_iter()
        .flatten()
        // IDLE is a long-lived connection this loop does not hold. Until it does, an account that
        // supports it is polled like any other, at `due::IDLE`.
        .map(|a| due::every(&a.caps.watch))
        .min()
        .unwrap_or(default)
}

/// Keep syncing until the process is stopped — `mailo watch`, and `plan.md` phase 8g.
///
/// Where `AccountEngine::watch` finally has a caller outside a test. IDLE is a connection held
/// open for as long as the server allows, so it needs a process whose job is to stay open; the
/// window was meant to be that and F140 says it is not, so this is.
///
/// `notifications` decides whether what each pass fetches is announced on the desktop
/// (`plan.md` 10.6). With no session bus to announce on, it is as if they were off.
///
/// A watch has no end to return a report at, so each thing it would say is handed to `tell` as it
/// happens ([`Watched`]) and the caller words it. It returns only once every account has stopped
/// for a reason worth stopping for, with how each ended.
pub fn watch(
    store: Arc<SqliteStore>,
    now: chrono::DateTime<chrono::Utc>,
    notifications: crate::notify::Setting,
    tell: &dyn Fn(Watched),
) -> Result<Vec<PassEnd>, String> {
    let registry = clients::load_default().map_err(|e| e.to_string())?;
    let desktop;
    let announce = match notifications {
        crate::notify::Setting::On => {
            desktop = crate::notify::desktop::Desktop::connect();
            Announce::To {
                store: &store,
                notifier: &desktop,
            }
        }
        crate::notify::Setting::Off => Announce::Quietly,
    };
    run_all(
        store.clone(),
        platform_secrets(),
        &registry,
        now,
        Mode::Watch,
        announce,
        &Scope {
            due: &|_| true,
            kept: &crate::offline::load_default(),
        },
        Hooks {
            told: Some(tell),
            ..Hooks::default()
        },
    )
}

/// Sync every configured account that has a credential, how each ended kept as data.
///
/// Accounts without one end as [`PassEnd::Failed`] rather than failing the run: having one
/// account that needs attention should not stop the others from fetching mail. An account with
/// nothing to sync has no entry, so a run over no accounts is an empty list, and what to say of
/// that is for the caller.
pub fn run(
    store: Arc<SqliteStore>,
    now: chrono::DateTime<chrono::Utc>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, String> {
    // The registry is read once per run rather than once per account: it is deployment
    // configuration, and an edit halfway through a run producing two different client ids is
    // not a behaviour worth having.
    let registry = clients::load_default().map_err(|e| e.to_string())?;
    run_all(
        store,
        platform_secrets(),
        &registry,
        now,
        Mode::Once,
        Announce::Quietly,
        &Scope {
            due: &|_| true,
            kept: &crate::offline::load_default(),
        },
        hooks,
    )
}

/// The same, with the secret store named.
///
/// The keyring is the only thing in this function that cannot exist in a test, and it was
/// reached for directly — so the one function that ties configuration, credentials, the backend,
/// the engine and the store together was the one function nothing could run. Every piece below
/// it had tests; the assembly had none.
///
/// Injected rather than faked at a lower level, because what is worth testing here *is* the
/// assembly: which credential is fetched for which purpose, what happens to an account that has
/// none, and that a failure in one account does not stop the next.
pub fn run_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, String> {
    run_all(
        store,
        secrets,
        registry,
        now,
        Mode::Once,
        Announce::Quietly,
        &Scope {
            due: &|_| true,
            kept: &crate::offline::Kept::default(),
        },
        hooks,
    )
}

/// Which accounts a run syncs, and how much of each it keeps.
struct Scope<'a> {
    /// Whether an account is due a pass ([`due`]).
    due: &'a dyn Fn(AccountId) -> bool,
    /// Which accounts keep everything offline ([`crate::offline`]).
    kept: &'a crate::offline::Kept,
}

/// A run over the accounts in `scope`, told whether to stop after one pass.
#[allow(clippy::too_many_arguments)]
fn run_all(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    announce: Announce<'_>,
    scope: &Scope<'_>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, String> {
    let accounts = pick(&store, scope)?;
    pass_over(
        &store, secrets, registry, now, mode, announce, accounts, hooks,
    )
}

/// The accounts a run is to sync.
fn pick(store: &SqliteStore, scope: &Scope<'_>) -> Result<Vec<Configured>, String> {
    // An account that keeps its mail here has no server: nothing to fetch, nothing to drain,
    // and no credential to ask the keyring for. Left out rather than reported, because a line
    // saying so on every pass would be noise about something that is working as intended.
    Ok(configured(store)?
        .into_iter()
        .filter(|account| !matches!(account.plan.incoming, Incoming::Local))
        .filter(|a| (scope.due)(a.id.clone()))
        .map(|a| Configured {
            keep: scope.kept.of(a.id.clone()),
            ..a
        })
        .collect())
}

/// One pass over `accounts`, concurrently, each ended as a [`PassEnd`].
#[allow(clippy::too_many_arguments)]
fn pass_over(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    announce: Announce<'_>,
    accounts: Vec<Configured>,
    hooks: Hooks<'_>,
) -> Result<Vec<PassEnd>, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;

    // All the accounts at once — `plan.md` phase 8f.
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
    // mutex, so those milliseconds cannot overlap anyway. That is phase 8b's subject.
    //
    // Nothing here needs a rate limiter. Throttling is a within-account question — one server,
    // several connections — and this is one connection each to servers that have never heard of
    // each other.
    //
    // Ended in the order the accounts are configured, which `join_all` preserves. A pass that
    // listed its accounts in whatever order they happened to finish would read differently
    // between runs for no reason the user could see.
    Ok(
        runtime.block_on(futures_util::future::join_all(accounts.iter().map(
            |account| async {
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
                    announce,
                    cancel,
                    Some(&emit),
                    hooks.told,
                )
                .await;
                ended(account, result, cancelled)
            },
        ))),
    )
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
        Err(Failure { retry, why, pause }) => PassEnd::Failed {
            account: account.id.clone(),
            address: account.address.clone(),
            retry,
            why,
            pause,
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
) -> Result<Credential, String> {
    signed_in_typed(account, credential, secrets, registry, now)
        .await
        .map_err(|failure| failure.why)
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
        return Err(Failure::reauth(crate::account::no_client_id(
            *issuer,
            &account.address,
        )));
    };
    tokens::renew(account.id.clone(), client, scopes, credential, secrets, now)
        .await
        .map_err(|e| Failure::of("cannot renew the sign-in: ", &e))
}

/// The credential stored for `account`'s sign-in: what the keyring holds for an account of mailo's
/// own. An account that is the desktop's accountd's holds none here (its servers are reached
/// through accountd's relays and its bearers asked for on its grant, `mail_runtime::link`), so
/// what comes back is a placeholder no engine reads; without a link to accountd it cannot be
/// reached at all, which is said as something to try again.
pub(crate) async fn stored_credential(
    account: &Configured,
    secrets: &dyn AccountSecrets,
) -> Result<Credential, Failure> {
    if account.plan.grant().is_some() {
        return match secrets.link() {
            Some(_) => Ok(Credential::Password(SecretText::new(""))),
            None => Err(Failure {
                retry: Retry::After(std::time::Duration::from_secs(30)),
                why: format!(
                    "{} is an account of the desktop's account service, which is not reachable",
                    account.address
                ),
                pause: crate::fetch::Pause::ServerBusy,
            }),
        };
    }
    secrets
        .get(&SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::IncomingPassword,
        })
        .await
        // A credential that is not there is one to be asked for again.
        .map_err(|_| {
            Failure::reauth(crate::account::no_credential(
                &account.address,
                &account.plan.auth,
            ))
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
    // An account that is accountd's presents a bearer only to Graph, and asks accountd for it.
    if let AuthPlan::Granted { grant, .. } = &account.plan.auth {
        let link = secrets.link()?;
        return (account.plan.incoming == Incoming::Graph).then(|| {
            let audience = porter_core::Audience(Family::Graph.slug());
            let tokens = LinkedTokens::new(link, grant.clone(), audience).with_clock(clock);
            Arc::new(tokens) as Arc<dyn TokenSource>
        });
    }
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
    announce: Announce<'_>,
    cancel: Option<watch::Receiver<bool>>,
    emit: Emit<'_>,
    told: Told<'_>,
) -> Result<Done, Failure> {
    say(emit, Progress::Connecting);
    let stored = stored_credential(account, secrets.as_ref()).await?;
    let credential = signed_in_typed(account, stored, secrets.as_ref(), registry, now).await?;
    // Not fatal to the pass: an account that cannot send can still receive.
    let sending = sending_token_typed(account, secrets.as_ref(), registry, now)
        .await
        .err();
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
            let username = username_for(&account.plan);
            let Credential::Password(password) = held.current() else {
                return Err(Failure::fatal("POP3 needs a password credential"));
            };
            let sasl = sasl_for(&account.plan);
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
                    Pop3Session::new(username.clone(), password.expose().to_owned(), all)
                }),
            );
            let mut engine = AccountEngine::new(
                account.id.clone(),
                account.plan.clone(),
                backend,
                store.clone(),
                secrets,
            );
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            follow(
                &mut engine,
                account,
                &mailboxes,
                &mut cancel,
                now,
                mode,
                announce,
                emit,
                told,
            )
            .await
        }
        Incoming::Imap { .. } => {
            let mut engine = imap_engine(store, account, held, secrets);
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            follow(
                &mut engine,
                account,
                &mailboxes,
                &mut cancel,
                now,
                mode,
                announce,
                emit,
                told,
            )
            .await
        }
        Incoming::Graph => {
            let mut engine = graph_engine(store, account, secrets).map_err(Failure::fatal)?;
            if let Some(source) = renewal {
                engine = engine.with_tokens(source);
            }
            follow(
                &mut engine,
                account,
                &mailboxes,
                &mut cancel,
                now,
                mode,
                announce,
                emit,
                told,
            )
            .await
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
            jmap::drive(
                &mut engine,
                account,
                &mut cancel,
                now,
                mode,
                announce,
                emit,
                told,
            )
            .await
        }
    };
    report.map(|mut done| {
        if let Some(failure) = sending {
            done.trouble(None, failure.retry, failure.why);
        }
        done
    })
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
) -> Result<(), String> {
    sending_token_typed(account, secrets, registry, now)
        .await
        .map_err(|failure| failure.why)
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
    let relayed = account.plan.grant().is_some();
    let backend = ImapBackend::new(
        account.id.clone(),
        account.caps.clone(),
        // The factory owns authentication, so the backend never names a mechanism and never
        // holds the credential that would decide one. It reads the credential per session, not
        // once: a renewal replaces it in `held`, and a copy taken here would be the token the
        // process started with, for as long as it runs.
        Box::new(move |authenticate, commands: Vec<ImapCommand>| {
            let mut all = Vec::new();
            // A relay's connection greets with `PREAUTH`: nothing to sign in with.
            if authenticate == Authenticate::First && !relayed {
                all.push(mechanism.clone());
            }
            all.extend(commands);
            let auth = ImapAuth {
                username: username.clone(),
                credential: held.current(),
                sasl: sasl.clone(),
            };
            ImapSession::new(auth, all)
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
) -> Result<AccountEngine<OverHttp>, String> {
    let reader =
        Reader::new(account.id.clone(), account.caps.clone()).map_err(|e| e.to_string())?;
    Ok(AccountEngine::new(
        account.id.clone(),
        account.plan.clone(),
        OverHttp::new(account.caps.clone()),
        store.clone(),
        secrets,
    )
    .with_graph_reader(reader))
}

/// Download one attachment a sync left on the server, and record it as held.
///
/// Blocking, with a runtime of its own, for the same reason [`run`] has one: its callers are a
/// command and a click handler, not async code.
pub fn fetch_part(
    store: &Arc<SqliteStore>,
    message: mail_domain::MessageId,
    section: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let registry = clients::load_default().map_err(|e| e.to_string())?;
    fetch_part_with(store, platform_secrets(), &registry, message, section, now)
}

/// The same, with the secret store named, so a test can run it.
pub fn fetch_part_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    message: mail_domain::MessageId,
    section: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    use mail_store::Store as _;
    let owner = store.message(message).map_err(|e| e.to_string())?.account;
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == owner)
        .ok_or_else(|| "the account this message belongs to is no longer configured".to_owned())?;
    if !matches!(account.plan.incoming, Incoming::Imap { .. }) {
        return Err("only IMAP leaves attachments on the server".to_owned());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let (_tx, mut cancel) = watch::channel(false);
        let mut engine = signed_in_imap(store, &account, secrets, registry, now).await?;
        engine
            .fetch_part(message, section, &mut cancel)
            .await
            .map(|_| ())
            .map_err(|e| format!("cannot download the attachment: {e}"))
    })
}

/// An IMAP engine for `account`, signed in with a fresh credential and kept fresh.
async fn signed_in_imap(
    store: &Arc<SqliteStore>,
    account: &Configured,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<AccountEngine<ImapBackend>, String> {
    let stored = stored_credential(account, secrets.as_ref())
        .await
        .map_err(|failure| failure.why)?;
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

/// Run one IMAP account's outbox now, and nothing else: no fetch, no flag sweep.
///
/// What `mailo import --to-mailbox` runs after queueing its uploads, so they go while the user
/// is watching rather than at the next sync. Blocking, with a runtime of its own, like [`run`].
pub fn drain(
    store: &Arc<SqliteStore>,
    account: AccountId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, String> {
    let registry = clients::load_default().map_err(|e| e.to_string())?;
    drain_with(store, platform_secrets(), &registry, account, now)
}

/// The same, with the secret store named, so a test can run it.
pub fn drain_with(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<SyncReport, String> {
    let account = configured(store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or_else(|| "that account is no longer configured".to_owned())?;
    if !matches!(account.plan.incoming, Incoming::Imap { .. }) {
        return Err("only an IMAP account has mailboxes to upload into".to_owned());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let (_tx, mut cancel) = watch::channel(false);
        let mut engine = signed_in_imap(store, &account, secrets, registry, now).await?;
        engine
            .drain_outbox(&mut cancel, now)
            .await
            .map_err(|e| e.to_string())
    })
}

/// One pass, or passes until the process is stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// `mailo sync`: one pass per account, then return.
    Once,
    /// `mailo watch`: keep going, waiting the way each server prefers to be waited on.
    Watch,
}

/// How long to wait after a pass that failed, before trying again.
///
/// A watch that retried immediately against a server that is down is a client hammering someone
/// else's machine, and the credential rule from F128 applies here too: a rejected sign-in must
/// not be retried in a loop. `Retryable` already decides that per error; this is the floor for
/// everything else.
const AFTER_A_FAILURE: std::time::Duration = std::time::Duration::from_secs(60);

/// Drive one account: a single pass, or a loop that never returns of its own accord.
///
/// The loop is where `AccountEngine::watch` finally gets a caller. It has handled IDLE since
/// phase 3 — and every caller was a test, which F128 found and the window's poll loop was meant
/// to answer. F140 then established that the window's loop never runs, so this is the first
/// place IDLE is actually reachable by a user: a process whose whole job is to stay open.
///
/// How the pass ended is the return; what a watch does between passes goes to `told`.
#[allow(clippy::too_many_arguments)]
pub async fn drive<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    account: &Configured,
    mailboxes: &[MailboxRef],
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    announce: Announce<'_>,
    told: Told<'_>,
) -> PassEnd {
    let result = follow(
        engine, account, mailboxes, cancel, now, mode, announce, None, told,
    )
    .await;
    ended(account, result, None)
}

/// [`drive`], with a failure's classification kept and progress reported.
#[allow(clippy::too_many_arguments)]
async fn follow<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    account: &Configured,
    mailboxes: &[MailboxRef],
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    announce: Announce<'_>,
    emit: Emit<'_>,
    told: Told<'_>,
) -> Result<Done, Failure> {
    if mode == Mode::Once {
        return pass(engine, mailboxes, account.keep, cancel, now, emit).await;
    }

    let poll_every = poll_floor(&account.caps.watch);
    // The inbox only. `IDLE` holds one selected mailbox per connection, and new mail in a
    // followed folder — usually filed there by a server-side rule — waits for the next pass,
    // which every wake-up and every interval runs over all of them.
    let inbox = mailboxes
        .first()
        .cloned()
        .ok_or_else(|| Failure::fatal("nothing to watch"))?;

    loop {
        let at = chrono::Utc::now();
        let report = pass(engine, mailboxes, account.keep, cancel, at, emit).await;
        match after_pass(account, &report, at, announce, told) {
            AfterPass::Stop => return report,
            AfterPass::Hold(wait) => {
                tokio::time::sleep(wait).await;
                continue;
            }
            AfterPass::Wait => {}
        }

        // Then wait the way this server prefers — IDLE, or the poll interval, which is the whole
        // of the waiting for a POP3 account and a safety floor for an IMAP one — or until a send
        // in the outbox comes due, so one scheduled for nine leaves at nine and not whenever the
        // server next has news.
        match engine.wait(&inbox, cancel, poll_every).await {
            Ok(_) => continue,
            Err(e) => waited_in_vain(account, &e, told).await,
        }
    }
}

/// How long a watch sleeps between waits on a server that cannot be waited on, and the floor
/// under one that can: shared by `mailo watch` and the window's live watch ([`live`]).
fn poll_floor(watch: &WatchMode) -> std::time::Duration {
    match watch {
        WatchMode::Poll { every } => *every,
        // An IDLE account still needs a floor: `watch` returns as soon as the server says
        // anything, and a mailbox that is busy would otherwise pass in a tight loop.
        WatchMode::Idle => std::time::Duration::from_secs(30),
    }
}

/// What a watch does once a pass has been reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterPass {
    /// The credential was refused: stop, and wait for the user.
    Stop,
    /// Sleep this long before the next pass, without waiting on the server.
    Hold(std::time::Duration),
    /// Wait the way the server prefers.
    Wait,
}

/// Tell the watch's caller, if there is one, what it did.
fn relay(told: Told<'_>, watched: Watched) {
    if let Some(sink) = told {
        sink(watched);
    }
}

/// Report one pass of a watch, announce what it fetched, and say what to do next.
///
/// Shared by every protocol's loop, so a JMAP watch stops on a refused password and honours a
/// rate limit exactly as an IMAP one does.
fn after_pass(
    account: &Configured,
    report: &Result<Done, Failure>,
    at: chrono::DateTime<chrono::Utc>,
    announce: Announce<'_>,
    told: Told<'_>,
) -> AfterPass {
    relay(told, Watched::Pass(ended(account, report.clone(), None)));
    let done = match report {
        Ok(done) => done,
        Err(_) => return AfterPass::Hold(AFTER_A_FAILURE),
    };
    // After the pass is told, so a notification never announces mail the caller has not yet
    // heard was fetched. A failure here is told and does not stop the watch: mail that cannot be
    // announced is still mail worth fetching.
    if let Announce::To { store, notifier } = announce
        && let Err(why) = crate::notify::announce(
            store,
            account.id.clone(),
            &done.counts.arrived,
            notifier,
            at,
        )
    {
        relay(
            told,
            Watched::AnnounceFailed {
                address: account.address.clone(),
                why,
            },
        );
    }
    // Then the follow-up reminders (`crate::follow_up`): what this pass fetched may be the reply
    // one was waiting for, or the Sent copy a composer's reminder was waiting to join, and one
    // may simply have come due. Only a watch that announces sweeps, so that a reminder coming
    // back is said by whoever brings it back: a quiet watch leaves it to the window.
    if let Announce::To { store, notifier } = announce
        && let Err(why) = crate::follow_up::sweep_and_announce(store, Some(notifier), at)
    {
        relay(told, Watched::RemindersFailed { why });
    }
    // The rule F128 built its loop on, and the reason this one is not a bare sleep: a credential
    // the server has already refused must stop the loop rather than slow it. Sixty seconds of
    // wrong passwords is still a locked account by morning.
    if report::needs_reauth(&done.trouble) {
        return AfterPass::Stop;
    }
    // The server asked. Honour it before doing anything else (F130).
    match report::hold(&done.trouble) {
        Some(wait) => AfterPass::Hold(wait),
        None => AfterPass::Wait,
    }
}

/// A wait that failed is told, and the watch gives the server a minute before it passes again.
async fn waited_in_vain(account: &Configured, e: &impl std::fmt::Display, told: Told<'_>) {
    relay(
        told,
        Watched::WaitFailed {
            address: account.address.clone(),
            why: e.to_string(),
        },
    );
    tokio::time::sleep(AFTER_A_FAILURE).await;
}

/// One whole sync pass, whatever the protocol underneath.
///
/// Shared rather than written once per branch, because it was written once per branch and the
/// two drifted: POP3 fetched headers, then bodies, then drained the outbox, while IMAP stopped
/// after the headers. An IMAP account therefore never downloaded a message body and never sent
/// anything it had queued, and nothing said so — the pass reported success for the part it did.
///
/// `plan.md` phase 5 asks that "the **same** CLI works through the IMAP backend". The surest way
/// to make two paths the same is for there to be one.
async fn pass<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailboxes: &[MailboxRef],
    keep: crate::offline::Keep,
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
            // credential already refused, which is the loop F128 exists to prevent.
            Err(e) if matches!(e.retry(), Retry::NeedsReauth) => {
                let mut done = Done::default();
                done.trouble(None, Retry::NeedsReauth, e.to_string());
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
        one_mailbox(engine, mailbox, keep, cancel, now, &mut done, emit).await;
    }

    // Rules, over what arrived in any folder this pass, before the drain so that what they
    // queue leaves in the same pass. After the bodies, so a rule about words in the body finds
    // them for every message the pass had room to fetch.
    match engine.run_rules(&done.counts.arrived, now) {
        Ok(ran) => done.counts.ruled.extend(ran.acted),
        Err(e) => done.trouble(None, e.retry(), format!("rules: {e}")),
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
        && !report::needs_reauth(&done.trouble)
        && report::hold(&done.trouble).is_none()
    {
        say(emit, Progress::Folders);
        if let Err(e) = engine.refresh_folders(cancel, now).await {
            done.trouble(None, e.retry(), format!("folders: {e}"));
        }
    }
    Ok(done)
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
/// is worth seeing and is not a reason to abandon the mail already in hand.
async fn one_mailbox<B: mail_proto::Backend>(
    engine: &mut AccountEngine<B>,
    mailbox: &MailboxRef,
    keep: crate::offline::Keep,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    done: &mut Done,
    emit: Emit<'_>,
) {
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
            done.trouble(Some(path), e.retry(), format!("{path}: {e}"));
            return;
        }
    };
    done.counts.headers_fetched += one.headers_fetched;
    done.counts.arrived.extend(one.arrived);
    done.notes(Some(path), one.needs_attention);
    say(emit, headers(done_count(one.headers_fetched)));

    // What the server knows and a header fetch does not carry: flags, Gmail's labels, and
    // messages that have gone. `AccountEngine::sweep` has done this since phase 3 and was
    // called from nowhere but its own tests, so every message stayed unread for ever — mail
    // read on a phone stayed bold, unread counts were the size of the mailbox, and labels
    // never arrived, since they ride the same survey.
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

/// Fetch one folder now: its first page of headers, what the server says of them, and bodies.
///
/// For a folder the window opens that no pass fetches — one the user does not follow — so its
/// mail is there to list without waiting for the next pass. A followed folder is fetched by
/// every pass anyway, and asking again here is harmless: what is already held is not fetched
/// twice. List it afterwards with `Filter::InFolder`.
///
/// Blocking, with a runtime of its own, like [`run`] and [`fetch_part`]: its callers are a
/// command and a click handler, which reach it through `spawn_blocking`.
///
/// How the fetch ended is the return, as one account's pass does ([`PassEnd`]); it is never
/// `Cancelled`, because nothing holds a signal to cancel it with. An `Err` is a request there is
/// nothing to fetch for: POP3, which has one mailbox and fetches it every pass, and an account
/// whose folders are labels, where a folder's mail is the mail with that label and arrives with
/// it.
pub fn folder_now(
    store: Arc<SqliteStore>,
    account: AccountId,
    path: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PassEnd, String> {
    let registry = clients::load_default().map_err(|e| e.to_string())?;
    folder_now_with(store, platform_secrets(), &registry, account, path, now)
}

/// The same, with the secret store named, so a test can run it.
pub fn folder_now_with(
    store: Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    registry: &ClientRegistry,
    account: AccountId,
    path: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<PassEnd, String> {
    let account = configured(&store)?
        .into_iter()
        .find(|a| a.id == account)
        .ok_or_else(|| "no such account".to_owned())?;
    match account.plan.incoming {
        Incoming::Pop3 { .. } => {
            return Err("POP3 has one mailbox, and every sync fetches it".to_owned());
        }
        Incoming::Local => {
            return Err(format!(
                "{} is kept on this computer: there is no server to fetch from",
                account.address
            ));
        }
        Incoming::Imap { .. } | Incoming::Graph => {}
        // Every pass fetches the whole account, and a folder's mail arrives with its label.
        Incoming::Jmap { .. } => {
            return Err(format!(
                "on {} folders are labels: every sync fetches the whole account",
                account.address
            ));
        }
    }
    if account.caps.labels == ServerLabels::Supported {
        return Err(format!(
            "on {} folders are labels: a folder's mail arrives with its label",
            account.address
        ));
    }
    let mailbox = MailboxRef {
        account: account.id.clone(),
        path: path.to_owned(),
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    let done = runtime.block_on(async {
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
            let mut engine =
                graph_engine(&store, &account, secrets.clone()).map_err(Failure::fatal)?;
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
            .await;
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
        .await;
        Ok(done)
    });
    Ok(ended(&account, done, None))
}

/// The mailboxes the next pass would fetch, per account, by address.
///
/// Reported by `mailo account list`, because "why is my Sent folder empty" is a question a user
/// asks and the answer is a fact this client knows: only these folders are fetched. Exposed
/// rather than duplicated so a test can ask the code rather than restate its rules —
/// CONVENTIONS §"An assertion that was already true proves nothing", second half.
pub fn mailboxes_by_account(store: &SqliteStore) -> Result<Vec<(String, Vec<String>)>, String> {
    use mail_store::Store as _;
    configured(store)?
        .into_iter()
        .map(|account| {
            let folders = store
                .folders(account.id.clone())
                .map_err(|e| e.to_string())?;
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

/// Whether a pass fetches this folder, beside the inbox and Sent.
///
/// One the user follows (`LSUB`) that can hold mail. An unfollowed one is fetched when it is
/// opened, by [`folder_now`], and not on every pass: a server can list hundreds of them.
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
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
        );
        assert_eq!(username_for(&preset.plan), "s1234567");
    }
}

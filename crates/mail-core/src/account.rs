//! Adding an account, and running one sync pass.
//!
//! This is where the pieces meet a real server, so it is also where the honest boundaries are:
//! a password is read from the environment rather than invented, and an OAuth account says what
//! it still needs rather than pretending to be configured.

use crate::error::CoreError;
use mail_domain::id::new_account_id;
use mail_domain::*;
use mail_runtime::{AccountSecrets, ClientRegistry, clients, tokens};
use mail_store::{NewAccount, SqliteStore};
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_provider::ClientEntry;
use porter_provider::Issuer;
use std::fmt::Write as _;

mod linked;
mod remove;

pub use linked::{
    Linked, Reconciled, find as find_linked, forget, linked as linked_accounts, named_by_segment,
    preset_of, reconcile,
};
pub use remove::{RemoveError, Removed, remove};

/// Servers the user named, for an address the preset table does not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    /// `--imap`: mail is read where it lies, in folders.
    Imap(mail_domain::presets::Manual),
    /// `--pop3`: mail is downloaded from one mailbox, and left there.
    Pop3(mail_domain::presets::ManualPop3),
    /// `--jmap [URL]`: a JMAP server, at the session URL given, or — with none — at
    /// `https://<domain>/.well-known/jmap`, found and confirmed before anything is sent there.
    Jmap {
        session: Option<String>,
        /// `--login NAME`, when the server wants something other than the whole address.
        login: Option<String>,
        /// Which header the secret travels in. The command line parses [`HttpAuth::Basic`]
        /// and [`add`] makes it a bearer token when `MAILO_JMAP_TOKEN` is set;
        /// the window says which it was given.
        ///
        /// [`HttpAuth::Basic`]: mail_domain::HttpAuth::Basic
        auth: mail_domain::HttpAuth,
    },
    /// Found by discovery and confirmed by the user. Never parsed from arguments: the binary
    /// puts it here after asking, so [`add`] can store it like any other.
    Discovered(Box<mail_domain::presets::Preset>),
}

/// How a Microsoft 365 account receives: `--receive imap`, the default, or `--receive graph`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receive {
    Imap,
    /// Through Microsoft Graph, for a tenant that switched IMAP off. Sends through Graph too.
    Graph,
}

/// Configure an account from its address.
///
/// The preset supplies hosts, ports and expected capabilities; the credential comes from the
/// environment and goes straight to the keyring. Nothing about a password is persisted in
/// SQLite, which is the whole reason `Credential` exists as a separate type.
// Each argument is one of the command line's independent answers, passed through as it came.
#[allow(clippy::too_many_arguments)]
pub fn add(
    store: &SqliteStore,
    address: &str,
    manual: Option<&crate::account::Setup>,
    microsoft: bool,
    graph: bool,
    receive: crate::account::Receive,
    saved: &ClientRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, CoreError> {
    // A JMAP account given `MAILO_JMAP_TOKEN` signs in with that token as a bearer; the token
    // then travels where a password would, through `Credentials`, so the window can do the same
    // by naming `HttpAuth::Bearer` itself.
    let token = std::env::var("MAILO_JMAP_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    let bearer;
    let (manual, secret) = match (manual, token) {
        (Some(crate::account::Setup::Jmap { session, login, .. }), Some(token)) => {
            bearer = crate::account::Setup::Jmap {
                session: session.clone(),
                login: login.clone(),
                auth: HttpAuth::Bearer,
            };
            (Some(&bearer), Some(token))
        }
        (manual, _) => (manual, std::env::var("MAILO_PASSWORD").ok()),
    };
    let password = secret.map(crate::password::Password::new);
    add_receiving(
        store,
        address,
        manual,
        microsoft,
        graph,
        receive,
        now,
        Credentials {
            password: password.as_ref(),
            saved,
            secrets: mail_runtime::platform_secrets().as_ref(),
            on_url: &|url| {
                // As `println!` would, but a closed stdout is not worth a panic mid-sign-in.
                let _ = print_signin(url, &mut std::io::stdout().lock());
            },
            signed: None,
        },
    )
}

/// What the command line says when a sign-in needs a browser: the address to open, then that it
/// is waiting. Written to `out` so a test can read it; [`add`] hands it stdout.
fn print_signin(url: &str, out: &mut dyn std::io::Write) -> std::io::Result<()> {
    writeln!(out, "Open this in a browser to sign in:\n\n  {url}\n")?;
    writeln!(out, "Waiting for the redirect…")
}

/// Where [`add_with_password`] gets a credential, and where it keeps one.
pub struct Credentials<'a> {
    /// The password for a password account. `None`, or an empty one, is no password.
    pub password: Option<&'a crate::password::Password>,
    /// The OAuth clients earlier sign-ins recorded.
    pub saved: &'a ClientRegistry,
    /// Where the password or the sign-in's token is put.
    pub secrets: &'a dyn AccountSecrets,
    /// Handed the address to open when the account signs in in a browser, before the sign-in
    /// waits for it. The command line prints it; the window shows it and opens a browser.
    /// Called on the thread the add runs on.
    pub on_url: &'a dyn Fn(&str),
    /// The OAuth credential a sign-in already obtained, for an account that signs in in a browser:
    /// the add window signs in first and asks for the confirmation after, so the browser is not
    /// opened a second time here. `None` signs in here, as the command line does.
    pub signed: Option<&'a Credential>,
}

// By hand: the password's own `Debug` already redacts it, and the credential store has none.
impl std::fmt::Debug for Credentials<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("password", &self.password)
            .finish_non_exhaustive()
    }
}

/// [`add`], with the password handed over rather than read from the environment, and the
/// credential store named.
///
/// The window's Add account sheet calls this: setting `MAILO_PASSWORD` from a running window
/// would mean writing the environment while other threads read it. `add` calls it with the
/// variable's value and the platform keyring, so the command line behaves exactly as before.
/// The password goes to `secrets` and nowhere else — not SQLite, not a file, not the text this
/// returns. (Minting a Graph token for `--send graph` still reads the platform keyring; the
/// window never asks for Graph.)
pub fn add_with_password(
    store: &SqliteStore,
    address: &str,
    manual: Option<&crate::account::Setup>,
    microsoft: bool,
    graph: bool,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<String, CoreError> {
    add_receiving(
        store,
        address,
        manual,
        microsoft,
        graph,
        crate::account::Receive::Imap,
        now,
        credentials,
    )
}

/// [`add_with_password`], with how mail is read named: `--receive graph` for a Microsoft tenant
/// that has switched IMAP off.
#[allow(clippy::too_many_arguments)]
pub fn add_receiving(
    store: &SqliteStore,
    address: &str,
    manual: Option<&crate::account::Setup>,
    microsoft: bool,
    graph: bool,
    receive: crate::account::Receive,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<String, CoreError> {
    let Credentials {
        password,
        saved,
        secrets,
        on_url,
        signed,
    } = credentials;
    // Normalised once, here, and used for the preset, the stored plan and the stored column
    // alike. `known` deliberately keeps the address exactly as typed, and the accounts
    // table deliberately lowercases it — so without this the plan and the column disagree in
    // case, and anything deriving a login name from one gets a different answer than anything
    // deriving it from the other. A mail server that is case-sensitive about the local part
    // then rejects one of them with nothing to explain why.
    let address = address.to_lowercase();
    let preset = match manual {
        // Named explicitly, so no guessing from a domain that says nothing.
        _ if microsoft && receive == crate::account::Receive::Graph => {
            mail_domain::presets::receive_through_graph(mail_domain::presets::microsoft_preset(
                &address, now,
            ))
        }
        _ if microsoft && graph => mail_domain::presets::send_through_graph(
            mail_domain::presets::microsoft_preset(&address, now),
        ),
        _ if microsoft => mail_domain::presets::microsoft_preset(&address, now),
        // Explicit servers win over the table. Someone who names a host means that host, even
        // for a domain a preset happens to cover.
        Some(crate::account::Setup::Imap(manual)) => {
            mail_domain::presets::manual(&address, manual, now)
        }
        Some(crate::account::Setup::Pop3(manual)) => {
            mail_domain::presets::manual_pop3(&address, manual, now)
        }
        // The secret in `credentials.password` is a password sent as HTTP Basic, or a bearer
        // token, as the setup says.
        Some(crate::account::Setup::Jmap {
            session: Some(session),
            login,
            auth,
        }) => {
            let mut preset = mail_domain::presets::jmap(&address, session, *auth);
            if let (Some(name), AuthPlan::Password { username, .. }) =
                (login, &mut preset.plan.auth)
            {
                *username = Username::Literal(name.clone());
            }
            preset
        }
        // Discovery fills the URL in before this runs (`discover::before_add_jmap`); a caller
        // that skipped it gets told how to name the server instead.
        Some(crate::account::Setup::Jmap { session: None, .. }) => {
            return Err(CoreError::NoJmapSession { address });
        }
        // Found by discovery and already shown to the user, who said yes. The address is set
        // again because it is the one normalised here that the stored column will hold.
        Some(crate::account::Setup::Discovered(found)) => {
            let mut preset = (**found).clone();
            preset.plan.address = address.clone();
            preset
        }
        None => match crate::discover::known(&address, now) {
            Some(preset) => preset,
            None => {
                return Err(CoreError::NoPreset { address });
            }
        },
    };

    // An address that is already here keeps its account id, and re-running is not an error.
    //
    // Every message this program prints about a missing credential says to re-run this command:
    // that is how a client id is supplied, and how a password is. It used to fail the second
    // time with `UNIQUE constraint failed: accounts.address` — a raw SQLite error, and a dead
    // end, because no other command finishes a half-configured account either. The id in
    // particular must be the *existing* one: it is the keyring key, so minting a fresh one
    // would orphan a credential already stored and a working account would quietly stop working.
    let existing: Option<AccountId> = store
        .account_by_address(&address)
        .ok()
        .flatten()
        .map(|stored| stored.id);
    let account = existing
        .clone()
        .unwrap_or_else(mail_domain::id::new_account_id);

    // The preset leaves `identities` empty on purpose: minting one needs an `IdentityId` and
    // an `AccountId`, which would make a preset impure and invent an account id no row
    // matches. Creating the account is where both exist, so this is where the default identity
    // is built — and without it nothing can be sent, because a draft names the identity it is
    // from and `mail_mime::build` reads the `From` header out of it.
    let mut plan = preset.plan;
    // Reuse the identity too, where there is one. `mailo signature` writes to that row, and
    // replacing it on a re-run would silently delete a signature the user had set — the sort of
    // loss nobody notices until it has gone out on a week of mail.
    let established: Option<Identity> = existing
        .clone()
        .and_then(|account| default_identity(store, account));
    let identity = established.unwrap_or_else(|| Identity {
        id: IdentityId::generate(),
        account: account.clone(),
        from: Address {
            // No display name. Inventing one from the local part produces "S1234567", and a
            // name the user did not choose is worse than no name: it goes out on every message.
            name: None,
            email: address.clone(),
        },
        reply_to: None,
        signature: None,
        default: IsDefault::Default,
    });
    plan.identities = vec![identity.clone()];

    store
        .upsert_account(&NewAccount {
            id: &account,
            address: &address,
            plan: &plan,
            caps: &preset.expected_caps,
            identities: &plan.identities,
            at: now,
        })
        .map_err(|e| CoreError::cannot("save the account", e))?;

    let mut out = format!(
        "{} {address} as {account}\n",
        if existing.is_some() {
            "updated"
        } else {
            "added"
        }
    );
    // A JMAP bearer token is kept where a password would be.
    let bearer = matches!(
        plan.incoming,
        Incoming::Jmap {
            auth: HttpAuth::Bearer,
            ..
        }
    );
    match &plan.auth {
        // Presets make no such plan: an account of the desktop's accountd is read from it
        // (`linked::reconcile`), never typed in here.
        AuthPlan::Granted { .. } => {
            return Err(
                "this account is the desktop's account service's: add it there, in Add Account"
                    .to_owned(),
            );
        }
        AuthPlan::Password { username, sasl } => {
            let login = username.resolve(&address);
            match password {
                Some(password) if !password.is_empty() => {
                    for purpose in [
                        SecretPurpose::IncomingPassword,
                        SecretPurpose::OutgoingPassword,
                    ] {
                        let key = SecretKey {
                            account: account.clone(),
                            purpose,
                        };
                        let value =
                            Credential::Password(SecretText::new(password.expose().to_owned()));
                        mail_runtime::block_on(secrets.put(&key, &value))
                            .map_err(|e| CoreError::cannot("save the password", e))?;
                    }
                    if bearer {
                        let _ = writeln!(out, "token stored in the keyring");
                    } else {
                        let _ = writeln!(out, "password stored in the keyring for login {login:?}");
                    }
                    // Said here rather than at the first sync, where it arrives as an
                    // authentication failure with nothing to say it was never going to work.
                    if let Some(why) = incoming_host(&plan)
                        .and_then(mail_domain::presets::password_warning)
                        .map(password_warning_words)
                    {
                        let _ = writeln!(out, "\nwarning: {why}");
                    }
                }
                _ => {
                    // Saying what is missing beats a half-configured account that fails later
                    // with a less obvious message.
                    if matches!(plan.incoming, Incoming::Jmap { .. }) {
                        let _ = writeln!(
                            out,
                            "no password stored. Re-run with MAILO_PASSWORD set (the login name \
                             will be {login:?}), or with MAILO_JMAP_TOKEN for a bearer token."
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "no password stored. Re-run with MAILO_PASSWORD set; \
                             the login name will be {login:?} and the server offers {sasl:?}."
                        );
                    }
                }
            }
        }
        AuthPlan::OAuth { issuer, scopes } => match client_for(*issuer, saved) {
            Some((client, typed)) => {
                let credential = match signed {
                    Some(credential) => credential.clone(),
                    None => authorize(&client, scopes, on_url, now)?,
                };
                mail_runtime::block_on(secrets.put(
                    &SecretKey {
                        account: account.clone(),
                        purpose: SecretPurpose::OAuthRefresh,
                    },
                    &credential,
                ))
                .map_err(|e| CoreError::cannot("save the token", e))?;
                // Incoming and outgoing share one OAuth credential: the scopes cover IMAP and
                // SMTP together, and storing it twice would mean refreshing it twice.
                mail_runtime::block_on(secrets.put(
                    &SecretKey {
                        account: account.clone(),
                        purpose: SecretPurpose::IncomingPassword,
                    },
                    &credential,
                ))
                .map_err(|e| CoreError::cannot("save the token", e))?;
                // Minted now rather than at the first send, so a permission the tenant withheld
                // is reported while the user is still at the setup command — not hours later as
                // a draft that will not leave.
                if plan.outgoing == Outgoing::Graph {
                    let reach = tokens::GraphReach::of(&plan);
                    match graph_token(account, &client, reach, now) {
                        Ok(()) if reach == tokens::GraphReach::ReadAndSend => {
                            let _ = writeln!(out, "reading and sending go through Microsoft Graph");
                        }
                        Ok(()) => {
                            let _ = writeln!(out, "sending goes through Microsoft Graph");
                        }
                        Err(why) => {
                            let _ = writeln!(
                                out,
                                "warning: signed in, but Graph would not issue a token for \
                                 sending ({why}). Mail will be received; sending needs Graph's \
                                 Mail.Send permission on the app registration, consented to."
                            );
                        }
                    }
                }
                // A client the registry already has is not recorded again. One typed in the
                // environment is remembered, because renewing an access token an hour from now
                // needs the same client id and nothing else will have it. Without this the
                // account signs in, works, expires, and cannot be renewed — the environment
                // variable that configured it is long gone by then.
                match remember(typed.then_some(&client)) {
                    Ok(None) if !typed => {
                        let _ = writeln!(out, "signed in; token stored in the keyring");
                    }
                    Ok(Some(path)) => {
                        let _ = writeln!(
                            out,
                            "signed in; token stored in the keyring, client id in {}",
                            path.display()
                        );
                    }
                    Ok(None) => {
                        let _ = writeln!(out, "signed in; token stored in the keyring");
                    }
                    // Not fatal: the account works until the token expires, and saying so is
                    // better than discarding a sign-in the user just completed in a browser.
                    Err(why) => {
                        let _ = writeln!(
                            out,
                            "signed in; token stored in the keyring\n\
                             warning: could not record the client id ({why}), so renewing this \
                             sign-in will need MAILO_OAUTH_CLIENT_ID set again"
                        );
                    }
                }
            }
            _ => {
                // Carrying the flags into the suggested command, because the address alone does
                // not reproduce this account: `--microsoft` is precisely the information the
                // preset table does not have, and a re-run without it fails to find any preset
                // at all. Advice that does not work when followed is worse than none.
                let flags = match (microsoft, graph, receive) {
                    (true, _, crate::account::Receive::Graph) => " --microsoft --receive graph",
                    (true, true, _) => " --microsoft --send graph",
                    (true, false, _) => " --microsoft",
                    _ => "",
                };
                // The client id is deployment configuration and cannot be shipped in a source
                // tree, so the honest thing is to say exactly what is missing and how to
                // supply it — not to look configured and fail at first connect.
                let _ = writeln!(
                    out,
                    "this account uses OAuth ({issuer:?}) and needs a client id.\n\
                     \n{where}\n\
                     \nThen re-run:\n\
                     \n  MAILO_OAUTH_CLIENT_ID=… {secret}mailo account add {address}{flags}\n\
                     \nThey are recorded after the first sign-in, so the variables are needed \
                     once.\n\
                     \nScopes it will request: {scopes:?}",
                    // Named, because two bare `{}` fill in source order and these two read
                    // perfectly plausibly the wrong way round.
                    where = where_to_get_one(*issuer),
                    secret = if matches!(issuer, Issuer::Google) {
                        "MAILO_OAUTH_CLIENT_SECRET=… "
                    } else {
                        ""
                    },
                );
            }
        },
    }
    crate::provider::icon::fetch_if_missing(crate::provider::provider(&plan));
    Ok(out)
}

/// The local-only account, created the first time something is kept in it.
///
/// No identity and no credential: it sends nothing and signs in nowhere. Its capabilities are
/// stored like any account's, because everything that lists accounts reads them.
pub fn local(
    store: &SqliteStore,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<AccountId, CoreError> {
    let address = mail_domain::presets::LOCAL_FOLDERS;
    let existing = store
        .account_by_address(address)
        .map_err(|e| CoreError::cannot("read the accounts", e))?;
    if let Some(account) = existing {
        return Ok(account.id);
    }
    let account = new_account_id();
    let preset = mail_domain::presets::local_folders(now);
    store
        .upsert_account(&NewAccount {
            id: &account,
            address,
            plan: &preset.plan,
            caps: &preset.expected_caps,
            identities: &[],
            at: now,
        })
        .map_err(|e| CoreError::cannot("save the local account", e))?;
    Ok(account)
}

/// The OAuth client to sign in with: from the environment, or else the one recorded by an
/// earlier sign-in.
///
/// The recorded one is what makes re-running this command work as its own advice says — to
/// change how an account sends, say — without digging the client id back out of a portal.
///
/// The client, and whether it came from the environment (and so is to be remembered).
fn client_for(issuer: Issuer, saved: &ClientRegistry) -> Option<(ClientEntry, bool)> {
    let from_env = |name| std::env::var(name).ok().filter(|s: &String| !s.is_empty());
    if let Some(client_id) = from_env("MAILO_OAUTH_CLIENT_ID") {
        // Google issues one with every "Desktop app" client and refuses the exchange without
        // it; Microsoft's public clients want none. Read here rather than demanded, so the
        // issuer that does not need one is not asked for it.
        let typed = clients::entry(
            issuer,
            &client_id,
            from_env("MAILO_OAUTH_CLIENT_SECRET").as_deref(),
        );
        return Some((typed, true));
    }
    clients::client(saved, issuer).map(|client| (client.clone(), false))
}

/// The OAuth client an address's sign-in would use for `issuer`: the one typed in the environment,
/// else one the registry holds. `None` is a sign-in that cannot start, and the one thing to say is
/// where to get a client id ([`no_client_id`]).
pub fn oauth_client(issuer: Issuer, saved: &ClientRegistry) -> Option<ClientEntry> {
    client_for(issuer, saved).map(|(client, _)| client)
}

/// The OAuth clients earlier sign-ins recorded, for [`add`] to fall back on.
///
/// Empty in this crate's unit tests. Integration tests link the ordinary library, so this
/// guard does not apply to them; they pass an empty registry to [`crate::cli::run_with_clients`].
/// A test that found a real client id here would open a sign-in and wait on it.
pub fn saved_clients() -> ClientRegistry {
    if cfg!(test) {
        return ClientRegistry::default();
    }
    mail_runtime::clients::load_default().unwrap_or_default()
}

/// Exchange the sign-in's refresh token for a Graph token and keep it as the outgoing credential.
fn graph_token(
    account: AccountId,
    client: &ClientEntry,
    reach: tokens::GraphReach,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(CoreError::NoRuntime)?;
    runtime.block_on(async {
        tokens::graph_token(
            account,
            client,
            reach,
            mail_runtime::platform_secrets().as_ref(),
            now,
        )
        .await
        .map(|_| ())
        .map_err(CoreError::from)
    })
}

/// Record a client typed in the environment in mailo's own `oauth.json` (never porter's
/// `clients.toml`: [`mail_runtime::clients`]), so the account can be renewed later. `None` in,
/// nothing recorded. Returns where it was written, or `None` when this machine has no config
/// directory to write to, which is not a failure, just an installation that will need the
/// variable again.
fn remember(typed: Option<&ClientEntry>) -> Result<Option<std::path::PathBuf>, CoreError> {
    let Some(client) = typed else {
        return Ok(None);
    };
    clients::remember(
        clients::legacy_path().as_deref(),
        client.issuer,
        &client.client_id.0,
        client.client_secret.as_ref().map(|s| s.expose()),
    )
    .map_err(CoreError::from)
}

/// Run the browser sign-in and return the resulting credential.
///
/// Blocking, and deliberately so: this is a one-shot setup command, the user is watching, and
/// there is nothing else for the process to do while they sign in.
fn authorize(
    client: &ClientEntry,
    scopes: &[String],
    on_url: &dyn Fn(&str),
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, CoreError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(CoreError::NoRuntime)?;
    runtime
        .block_on(mail_runtime::authorize::sign_in(
            client,
            scopes,
            on_url,
            porter_core::UnixSeconds(now.timestamp()),
        ))
        .map_err(CoreError::from)
}

/// Accounts, with what each one still needs.
pub fn list(store: &SqliteStore) -> Result<String, CoreError> {
    let accounts = store.list_accounts()?;

    // Which folders each account fetches, so `account list` can answer "why is my Sent folder
    // empty" without the user having to guess. A store that cannot answer is not an error here:
    // this command's job is to list accounts.
    let folders = crate::sync::mailboxes_by_account(store).unwrap_or_default();
    let plans = crate::sync::auth_by_account(store).unwrap_or_default();
    let local = crate::sync::local_accounts(store);

    let secrets = mail_runtime::platform_secrets();
    let mut out = String::new();
    for stored in accounts {
        let (account, address) = (stored.id, stored.address);
        // Asked of the keyring for no reason otherwise: there is no credential to have.
        if local.contains(&account) {
            let _ = writeln!(out, "{address:<28} kept on this computer; nothing to sync");
            continue;
        }
        // An account of the desktop's accountd has no credential here to look for: it is ready
        // when there is a link to it.
        let granted = matches!(plans.get(&address), Some(AuthPlan::Granted { .. }));
        let has_password = if granted {
            secrets.link().is_some()
        } else {
            mail_runtime::block_on(secrets.get(&SecretKey {
                account,
                purpose: SecretPurpose::IncomingPassword,
            }))
            .is_ok()
        };
        // What is missing depends on how the account signs in, and `sync` says so at length.
        // Saying "no credential stored" for an OAuth account reads as "find a password", which
        // is the one thing that will not work — the same contradiction, one line shorter.
        let waiting_on = match plans.get(&address) {
            Some(AuthPlan::OAuth { .. }) => "not signed in",
            Some(AuthPlan::Granted { .. }) => "the desktop's account service is not reachable",
            _ => "no credential stored",
        };
        let _ = writeln!(
            out,
            "{address:<28} {}",
            if has_password { "ready" } else { waiting_on }
        );
        if let Some((_, paths)) = folders.iter().find(|(a, _)| *a == address) {
            let _ = writeln!(out, "{:<28} syncs {}", "", paths.join(", "));
        }
    }
    if out.is_empty() {
        out.push_str("no accounts. Add one with: mailo account add <address>\n");
    }
    Ok(out)
}

/// Where an installed-application client id comes from, per issuer.
///
/// Named rather than left as "register an installed application with the issuer", which is a
/// research task standing between someone and their own mail. A client id is the one thing this
/// program cannot supply — it is registered against the user's account with the issuer, and
/// shipping one in a source tree would mean every user of this client shared an identity and a
/// quota.
///
/// Deliberately names the durable things — the product, the credential type, the consent
/// requirement — and not a path through a menu, because console navigation is rewritten far more
/// often than any of those.
fn where_to_get_one(issuer: Issuer) -> &'static str {
    match issuer {
        Issuer::Google => concat!(
            "Create one in the Google Cloud console (console.cloud.google.com) as an OAuth ",
            "client ID of application type \"Desktop app\", and download its JSON. While the ",
            "consent screen is still in Testing, the address above has to be listed as a test ",
            "user or the sign-in is refused — that is the step most people miss.\n",
            "\nGoogle issues a client *secret* with that client and will not exchange a code ",
            "without it, PKCE or no PKCE, so set MAILO_OAUTH_CLIENT_SECRET as well. Both are ",
            "in the downloaded JSON, as client_id and client_secret."
        ),
        Issuer::Microsoft => concat!(
            "Register an application in the Microsoft Entra admin centre (entra.microsoft.com) ",
            "under App registrations, with a redirect URI of type \"Public client/native\". A ",
            "managed tenant may also require an administrator to consent to the scopes below ",
            "before any sign-in succeeds."
        ),
        // porter names more issuers than mailo reads mail from, and nothing here creates an
        // account that signs in with one.
        _ => "mailo has no mail sign-in through this issuer.",
    }
}

/// The account's default identity, as stored.
///
/// Read back rather than rebuilt so that re-running `account add` keeps whatever the user has
/// since put on it — a signature, a display name — instead of resetting it to the bare address.
fn default_identity(store: &SqliteStore, account: AccountId) -> Option<Identity> {
    store.identities(account).ok()?.into_iter().next()
}

/// The host mail arrives from, whichever protocol that is.
fn incoming_host(plan: &AccountPlan) -> Option<&str> {
    match &plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => Some(host.as_str()),
        // A URL, not a host; and a JMAP sign-in is HTTP authentication, which none of the
        // password warnings (all about IMAP app passwords) are about.
        Incoming::Local | Incoming::Graph | Incoming::Jmap { .. } => None,
    }
}

/// What to tell someone whose account has no credential stored yet.
///
/// Per account, because the answer differs and getting it wrong is not a matter of tone. Every
/// account used to be told to set `MAILO_PASSWORD`. For a password account that is right. For a
/// Google one it is advice that cannot work — Google stopped accepting passwords for IMAP in May
/// 2022 — and following it means a failed sign-in against Google with a credential that was
/// never going to be accepted, which is the hazard this whole project has been careful about.
/// `mailo account add` already said the right thing; `mailo sync` contradicted it, and sync is
/// the command someone runs second.
///
/// `microsoft` carries `--microsoft` into the command, because the address alone does not
/// reproduce a managed-tenant account: that flag is exactly what the preset table cannot work
/// out, and a re-run without it finds no preset at all.
pub fn no_credential(address: &str, auth: &AuthPlan) -> String {
    match auth {
        AuthPlan::OAuth { issuer, .. } => {
            let flag = match issuer {
                Issuer::Microsoft => " --microsoft",
                _ => "",
            };
            format!(
                concat!(
                    "not signed in yet. This account uses OAuth ({issuer:?}), which needs a ",
                    "client id registered with the issuer — a password will not work. Run:\n",
                    "    MAILO_OAUTH_CLIENT_ID=… {secret}mailo account add {address}{flag}"
                ),
                issuer = issuer,
                address = address,
                flag = flag,
                // Google will not exchange a code without the application secret it issued.
                secret = match issuer {
                    Issuer::Google => "MAILO_OAUTH_CLIENT_SECRET=… ",
                    _ => "",
                },
            )
        }
        AuthPlan::Password { .. } => {
            format!("no credential stored. Run:\n    MAILO_PASSWORD=… mailo account add {address}")
        }
        // Not a credential of ours to be missing: the grant is what is wanted.
        AuthPlan::Granted { .. } => format!(
            "the desktop's account service does not let Mail use {address} (any more). Allow it \
             again in Add Account"
        ),
    }
}

/// What an OAuth account that has expired is told when this installation has no client id for
/// its issuer, and so cannot renew it.
///
/// Beside [`no_credential`], which says the same kind of thing about an account that never
/// signed in: the remedy is the same, and so is who needs to read it.
pub fn no_client_id(issuer: Issuer, address: &str) -> String {
    format!(
        concat!(
            "the sign-in has expired and no OAuth client id is configured ",
            "for {:?}. Re-run: MAILO_OAUTH_CLIENT_ID=… mailo account add {}",
        ),
        issuer, address
    )
}

/// What to tell someone about to store a password where the host will not take one.
///
/// Advice, not a refusal. A tenant may have re-enabled something, an app password may exist, and
/// the user knows their own account better than a table does, so this explains and proceeds.
fn password_warning_words(warning: mail_domain::presets::PasswordWarning) -> &'static str {
    use mail_domain::presets::PasswordWarning;
    match warning {
        PasswordWarning::Microsoft365 => {
            "Microsoft 365 turned off password authentication for IMAP, POP and SMTP, so a \
             password will be rejected however it is stored. These mailboxes need OAuth, which \
             is queued in plan.md and not written yet."
        }
        PasswordWarning::Google => {
            "Google stopped accepting account passwords for IMAP and SMTP. An App Password (which \
             needs two-factor authentication switched on) still works here; the account's own \
             password will not."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    fn store() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        (store, dir)
    }

    /// The command line's sign-in prompt is byte for byte what `authorize` printed itself before
    /// the address went through `on_url`: scripts that read it keep working.
    #[test]
    fn the_command_line_prints_the_sign_in_address_as_it_always_has() {
        let url = "https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz";
        let mut out = Vec::new();
        print_signin(url, &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "Open this in a browser to sign in:\n\n  \
             https://accounts.example.test/o/oauth2/auth?client_id=abc&state=xyz\n\n\
             Waiting for the redirect…\n"
        );
    }

    /// A campus server that offers only POP3, and logs in with a student number.
    fn pop3() -> crate::account::Setup {
        crate::account::Setup::Pop3(mail_domain::presets::ManualPop3 {
            pop3_host: "pop.example.edu".to_owned(),
            pop3_port: 995,
            smtp_host: "smtp.example.edu".to_owned(),
            smtp_port: 465,
            login: Some("s1234567".to_owned()),
        })
    }

    /// Re-running `account add` is the documented way to supply a client id or a password —
    /// every message this program prints about a missing credential says to do exactly that.
    /// It used to fail on the second run with `UNIQUE constraint failed: accounts.address`,
    /// a raw SQLite error, leaving the account permanently half-configured and no command able
    /// to finish it. Advice that does not work when followed is worse than none.
    mod adding_an_account_that_is_already_here {
        use super::*;

        fn id_of(store: &SqliteStore, address: &str) -> String {
            store
                .account_by_address(address)
                .unwrap()
                .expect("the account")
                .id
                .to_string()
        }

        #[test]
        fn it_succeeds_and_says_what_it_did() {
            let (store, _dir) = store();
            add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::account::Receive::Imap,
                &ClientRegistry::default(),
                now(),
            )
            .unwrap();
            let out = add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::account::Receive::Imap,
                &ClientRegistry::default(),
                now(),
            )
            .expect("re-running is what every message tells the user to do");
            assert!(
                !out.contains("UNIQUE constraint"),
                "a database error reached the user: {out}"
            );
            assert!(out.contains("someone@gmail.com"), "{out}");
        }

        /// The account id is the keyring key. Minting a fresh one would orphan a credential the
        /// user had already stored, so a working account would silently stop working.
        #[test]
        fn the_account_keeps_its_identity() {
            let (store, _dir) = store();
            add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::account::Receive::Imap,
                &ClientRegistry::default(),
                now(),
            )
            .unwrap();
            let first = id_of(&store, "someone@gmail.com");
            add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::account::Receive::Imap,
                &ClientRegistry::default(),
                now(),
            )
            .unwrap();
            assert_eq!(first, id_of(&store, "someone@gmail.com"));
        }

        /// And there is still exactly one of everything.
        #[test]
        fn nothing_is_duplicated() {
            let (store, _dir) = store();
            for _ in 0..3 {
                add(
                    &store,
                    "someone@gmail.com",
                    None,
                    false,
                    false,
                    crate::account::Receive::Imap,
                    &ClientRegistry::default(),
                    now(),
                )
                .unwrap();
            }
            let accounts: i64 = mail_store::testing::count(&store, "accounts");
            let identities: i64 = mail_store::testing::count(&store, "identities");
            let caps: i64 = mail_store::testing::count(&store, "account_caps");
            assert_eq!((accounts, identities, caps), (1, 1, 1));
        }

        /// A signature is set on the identity by a separate command, and re-running `account
        /// add` must not be the thing that quietly deletes it.
        #[test]
        fn a_signature_already_set_survives() {
            let (store, _dir) = store();
            add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::account::Receive::Imap,
                &ClientRegistry::default(),
                now(),
            )
            .unwrap();
            let account = store.list_accounts().unwrap().remove(0).id;
            let mut mine = store.identities(account).unwrap().remove(0);
            mine.signature = Some("Ada, sent from mailo".to_owned());
            mine.from.name = Some("Ada".to_owned());
            mail_store::testing::seed_identity(&store, &mine);

            add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::account::Receive::Imap,
                &ClientRegistry::default(),
                now(),
            )
            .unwrap();

            let account = store.list_accounts().unwrap().remove(0).id;
            let kept = store.identities(account).unwrap().remove(0);
            assert_eq!(kept.signature.as_deref(), Some("Ada, sent from mailo"));
            assert_eq!(kept.from.name.as_deref(), Some("Ada"));
        }
    }

    #[test]
    fn a_client_id_recorded_by_an_earlier_sign_in_is_used_again() {
        // Re-running `account add` to change how an account sends must not need the client id
        // dug back out of a portal: the first sign-in recorded it.
        let saved = clients::registry_of(vec![clients::entry(
            Issuer::Microsoft,
            "recorded-client",
            None,
        )]);
        if std::env::var_os("MAILO_OAUTH_CLIENT_ID").is_none() {
            let (client, typed) = client_for(Issuer::Microsoft, &saved).unwrap();
            assert_eq!(client.client_id.0, "recorded-client");
            assert!(!typed, "a recorded client is not recorded again");
        }
        assert_eq!(
            client_for(Issuer::Google, &ClientRegistry::default()).is_some(),
            std::env::var_os("MAILO_OAUTH_CLIENT_ID").is_some(),
            "nothing recorded and nothing in the environment is no client"
        );
    }

    #[test]
    fn an_unknown_domain_says_which_are_known() {
        let (store, _dir) = store();
        let err = add(
            &store,
            "someone@example.test",
            None,
            false,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("gmail.com"), "{err}");
        assert!(
            err.contains("--pop3"),
            "and how to name a POP3-only server: {err}"
        );
    }

    #[test]
    fn an_oauth_account_without_a_client_id_says_how_to_supply_one() {
        // Rather than appearing configured and failing at first connect with something less
        // obvious. A client id cannot be shipped in the source tree, and that is worth saying.
        let (store, _dir) = store();
        // With no MAILO_OAUTH_CLIENT_ID set, which is the state anyone starts in. The client
        // id is deployment configuration and cannot be shipped in a source tree, so the useful
        // thing is the exact command to run once they have one.
        let out = add(
            &store,
            "someone@gmail.com",
            None,
            false,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap();
        assert!(out.contains("client id"), "{out}");
        assert!(
            out.contains(
                "MAILO_OAUTH_CLIENT_ID=… MAILO_OAUTH_CLIENT_SECRET=… \
                 mailo account add someone@gmail.com"
            ),
            "it should print the command to re-run: {out}"
        );
        // Google issues a secret with every Desktop-app client and refuses the exchange without
        // it. Naming only the client id is what sent the first real sign-in into
        // `invalid_request: client_secret is missing`.
        assert!(
            out.contains("client_secret"),
            "it should say where the secret comes from: {out}"
        );
        assert!(
            out.contains("https://mail.google.com/"),
            "and the scopes: {out}"
        );
    }

    #[test]
    fn a_password_account_names_the_login_it_resolved() {
        // Some servers log in with a student or staff number, not the address. Getting that
        // wrong is a failed authentication with no explanation, so the CLI says which name it
        // will use.
        let (store, _dir) = store();
        let out = add(
            &store,
            "s1234567@example.edu",
            Some(&pop3()),
            false,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap();
        assert!(out.contains("s1234567"), "{out}");
        assert!(
            !out.contains("s1234567@example.edu\""),
            "the login is the one named: {out}"
        );
    }

    #[test]
    fn adding_an_account_persists_its_plan_and_capabilities() {
        let (store, _dir) = store();
        add(
            &store,
            "s1234567@example.edu",
            Some(&pop3()),
            false,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap();
        let accounts: i64 = mail_store::testing::count(&store, "accounts");
        let caps: i64 = mail_store::testing::count(&store, "account_caps");
        assert_eq!((accounts, caps), (1, 1));
    }

    #[test]
    fn listing_nothing_explains_how_to_add_one() {
        let (store, _dir) = store();
        assert!(list(&store).unwrap().contains("mailo account add"));
    }
}

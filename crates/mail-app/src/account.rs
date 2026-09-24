//! Adding an account, and running one sync pass.
//!
//! This is where the pieces meet a real server, so it is also where the honest boundaries are:
//! a password is read from the environment rather than invented, and an OAuth account says what
//! it still needs rather than pretending to be configured.

use mail_domain::*;
use mail_runtime::{KeyringSecrets, Loopback, OAuthRegistry, Registration, Secrets, signin};
use mail_store::SqliteStore;
use std::fmt::Write as _;

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
    manual: Option<&crate::cli::Setup>,
    microsoft: bool,
    graph: bool,
    receive: crate::cli::Receive,
    saved: &OAuthRegistry,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
    let password = std::env::var("MAILO_PASSWORD")
        .ok()
        .map(crate::password::Password::new);
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
            secrets: &KeyringSecrets,
        },
    )
}

/// Where [`add_with_password`] gets a credential, and where it keeps one.
pub struct Credentials<'a> {
    /// The password for a password account. `None`, or an empty one, is no password.
    pub password: Option<&'a crate::password::Password>,
    /// The OAuth clients earlier sign-ins recorded.
    pub saved: &'a OAuthRegistry,
    /// Where the password or the sign-in's token is put.
    pub secrets: &'a dyn Secrets,
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
    manual: Option<&crate::cli::Setup>,
    microsoft: bool,
    graph: bool,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<String, String> {
    add_receiving(
        store,
        address,
        manual,
        microsoft,
        graph,
        crate::cli::Receive::Imap,
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
    manual: Option<&crate::cli::Setup>,
    microsoft: bool,
    graph: bool,
    receive: crate::cli::Receive,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<String, String> {
    let Credentials {
        password,
        saved,
        secrets,
    } = credentials;
    // Normalised once, here, and used for the preset, the stored plan and the stored column
    // alike. `preset_for` deliberately keeps the address exactly as typed, and the accounts
    // table deliberately lowercases it — so without this the plan and the column disagree in
    // case, and anything deriving a login name from one gets a different answer than anything
    // deriving it from the other. A mail server that is case-sensitive about the local part
    // then rejects one of them with nothing to explain why.
    let address = address.to_lowercase();
    let preset = match manual {
        // Named explicitly, so no guessing from a domain that says nothing.
        _ if microsoft && receive == crate::cli::Receive::Graph => {
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
        Some(crate::cli::Setup::Imap(manual)) => {
            mail_domain::presets::manual(&address, manual, now)
        }
        Some(crate::cli::Setup::Pop3(manual)) => {
            mail_domain::presets::manual_pop3(&address, manual, now)
        }
        // Found by discovery and already shown to the user, who said yes. The address is set
        // again because it is the one normalised here that the stored column will hold.
        Some(crate::cli::Setup::Discovered(found)) => {
            let mut preset = (**found).clone();
            preset.plan.address = address.clone();
            preset
        }
        None => match mail_domain::presets::preset_for(&address, now) {
            Some(preset) => preset,
            None => {
                return Err(format!(
                    "no preset for {address:?}. Either it is one of the known domains \
                     (gmail.com, googlemail.com), or name the servers:\n\n  \
                     mailo account add {address} --imap imap.example.com --smtp smtp.example.com\n\n\
                     Ports default to 993 and 465, both with implicit TLS. A server that offers \
                     only POP3 takes --pop3 in place of --imap (port 995). Add --login NAME if \
                     the server wants something other than the whole address."
                ));
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
        .connection()
        .query_row(
            "SELECT id FROM accounts WHERE address = ?1",
            [&address],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|id| id.parse().ok())
        .map(AccountId::from_uuid);
    let account = existing.unwrap_or_else(AccountId::generate);

    // The preset leaves `identities` empty on purpose: minting one needs an `IdentityId` and
    // an `AccountId`, which would make `preset_for` impure and invent an account id no row
    // matches. Creating the account is where both exist, so this is where the default identity
    // is built — and without it nothing can be sent, because a draft names the identity it is
    // from and `mail_mime::build` reads the `From` header out of it.
    let mut plan = preset.plan;
    // Reuse the identity too, where there is one. `mailo signature` writes to that row, and
    // replacing it on a re-run would silently delete a signature the user had set — the sort of
    // loss nobody notices until it has gone out on a week of mail.
    let established: Option<Identity> =
        existing.and_then(|account| default_identity(store, account));
    let identity = established.unwrap_or_else(|| Identity {
        id: IdentityId::generate(),
        account,
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
    let plan_json =
        serde_json::to_string(&plan).map_err(|e| format!("cannot encode the account plan: {e}"))?;
    let caps_json = serde_json::to_string(&preset.expected_caps)
        .map_err(|e| format!("cannot encode capabilities: {e}"))?;

    store
        .connection()
        .execute(
            // The plan is refreshed — a preset may have learned a better host since — while
            // `created_at` and the id stay as they were.
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(address) DO UPDATE SET plan = excluded.plan",
            rusqlite_params(&[
                &account.to_string(),
                &address,
                &plan_json,
                &now.to_rfc3339(),
            ]),
        )
        .map_err(|e| format!("cannot save the account: {e}"))?;
    store
        .connection()
        .execute(
            // Left alone if it is already there, so a signature and a display name survive.
            "INSERT INTO identities (id, account, from_name, from_email, reply_to, signature,
                 is_default)
             VALUES (?1, ?2, NULL, ?3, NULL, NULL, ?4)
             ON CONFLICT(id) DO NOTHING",
            rusqlite_params(&[
                &identity.id.to_string(),
                &account.to_string(),
                &identity.from.email,
                &serde_json::to_string(&identity.default)
                    .map_err(|e| format!("cannot encode the identity: {e}"))?,
            ]),
        )
        .map_err(|e| format!("cannot save the identity: {e}"))?;
    store
        .connection()
        .execute(
            // Expected capabilities from the preset, which a real connection later replaces.
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(account) DO UPDATE
                 SET caps = excluded.caps, observed_at = excluded.observed_at",
            rusqlite_params(&[&account.to_string(), &caps_json, &now.to_rfc3339()]),
        )
        .map_err(|e| format!("cannot save capabilities: {e}"))?;

    let mut out = format!(
        "{} {address} as {account}\n",
        if existing.is_some() {
            "updated"
        } else {
            "added"
        }
    );
    match &plan.auth {
        AuthPlan::Password { username, sasl } => {
            let login = username.resolve(&address);
            match password {
                Some(password) if !password.is_empty() => {
                    secrets
                        .put(
                            &SecretKey {
                                account,
                                purpose: SecretPurpose::IncomingPassword,
                            },
                            &Credential::Password(password.expose().to_owned()),
                        )
                        .map_err(|e| format!("cannot save the password: {e}"))?;
                    secrets
                        .put(
                            &SecretKey {
                                account,
                                purpose: SecretPurpose::OutgoingPassword,
                            },
                            &Credential::Password(password.expose().to_owned()),
                        )
                        .map_err(|e| format!("cannot save the password: {e}"))?;
                    let _ = writeln!(out, "password stored in the keyring for login {login:?}");
                    // Said here rather than at the first sync, where it arrives as an
                    // authentication failure with nothing to say it was never going to work.
                    if let Some(why) =
                        incoming_host(&plan).and_then(mail_domain::presets::password_warning)
                    {
                        let _ = writeln!(out, "\nwarning: {why}");
                    }
                }
                _ => {
                    // Saying what is missing beats a half-configured account that fails later
                    // with a less obvious message.
                    let _ = writeln!(
                        out,
                        "no password stored. Re-run with MAILO_PASSWORD set; \
                         the login name will be {login:?} and the server offers {sasl:?}."
                    );
                }
            }
        }
        AuthPlan::OAuth { issuer, scopes } => match client_for(*issuer, saved) {
            Some((client_id, client_secret)) => {
                let credential =
                    authorize(*issuer, &client_id, client_secret.as_deref(), scopes, now)?;
                secrets
                    .put(
                        &SecretKey {
                            account,
                            purpose: SecretPurpose::OAuthRefresh,
                        },
                        &credential,
                    )
                    .map_err(|e| format!("cannot save the token: {e}"))?;
                // Incoming and outgoing share one OAuth credential: the scopes cover IMAP and
                // SMTP together, and storing it twice would mean refreshing it twice.
                secrets
                    .put(
                        &SecretKey {
                            account,
                            purpose: SecretPurpose::IncomingPassword,
                        },
                        &credential,
                    )
                    .map_err(|e| format!("cannot save the token: {e}"))?;
                // Minted now rather than at the first send, so a permission the tenant withheld
                // is reported while the user is still at the setup command — not hours later as
                // a draft that will not leave.
                if plan.outgoing == Outgoing::Graph {
                    let registration = Registration::new(*issuer, &client_id)
                        .with_secret(client_secret.as_deref());
                    let reach = signin::GraphReach::of(&plan);
                    match graph_token(account, &registration, reach, now) {
                        Ok(()) if reach == signin::GraphReach::ReadAndSend => {
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
                // Remembered, because renewing an access token an hour from now needs the
                // same client id and nothing else will have it. Without this the account
                // signs in, works, expires, and cannot be renewed — the environment variable
                // that configured it is long gone by then.
                match remember(*issuer, &client_id, client_secret.as_deref()) {
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
                    (true, _, crate::cli::Receive::Graph) => " --microsoft --receive graph",
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
                    secret = if matches!(issuer, OAuthIssuer::Google) {
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
pub fn local(store: &SqliteStore, now: chrono::DateTime<chrono::Utc>) -> Result<AccountId, String> {
    let address = mail_domain::presets::LOCAL_FOLDERS;
    let existing: Option<AccountId> = store
        .connection()
        .query_row(
            "SELECT id FROM accounts WHERE address = ?1",
            [address],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|id| id.parse().ok())
        .map(AccountId::from_uuid);
    if let Some(account) = existing {
        return Ok(account);
    }
    let account = AccountId::generate();
    let preset = mail_domain::presets::local_folders(now);
    let plan_json = serde_json::to_string(&preset.plan)
        .map_err(|e| format!("cannot encode the account plan: {e}"))?;
    let caps_json = serde_json::to_string(&preset.expected_caps)
        .map_err(|e| format!("cannot encode capabilities: {e}"))?;
    store
        .connection()
        .execute(
            "INSERT INTO accounts (id, address, plan, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite_params(&[&account.to_string(), address, &plan_json, &now.to_rfc3339()]),
        )
        .map_err(|e| format!("cannot save the local account: {e}"))?;
    store
        .connection()
        .execute(
            "INSERT INTO account_caps (account, caps, observed_at) VALUES (?1, ?2, ?3)",
            rusqlite_params(&[&account.to_string(), &caps_json, &now.to_rfc3339()]),
        )
        .map_err(|e| format!("cannot save the local account: {e}"))?;
    Ok(account)
}

/// The OAuth client to sign in with: from the environment, or else the one recorded by an
/// earlier sign-in.
///
/// The recorded one is what makes re-running this command work as its own advice says — to
/// change how an account sends, say — without digging the client id back out of a portal.
fn client_for(issuer: OAuthIssuer, saved: &OAuthRegistry) -> Option<(String, Option<String>)> {
    let from_env = |name| std::env::var(name).ok().filter(|s: &String| !s.is_empty());
    if let Some(client_id) = from_env("MAILO_OAUTH_CLIENT_ID") {
        // Google issues one with every "Desktop app" client and refuses the exchange without
        // it; Microsoft's public clients want none. Read here rather than demanded, so the
        // issuer that does not need one is not asked for it.
        return Some((client_id, from_env("MAILO_OAUTH_CLIENT_SECRET")));
    }
    let registration = saved.get(issuer)?;
    Some((
        registration.client_id.clone(),
        registration.client_secret.clone(),
    ))
}

/// The OAuth clients earlier sign-ins recorded, for [`add`] to fall back on.
///
/// Empty in this crate's unit tests. Integration tests link the ordinary library, so this
/// guard does not apply to them; they pass an empty registry to [`crate::cli::run_with_clients`].
/// A test that found a real client id here would open a sign-in and wait on it.
pub fn saved_clients() -> OAuthRegistry {
    if cfg!(test) {
        return OAuthRegistry::default();
    }
    OAuthRegistry::load_default().unwrap_or_default()
}

/// Exchange the sign-in's refresh token for a Graph token and keep it as the outgoing credential.
fn graph_token(
    account: AccountId,
    registration: &Registration,
    reach: signin::GraphReach,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(async {
        let http = signin::http_client().map_err(|e| e.to_string())?;
        signin::graph_token(account, registration, reach, &KeyringSecrets, &http, now)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}

/// Record the client id this account signed in with, so it can be renewed later.
///
/// Returns where it was written, or `None` when this machine has no config directory to write
/// to — which is not a failure, just an installation that will need the variable again.
fn remember(
    issuer: OAuthIssuer,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<Option<std::path::PathBuf>, String> {
    let Some(path) = signin::default_path() else {
        return Ok(None);
    };
    // Loaded and re-saved rather than overwritten, because a second account with a different
    // issuer must not erase the first one's registration.
    let mut registry = OAuthRegistry::load(&path).map_err(|e| e.to_string())?;
    registry.set(Registration::new(issuer, client_id).with_secret(client_secret));
    registry.save(&path).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

/// Run the browser sign-in and return the resulting credential.
///
/// Blocking, and deliberately so: this is a one-shot setup command, the user is watching, and
/// there is nothing else for the process to do while they sign in.
fn authorize(
    issuer: OAuthIssuer,
    client_id: &str,
    client_secret: Option<&str>,
    scopes: &[String],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;

    runtime.block_on(async {
        // Bind first: the redirect URI has to name the port we actually got, and an installed
        // application has no fixed one.
        let listener = Loopback::bind().await.map_err(|e| e.to_string())?;
        let authorization = mail_runtime::oauth::begin(
            issuer,
            client_id,
            client_secret,
            scopes,
            listener.redirect_uri(),
        )
        .map_err(|e| e.to_string())?;

        println!(
            "Open this in a browser to sign in:\n\n  {}\n",
            authorization.url
        );
        println!("Waiting for the redirect…");

        let code = listener
            .wait_for_code(&authorization.pending)
            .await
            .map_err(|e| e.to_string())?;
        // The shared client, which has a timeout: a token endpoint that accepts the connection
        // and then says nothing would otherwise leave the command waiting forever.
        let http = signin::http_client().map_err(|e| e.to_string())?;
        authorization
            .pending
            .exchange(&code, &http, now)
            .await
            .map_err(|e| e.to_string())
    })
}

/// Accounts, with what each one still needs.
pub fn list(store: &SqliteStore) -> Result<String, String> {
    let db = store.connection();
    let mut stmt = db
        .prepare("SELECT id, address FROM accounts ORDER BY created_at")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;

    // Which folders each account fetches, so `account list` can answer "why is my Sent folder
    // empty" without the user having to guess. A store that cannot answer is not an error here:
    // this command's job is to list accounts.
    let folders = crate::sync::mailboxes_by_account(store).unwrap_or_default();
    let plans = crate::sync::auth_by_account(store).unwrap_or_default();
    let local = crate::sync::local_accounts(store);

    let mut out = String::new();
    for row in rows {
        let (id, address) = row.map_err(|e| e.to_string())?;
        let Ok(uuid) = id.parse() else { continue };
        let account = AccountId::from_uuid(uuid);
        // Asked of the keyring for no reason otherwise: there is no credential to have.
        if local.contains(&account) {
            let _ = writeln!(out, "{address:<28} kept on this computer; nothing to sync");
            continue;
        }
        let has_password = KeyringSecrets
            .get(&SecretKey {
                account,
                purpose: SecretPurpose::IncomingPassword,
            })
            .is_ok();
        // What is missing depends on how the account signs in, and `sync` says so at length.
        // Saying "no credential stored" for an OAuth account reads as "find a password", which
        // is the one thing that will not work — the same contradiction, one line shorter.
        let waiting_on = match plans.get(&address) {
            Some(AuthPlan::OAuth { .. }) => "not signed in",
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

/// `rusqlite::params!` over a slice, so the call sites stay readable.
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
fn where_to_get_one(issuer: OAuthIssuer) -> &'static str {
    match issuer {
        OAuthIssuer::Google => concat!(
            "Create one in the Google Cloud console (console.cloud.google.com) as an OAuth ",
            "client ID of application type \"Desktop app\", and download its JSON. While the ",
            "consent screen is still in Testing, the address above has to be listed as a test ",
            "user or the sign-in is refused — that is the step most people miss.\n",
            "\nGoogle issues a client *secret* with that client and will not exchange a code ",
            "without it, PKCE or no PKCE, so set MAILO_OAUTH_CLIENT_SECRET as well. Both are ",
            "in the downloaded JSON, as client_id and client_secret."
        ),
        OAuthIssuer::Microsoft => concat!(
            "Register an application in the Microsoft Entra admin centre (entra.microsoft.com) ",
            "under App registrations, with a redirect URI of type \"Public client/native\". A ",
            "managed tenant may also require an administrator to consent to the scopes below ",
            "before any sign-in succeeds."
        ),
    }
}

/// The account's default identity, as stored.
///
/// Read back rather than rebuilt so that re-running `account add` keeps whatever the user has
/// since put on it — a signature, a display name — instead of resetting it to the bare address.
fn default_identity(store: &SqliteStore, account: AccountId) -> Option<Identity> {
    store
        .connection()
        .query_row(
            "SELECT id, from_name, from_email, reply_to, signature FROM identities
             WHERE account = ?1 ORDER BY is_default DESC LIMIT 1",
            [account.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .ok()
        .and_then(|(id, name, email, reply_to, signature)| {
            Some(Identity {
                id: IdentityId::from_uuid(id.parse().ok()?),
                account,
                from: Address { name, email },
                reply_to: reply_to.and_then(|r| crate::view::parse_addresses(&r).ok()?.pop()),
                signature,
                default: IsDefault::Default,
            })
        })
}

/// The host mail arrives from, whichever protocol that is.
fn incoming_host(plan: &AccountPlan) -> Option<&str> {
    match &plan.incoming {
        Incoming::Imap { host, .. } | Incoming::Pop3 { host, .. } => Some(host.as_str()),
        Incoming::Local | Incoming::Graph => None,
    }
}

fn rusqlite_params<'a>(values: &'a [&'a str]) -> impl rusqlite::Params + 'a {
    rusqlite::params_from_iter(values.iter().copied())
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

    /// A campus server that offers only POP3, and logs in with a student number.
    fn pop3() -> crate::cli::Setup {
        crate::cli::Setup::Pop3(mail_domain::presets::ManualPop3 {
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
                .connection()
                .query_row(
                    "SELECT id FROM accounts WHERE address = ?1",
                    [address],
                    |r| r.get(0),
                )
                .unwrap()
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
                crate::cli::Receive::Imap,
                &OAuthRegistry::default(),
                now(),
            )
            .unwrap();
            let out = add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::cli::Receive::Imap,
                &OAuthRegistry::default(),
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
                crate::cli::Receive::Imap,
                &OAuthRegistry::default(),
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
                crate::cli::Receive::Imap,
                &OAuthRegistry::default(),
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
                    crate::cli::Receive::Imap,
                    &OAuthRegistry::default(),
                    now(),
                )
                .unwrap();
            }
            let db = store.connection();
            let accounts: i64 = db
                .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
                .unwrap();
            let identities: i64 = db
                .query_row("SELECT count(*) FROM identities", [], |r| r.get(0))
                .unwrap();
            let caps: i64 = db
                .query_row("SELECT count(*) FROM account_caps", [], |r| r.get(0))
                .unwrap();
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
                crate::cli::Receive::Imap,
                &OAuthRegistry::default(),
                now(),
            )
            .unwrap();
            store
                .connection()
                .execute(
                    "UPDATE identities SET signature = 'Ada, sent from mailo', from_name = 'Ada'",
                    [],
                )
                .unwrap();

            add(
                &store,
                "someone@gmail.com",
                None,
                false,
                false,
                crate::cli::Receive::Imap,
                &OAuthRegistry::default(),
                now(),
            )
            .unwrap();

            let (signature, name): (Option<String>, Option<String>) = store
                .connection()
                .query_row("SELECT signature, from_name FROM identities", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .unwrap();
            assert_eq!(signature.as_deref(), Some("Ada, sent from mailo"));
            assert_eq!(name.as_deref(), Some("Ada"));
        }
    }

    #[test]
    fn a_client_id_recorded_by_an_earlier_sign_in_is_used_again() {
        // Re-running `account add` to change how an account sends must not need the client id
        // dug back out of a portal: the first sign-in recorded it.
        let mut saved = OAuthRegistry::default();
        saved.set(Registration::new(OAuthIssuer::Microsoft, "recorded-client"));
        if std::env::var_os("MAILO_OAUTH_CLIENT_ID").is_none() {
            assert_eq!(
                client_for(OAuthIssuer::Microsoft, &saved),
                Some(("recorded-client".to_owned(), None))
            );
        }
        assert_eq!(
            client_for(OAuthIssuer::Google, &OAuthRegistry::default()).is_some(),
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
            crate::cli::Receive::Imap,
            &OAuthRegistry::default(),
            now(),
        )
        .unwrap_err();
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
            crate::cli::Receive::Imap,
            &OAuthRegistry::default(),
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
            crate::cli::Receive::Imap,
            &OAuthRegistry::default(),
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
            crate::cli::Receive::Imap,
            &OAuthRegistry::default(),
            now(),
        )
        .unwrap();
        let accounts: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM accounts", [], |r| r.get(0))
            .unwrap();
        let caps: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM account_caps", [], |r| r.get(0))
            .unwrap();
        assert_eq!((accounts, caps), (1, 1));
    }

    #[test]
    fn listing_nothing_explains_how_to_add_one() {
        let (store, _dir) = store();
        assert!(list(&store).unwrap().contains("mailo account add"));
    }
}

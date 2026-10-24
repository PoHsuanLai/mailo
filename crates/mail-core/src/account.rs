//! Adding an account, and running one sync pass.
//!
//! This is where the pieces meet a real server, so it is also where the honest boundaries are:
//! a password is read from the environment rather than invented, and an OAuth account says what
//! it still needs rather than pretending to be configured.
//!
//! What was done comes back as values ([`Added`], [`Listed`]); the words for it are the front-end's.

use crate::environment::Environment;
use crate::error::{CoreError, Logged};
use mail_domain::id::new_account_id;
use mail_domain::*;
use mail_runtime::{AccountSecrets, ClientRegistry, SecretTrouble, clients, tokens};
use mail_store::{NewAccount, SqliteStore};
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_provider::ClientEntry;
use porter_provider::Issuer;

mod advice;
pub mod direct;
pub mod draft;
mod remove;
mod replan;
mod switch;

pub use advice::{no_client_id, no_credential};
pub use direct::{Direct, Mode, SignInError, sign_in_with};
pub use remove::{RemoveError, Removed, remove};
pub use replan::replan;
pub use switch::{graph_offer, switch_to_graph};

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

/// How a Microsoft 365 account receives: `--receive imap`, `--receive graph`, or neither named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receive {
    /// Over IMAP, named so.
    Imap,
    /// Through Microsoft Graph, named so. Sends through Graph too.
    Graph,
    /// Neither named: through Graph for a work or school account that is new here, as
    /// Thunderbird and Evolution set Microsoft 365 up, and as it already does for one that is
    /// here ([`add_receiving`] never moves an account off IMAP unasked).
    Usual,
}

/// What adding an account did, for a front-end to word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// The address as stored: lowercased.
    pub address: String,
    pub account: AccountId,
    /// The address was already here, and kept its account id.
    pub updated: bool,
    pub outcome: Outcome,
}

/// Where the account's credential stands once it is added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The password, or a JMAP bearer token, is in the keyring.
    PasswordStored {
        /// It is a token, not a password.
        bearer: bool,
        /// The name the server is logged in with.
        login: String,
        /// Why the host will probably not take it, when the table knows.
        warning: Option<mail_domain::presets::PasswordWarning>,
    },
    /// No password was given; the account is configured and waits for one.
    PasswordMissing {
        /// A JMAP account, which can also be given a token.
        jmap: bool,
        login: String,
        sasl: Vec<SaslMech>,
    },
    /// The browser sign-in is done and its token is in the keyring.
    SignedIn {
        /// What minting a Graph token for sending came to, for an account that goes through Graph.
        graph: Option<GraphSetup>,
        client: ClientRecord,
    },
    /// The issuer's sign-in cannot start: this installation has no OAuth client id for it.
    NeedsClientId {
        issuer: Issuer,
        scopes: Vec<String>,
        /// The flags that make the same address find the same account again.
        route: MicrosoftRoute,
    },
}

/// What minting an account's Graph token came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphSetup {
    /// Reading and sending both go through Graph.
    ReadAndSend,
    /// Sending goes through Graph; reading does not.
    SendOnly,
    /// Graph would not issue a token. Mail is received; sending needs a permission consented to.
    Refused(String),
}

/// Whether the OAuth client of a sign-in was written down for renewing it later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientRecord {
    /// Nothing was recorded: the client was already known, or there is nowhere to write one.
    NotRecorded,
    /// The client id was written to this file.
    Recorded(std::path::PathBuf),
    /// It could not be written, so renewing the sign-in will need the client id given again.
    Failed(String),
}

/// How the address was added, as far as the Microsoft flags go. The address alone does not
/// reproduce a managed-tenant account, so a front-end that suggests the command to run again
/// needs these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicrosoftRoute {
    /// Not a Microsoft 365 account named as such.
    Other,
    /// `--microsoft`: through Graph.
    Microsoft,
    /// `--microsoft --receive imap`.
    Imap,
    /// `--microsoft --send graph`.
    SendThroughGraph,
    /// `--microsoft --receive graph`.
    ReceiveThroughGraph,
}

/// One account of the list, with what it still needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub address: String,
    pub state: Readiness,
    /// The folders it fetches, where the store could say.
    pub syncs: Option<Vec<String>>,
}

/// Whether an account can sync, and what it waits for if not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// Kept on this computer: nothing to sync, no credential to have.
    Local,
    Ready,
    /// An OAuth account that has not signed in.
    NotSignedIn,
    /// A password account with no password stored.
    NoCredential,
    /// The keyring did not answer (locked, or not reachable), so whether there is a credential
    /// is not known. `why` says which, in plain words.
    KeyringUnreachable {
        why: &'static str,
    },
}

impl crate::mail::AccountOps<'_> {
    /// Configure an account from its address, with the password or token the environment gave.
    /// See [`add`].
    pub async fn add(
        &self,
        address: &str,
        manual: Option<&crate::account::Setup>,
        microsoft: bool,
        graph: bool,
        receive: crate::account::Receive,
        on_url: &dyn Fn(&str),
    ) -> Result<Added, CoreError> {
        let mail = self.0;
        add(
            mail.store(),
            address,
            manual,
            microsoft,
            graph,
            receive,
            mail.environment(),
            mail.secrets().as_ref(),
            &mail.saved_clients(),
            on_url,
            mail.now(),
        )
        .await
    }

    /// Every account, with what each one still needs. See [`list`].
    pub async fn list(&self) -> Result<Vec<Listed>, CoreError> {
        list(self.0.store(), self.0.secrets().as_ref()).await
    }

    /// Remove `account` and everything kept of it here. See [`remove`].
    pub async fn remove(&self, account: AccountId) -> Result<Removed, RemoveError> {
        remove(self.0.store(), self.0.secrets().as_ref(), account).await
    }

    /// The local-only account, created the first time something is kept in it.
    pub fn local(&self) -> Result<AccountId, CoreError> {
        local(self.0.store(), self.0.now())
    }
}

/// Configure an account from its address.
///
/// The preset supplies hosts, ports and expected capabilities; the credential comes from the
/// environment (`env`) and goes straight to `secrets`, the keyring. Nothing about a password is
/// persisted in SQLite, which is the whole reason `Credential` exists as a separate type.
// Each argument is one of the command line's independent answers, passed through as it came.
#[allow(clippy::too_many_arguments)]
pub async fn add(
    store: &SqliteStore,
    address: &str,
    manual: Option<&crate::account::Setup>,
    microsoft: bool,
    graph: bool,
    receive: crate::account::Receive,
    env: &Environment,
    secrets: &dyn AccountSecrets,
    saved: &ClientRegistry,
    on_url: &dyn Fn(&str),
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Added, CoreError> {
    // A JMAP account given `MAILO_JMAP_TOKEN` signs in with that token as a bearer; the token
    // then travels where a password would, through `Credentials`, so the window can do the same
    // by naming `HttpAuth::Bearer` itself.
    let token = env.jmap_token.clone();
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
        (manual, _) => (manual, env.password.clone()),
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
            secrets,
            environment: env,
            on_url,
            signed: None,
        },
    )
    .await
}

/// Where [`add_with_password`] gets a credential, and where it keeps one.
pub struct Credentials<'a> {
    /// The password for a password account. `None`, or an empty one, is no password.
    pub password: Option<&'a crate::password::Password>,
    /// The OAuth clients earlier sign-ins recorded.
    pub saved: &'a ClientRegistry,
    /// Where the password or the sign-in's token is put.
    pub secrets: &'a dyn AccountSecrets,
    /// What the program was started with: the OAuth client typed there is the one that signs in.
    pub environment: &'a Environment,
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
/// environment's value and the platform keyring.
/// The password goes to `secrets` and nowhere else — not SQLite, not a file, not the value this
/// returns. (Minting a Graph token for `--send graph` goes through the same `secrets`.)
pub async fn add_with_password(
    store: &SqliteStore,
    address: &str,
    manual: Option<&crate::account::Setup>,
    microsoft: bool,
    graph: bool,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<Added, CoreError> {
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
    .await
}

/// [`add_with_password`], with how mail is read named: `--receive graph` for a Microsoft tenant
/// that has switched IMAP off.
#[allow(clippy::too_many_arguments)]
pub async fn add_receiving(
    store: &SqliteStore,
    address: &str,
    manual: Option<&crate::account::Setup>,
    microsoft: bool,
    graph: bool,
    receive: crate::account::Receive,
    now: chrono::DateTime<chrono::Utc>,
    credentials: Credentials<'_>,
) -> Result<Added, CoreError> {
    let Credentials {
        password,
        saved,
        secrets,
        environment,
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
        // `--microsoft` names a work or school account, which reads through Graph unless its
        // sending was named too: that is a choice only an IMAP account has.
        _ if microsoft && receive == crate::account::Receive::Usual && !graph => {
            mail_domain::presets::microsoft_for(
                mail_domain::presets::MicrosoftAccount::Organisation,
                &address,
                now,
            )
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
    // So a store that cannot say is a failure, not an account that is not here.
    let stored = store.account_by_address(&address)?;
    let existing: Option<AccountId> = stored.as_ref().map(|stored| stored.id.clone());
    let account = existing
        .clone()
        .unwrap_or_else(mail_domain::id::new_account_id);
    // Signing an IMAP account in again is not a reason to move it to Graph.
    let preset = match stored.and_then(|stored| stored.plan.ok()) {
        Some(plan) => kept_on_imap(plan, preset, receive),
        None => preset,
    };

    // The preset leaves `identities` empty on purpose: minting one needs an `IdentityId` and
    // an `AccountId`, which would make a preset impure and invent an account id no row
    // matches. Creating the account is where both exist, so this is where the default identity
    // is built — and without it nothing can be sent, because a draft names the identity it is
    // from and `mail_mime::build` reads the `From` header out of it.
    let mut plan = preset.plan;
    // Reuse the identity too, where there is one. `mailo signature` writes to that row, and
    // replacing it on a re-run would silently delete a signature the user had set — the sort of
    // loss nobody notices until it has gone out on a week of mail.
    // A store that cannot read it fails the add rather than overwrite what it could not read.
    let established: Option<Identity> = match &existing {
        Some(account) => default_identity(store, account.clone())?,
        None => None,
    };
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
            caps: &unobserved(&preset.expected_caps),
            identities: &plan.identities,
            at: now,
        })
        .map_err(|e| CoreError::cannot("save the account", e))?;

    let updated = existing.is_some();
    // A JMAP bearer token is kept where a password would be.
    let bearer = matches!(
        plan.incoming,
        Incoming::Jmap {
            auth: HttpAuth::Bearer,
            ..
        }
    );
    let outcome = match &plan.auth {
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
                        secrets
                            .put(&key, &value)
                            .await
                            .map_err(|e| CoreError::cannot("save the password", e))?;
                    }
                    Outcome::PasswordStored {
                        bearer,
                        login,
                        // Said here rather than at the first sync, where it arrives as an
                        // authentication failure with nothing to say it was never going to work.
                        warning: incoming_host(&plan)
                            .and_then(mail_domain::presets::password_warning),
                    }
                }
                // Saying what is missing beats a half-configured account that fails later
                // with a less obvious message.
                _ => Outcome::PasswordMissing {
                    jmap: matches!(plan.incoming, Incoming::Jmap { .. }),
                    login,
                    sasl: sasl.clone(),
                },
            }
        }
        AuthPlan::OAuth { issuer, scopes } => match client_for(*issuer, environment, saved) {
            Some((client, typed)) => {
                let credential = match signed {
                    Some(credential) => credential.clone(),
                    None => authorize(&client, scopes, on_url, now).await?,
                };
                secrets
                    .put(
                        &SecretKey {
                            account: account.clone(),
                            purpose: SecretPurpose::OAuthRefresh,
                        },
                        &credential,
                    )
                    .await
                    .map_err(|e| CoreError::cannot("save the token", e))?;
                // Incoming and outgoing share one OAuth credential: the scopes cover IMAP and
                // SMTP together, and storing it twice would mean refreshing it twice.
                secrets
                    .put(
                        &SecretKey {
                            account: account.clone(),
                            purpose: SecretPurpose::IncomingPassword,
                        },
                        &credential,
                    )
                    .await
                    .map_err(|e| CoreError::cannot("save the token", e))?;
                // Minted now rather than at the first send, so a permission the tenant withheld
                // is reported while the user is still at the setup command — not hours later as
                // a draft that will not leave.
                let graph = if plan.outgoing == Outgoing::Graph {
                    let reach = tokens::GraphReach::of(&plan);
                    Some(
                        match graph_token(account.clone(), &client, reach, secrets, now).await {
                            Ok(()) if reach == tokens::GraphReach::ReadAndSend => {
                                GraphSetup::ReadAndSend
                            }
                            Ok(()) => GraphSetup::SendOnly,
                            Err(why) => GraphSetup::Refused(why.to_string()),
                        },
                    )
                } else {
                    None
                };
                // A client the registry already has is not recorded again. One typed in the
                // environment is remembered, because renewing an access token an hour from now
                // needs the same client id and nothing else will have it. Without this the
                // account signs in, works, expires, and cannot be renewed — the environment
                // variable that configured it is long gone by then.
                let client = match remember(typed.then_some(&client)) {
                    Ok(Some(path)) => ClientRecord::Recorded(path),
                    Ok(None) => ClientRecord::NotRecorded,
                    // Not fatal: the account works until the token expires, and saying so is
                    // better than discarding a sign-in the user just completed in a browser.
                    Err(why) => ClientRecord::Failed(why.to_string()),
                };
                Outcome::SignedIn { graph, client }
            }
            // Carrying the flags into the suggested command, because the address alone does
            // not reproduce this account: `--microsoft` is precisely the information the
            // preset table does not have, and a re-run without it fails to find any preset
            // at all. Advice that does not work when followed is worse than none.
            //
            // No client at hand: the registry handed in has none for this issuer, and the
            // environment names none.
            // The honest thing is to say exactly what is missing — not to look configured and
            // fail at first connect.
            _ => Outcome::NeedsClientId {
                issuer: *issuer,
                scopes: scopes.clone(),
                route: match (microsoft, graph, receive) {
                    (true, _, crate::account::Receive::Graph) => {
                        MicrosoftRoute::ReceiveThroughGraph
                    }
                    // `--receive imap --send smtp`, where plain `--microsoft` would read
                    // through Graph.
                    (true, false, crate::account::Receive::Imap) => MicrosoftRoute::Imap,
                    (true, true, _) => MicrosoftRoute::SendThroughGraph,
                    (true, false, _) => MicrosoftRoute::Microsoft,
                    _ => MicrosoftRoute::Other,
                },
            },
        },
    };
    crate::provider::icon::fetch_if_missing(crate::provider::provider(&plan), environment.program)
        .await;
    Ok(Added {
        address,
        account,
        updated,
        outcome,
    })
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

/// `found`, unless the account is already here reading a Microsoft mailbox over IMAP and Graph
/// was not asked for by name: then its own plan, with what `found` expects of the server.
///
/// Discovery now offers a work or school address Graph, so adding such an account again — to
/// sign in again, the way every refusal says to — would otherwise move it off IMAP without a
/// word. Thunderbird does not migrate an account either: a new type is the person's choice,
/// made with [`switch_to_graph`] or `--receive graph`.
fn kept_on_imap(
    stored: AccountPlan,
    found: mail_domain::presets::Preset,
    receive: crate::account::Receive,
) -> mail_domain::presets::Preset {
    let microsoft_imap = matches!(stored.incoming, Incoming::Imap { .. })
        && matches!(
            stored.auth,
            AuthPlan::OAuth {
                issuer: Issuer::Microsoft,
                ..
            }
        );
    if microsoft_imap
        && found.plan.incoming == Incoming::Graph
        && receive != crate::account::Receive::Graph
    {
        let expected_caps = mail_domain::presets::microsoft_preset(
            &stored.address,
            found.expected_caps.observed_at,
        )
        .expected_caps;
        return mail_domain::presets::Preset {
            plan: stored,
            expected_caps,
        };
    }
    found
}

/// Which kind of Microsoft account `address` is: by the tenant `credential` was issued in, when
/// it has been signed in and its token says ([`tokens::tenant_of`]), else by the address's
/// domain ([`crate::discover::microsoft_domain`]).
pub fn microsoft_account_of(
    address: &str,
    credential: Option<&Credential>,
) -> mail_domain::presets::MicrosoftAccount {
    let tenant = credential.and_then(tokens::tenant_of);
    mail_domain::presets::microsoft_account(
        tenant.as_deref(),
        crate::discover::microsoft_domain(address),
    )
}

/// How a Microsoft account at `address` is set up: over Graph for a work or school account, over
/// IMAP and SMTP for a personal one ([`mail_domain::presets::microsoft_for`]).
///
/// The one choice every path that adds a Microsoft account makes, for a caller that may not
/// know which kind it has until the sign-in is done: a sign-in straight in the browser learns
/// the address from the issuer, and the tenant from the token it was given.
pub fn microsoft_preset_for(
    address: &str,
    credential: Option<&Credential>,
    now: chrono::DateTime<chrono::Utc>,
) -> mail_domain::presets::Preset {
    mail_domain::presets::microsoft_for(microsoft_account_of(address, credential), address, now)
}

/// A preset's capabilities as an account is stored with them: an expectation, never an
/// observation.
///
/// Stamped as observed at the epoch, so the first pass asks the server (`caps_are_stale`)
/// instead of believing the preset for a day. Stamped as observed now, a Microsoft 365 account
/// added this morning kept the preset's empty folder roles and `MOVE` absent until tomorrow, and
/// one signed in again had the roles its server had already given wiped back to the guess. The
/// local account is the exception: its capabilities are the truth, with no server to ask.
pub(crate) fn unobserved(expected: &AccountCaps) -> AccountCaps {
    AccountCaps {
        observed_at: chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
        ..expected.clone()
    }
}

/// The OAuth client to sign in with: from the environment, or else the one recorded by an
/// earlier sign-in.
///
/// The recorded one is what makes re-running this command work as its own advice says — to
/// change how an account sends, say — without digging the client id back out of a portal.
///
/// The client, and whether it came from the environment (and so is to be remembered).
fn client_for(
    issuer: Issuer,
    env: &Environment,
    saved: &ClientRegistry,
) -> Option<(ClientEntry, bool)> {
    if let Some(client_id) = env.oauth_client_id.as_deref() {
        // Google issues one with every "Desktop app" client and refuses the exchange without
        // it; Microsoft's public clients want none. Read here rather than demanded, so the
        // issuer that does not need one is not asked for it.
        let typed = clients::entry(issuer, client_id, env.oauth_client_secret.as_deref());
        return Some((typed, true));
    }
    clients::client(saved, issuer).map(|client| (client.clone(), false))
}

/// The OAuth client an address's sign-in would use for `issuer`: the one typed in the environment,
/// else one the registry holds. `None` is a sign-in that cannot start, and the one thing to say is
/// where to get a client id ([`no_client_id`]).
pub fn oauth_client(
    issuer: Issuer,
    env: &Environment,
    saved: &ClientRegistry,
) -> Option<ClientEntry> {
    client_for(issuer, env, saved).map(|(client, _)| client)
}

/// Exchange the sign-in's refresh token for a Graph token and keep it as the outgoing credential.
async fn graph_token(
    account: AccountId,
    client: &ClientEntry,
    reach: tokens::GraphReach,
    secrets: &dyn AccountSecrets,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), CoreError> {
    tokens::graph_token(account, client, reach, secrets, now)
        .await
        .map(|_| ())
        .map_err(CoreError::from)
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
/// Waits for the person, and deliberately so: this is a one-shot setup command, the user is
/// watching, and there is nothing else for the process to do while they sign in. The window's
/// sheet waits on it too, on a thread of its own.
pub async fn authorize(
    client: &ClientEntry,
    scopes: &[String],
    on_url: &dyn Fn(&str),
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Credential, CoreError> {
    mail_runtime::authorize::sign_in(
        client,
        scopes,
        on_url,
        porter_core::UnixSeconds(now.timestamp()),
    )
    .await
    .map_err(CoreError::from)
}

/// Accounts, with what each one still needs.
pub async fn list(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
) -> Result<Vec<Listed>, CoreError> {
    let accounts = store.list_accounts()?;

    // Which folders each account fetches, so a listing can answer "why is my Sent folder
    // empty" without the user having to guess. A store that cannot answer is not an error here:
    // this command's job is to list accounts.
    let folders = crate::sync::mailboxes_by_account(store)
        .or_log_default("account list: the folders each account fetches could not be read");
    let plans = crate::sync::auth_by_account(store)
        .or_log_default("account list: how each account signs in could not be read");
    let local = crate::sync::local_accounts(store);

    let mut out = Vec::new();
    for stored in accounts {
        let (account, address) = (stored.id, stored.address);
        // Asked of the keyring for no reason otherwise: there is no credential to have.
        if local.contains(&account) {
            out.push(Listed {
                address,
                state: Readiness::Local,
                syncs: None,
            });
            continue;
        }
        let read = secrets
            .get(&SecretKey {
                account,
                purpose: SecretPurpose::IncomingPassword,
            })
            .await
            .map(|_| ())
            .map_err(|error| SecretTrouble::of(&error));
        // What is missing depends on how the account signs in, and `sync` says so at length.
        // Saying "no credential stored" for an OAuth account reads as "find a password", which
        // is the one thing that will not work — the same contradiction, one line shorter. A
        // keyring that did not answer is neither: the credential may be there.
        let state = match read {
            Ok(()) => Readiness::Ready,
            Err(SecretTrouble::Unreachable(why)) => Readiness::KeyringUnreachable { why },
            Err(SecretTrouble::Absent) => match plans.get(&address) {
                Some(AuthPlan::OAuth { .. }) => Readiness::NotSignedIn,
                _ => Readiness::NoCredential,
            },
        };
        let syncs = folders
            .iter()
            .find(|(a, _)| *a == address)
            .map(|(_, paths)| paths.clone());
        out.push(Listed {
            address,
            state,
            syncs,
        });
    }
    Ok(out)
}

/// The account's default identity, as stored.
///
/// Read back rather than rebuilt so that re-running `account add` keeps whatever the user has
/// since put on it — a signature, a display name — instead of resetting it to the bare address.
fn default_identity(
    store: &SqliteStore,
    account: AccountId,
) -> Result<Option<Identity>, mail_store::StoreError> {
    Ok(store.identities(account)?.into_iter().next())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support_tests::{now, pop3};

    fn store() -> SqliteStore {
        mail_store::testing::in_memory()
    }

    /// [`super::add`] with nothing in the environment and a store of secrets that is a test's.
    #[allow(clippy::too_many_arguments)]
    fn add(
        store: &SqliteStore,
        address: &str,
        manual: Option<&crate::account::Setup>,
        microsoft: bool,
        graph: bool,
        receive: crate::account::Receive,
        saved: &ClientRegistry,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<Added, CoreError> {
        mail_runtime::block_on(super::add(
            store,
            address,
            manual,
            microsoft,
            graph,
            receive,
            &Environment::default(),
            &porter_secrets::MemorySecrets::default(),
            saved,
            &|url| panic!("a sign-in asked for a browser: {url}"),
            now,
        ))
    }

    /// [`add`] of an address alone, over IMAP, with no client recorded.
    fn add_address(store: &SqliteStore, address: &str) -> Result<Added, CoreError> {
        add(
            store,
            address,
            None,
            false,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
    }

    fn list(store: &SqliteStore) -> Result<Vec<Listed>, CoreError> {
        mail_runtime::block_on(super::list(
            store,
            &porter_secrets::MemorySecrets::default(),
        ))
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

        /// The account id is the keyring key. Minting a fresh one would orphan a credential the
        /// user had already stored, so a working account would silently stop working. And there
        /// is still exactly one of everything.
        #[test]
        fn it_succeeds_says_so_and_keeps_the_one_account() {
            let store = store();
            add_address(&store, "someone@gmail.com").unwrap();
            let first = id_of(&store, "someone@gmail.com");
            for _ in 0..2 {
                let out = add_address(&store, "someone@gmail.com")
                    .expect("re-running is what every message tells the user to do");
                assert!(out.updated, "{out:?}");
                assert_eq!(out.address, "someone@gmail.com");
            }
            assert_eq!(first, id_of(&store, "someone@gmail.com"));
            let accounts: i64 = mail_store::testing::count(&store, "accounts");
            let identities: i64 = mail_store::testing::count(&store, "identities");
            let caps: i64 = mail_store::testing::count(&store, "account_caps");
            assert_eq!((accounts, identities, caps), (1, 1, 1));
        }

        /// A signature is set on the identity by a separate command, and re-running `account
        /// add` must not be the thing that quietly deletes it.
        #[test]
        fn a_signature_already_set_survives() {
            let store = store();
            add_address(&store, "someone@gmail.com").unwrap();
            let account = store.list_accounts().unwrap().remove(0).id;
            let mut mine = store.identities(account).unwrap().remove(0);
            mine.signature = Some("Ada, sent from mailo".to_owned());
            mine.from.name = Some("Ada".to_owned());
            mail_store::testing::seed_identity(&store, &mine);

            add_address(&store, "someone@gmail.com").unwrap();

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
        let none = Environment::default();
        let (client, typed) = client_for(Issuer::Microsoft, &none, &saved).unwrap();
        assert_eq!(client.client_id.0, "recorded-client");
        assert!(!typed, "a recorded client is not recorded again");
        assert!(
            client_for(Issuer::Google, &none, &ClientRegistry::default()).is_none(),
            "nothing recorded and nothing in the environment is no client"
        );
    }

    #[test]
    fn a_client_typed_in_the_environment_wins_and_is_remembered() {
        let saved = clients::registry_of(vec![clients::entry(
            Issuer::Microsoft,
            "recorded-client",
            None,
        )]);
        let typed = Environment {
            oauth_client_id: Some("typed-client".to_owned()),
            ..Environment::default()
        };
        let (client, remember) = client_for(Issuer::Microsoft, &typed, &saved).unwrap();
        assert_eq!(client.client_id.0, "typed-client");
        assert!(remember, "a typed client is recorded for renewing later");
        assert!(oauth_client(Issuer::Google, &typed, &ClientRegistry::default()).is_some());
    }

    #[test]
    fn an_unknown_domain_says_which_are_known() {
        let store = store();
        let err = add_address(&store, "someone@example.test").unwrap_err();
        assert!(err.to_string().contains("gmail.com"), "{err}");
        assert!(
            matches!(err.remedy(), Some(crate::Remedy::NameServers { .. })),
            "and the step is to name the servers: {err}"
        );
    }

    #[test]
    fn with_no_client_at_hand_an_oauth_account_says_how_to_supply_one() {
        // Rather than appearing configured and failing at first connect with something less
        // obvious. No client at hand is a registry without one, handed in on purpose here, and
        // no MAILO_OAUTH_CLIENT_ID: the useful thing then is the
        // exact command to run once they have one.
        let store = store();
        let out = add_address(&store, "someone@gmail.com").unwrap();
        assert!(
            matches!(
                &out.outcome,
                Outcome::NeedsClientId {
                    issuer: Issuer::Google,
                    scopes,
                    route: MicrosoftRoute::Other,
                } if scopes.iter().any(|scope| scope == "https://mail.google.com/")
            ),
            "it should carry what the command line needs to print the command to re-run, and \
             the scopes: {out:?}"
        );
    }

    #[test]
    fn a_password_account_names_the_login_it_resolved() {
        // Some servers log in with a student or staff number, not the address. Getting that
        // wrong is a failed authentication with no explanation, so the CLI says which name it
        // will use.
        let store = store();
        let out = add(
            &store,
            "s1234567@example.edu",
            Some(&pop3(Some("s1234567"))),
            false,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap();
        assert!(
            matches!(
                &out.outcome,
                Outcome::PasswordMissing { login, .. } if login == "s1234567"
            ),
            "the login is the one named: {out:?}"
        );
    }

    #[test]
    fn adding_an_account_persists_its_plan_and_capabilities() {
        let store = store();
        add(
            &store,
            "s1234567@example.edu",
            Some(&pop3(Some("s1234567"))),
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
    fn listing_nothing_is_an_empty_list() {
        let store = store();
        assert!(list(&store).unwrap().is_empty());
    }

    /// What a preset expects of a server is stored as never observed, so the first pass asks.
    ///
    /// Stamped with the moment of adding, a Microsoft 365 account kept the preset's empty folder
    /// roles for its first day — no Sent, no Drafts — and one added again lost the roles its
    /// server had already given.
    #[test]
    fn an_added_accounts_capabilities_are_asked_for_on_the_first_pass() {
        let store = store();
        let address = "someone@contoso.onmicrosoft.com";
        add_address(&store, address).unwrap();
        let id = store
            .account_by_address(address)
            .unwrap()
            .expect("the account")
            .id;
        let caps = crate::sync::caps_of(&store, id).expect("capabilities stored");
        assert_eq!(
            caps.observed_at,
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH
        );
        let expected = microsoft_preset_for(address, None, now()).expected_caps;
        assert_eq!(caps, super::unobserved(&expected), "nothing else changed");
    }

    /// A work or school Microsoft account is added reading and sending through Graph, signed in
    /// for Thunderbird's five Graph permissions; the address alone proves a tenant's.
    #[test]
    fn an_organisations_microsoft_account_is_added_over_graph() {
        let store = store();
        let address = "someone@contoso.onmicrosoft.com";
        let added = add_address(&store, address).unwrap();
        let plan = store
            .account_by_address(address)
            .unwrap()
            .expect("the account")
            .plan
            .unwrap();
        assert_eq!(plan.incoming, Incoming::Graph);
        assert_eq!(plan.outgoing, Outgoing::Graph);
        let AuthPlan::OAuth { issuer, scopes } = &plan.auth else {
            panic!("{:?}", plan.auth)
        };
        assert_eq!(*issuer, Issuer::Microsoft);
        let mut scopes: Vec<&str> = scopes.iter().map(String::as_str).collect();
        scopes.sort_unstable();
        let mut five = mail_domain::presets::GRAPH_SCOPES.to_vec();
        five.sort_unstable();
        assert_eq!(scopes, five);
        // No client id here, so it waits for one, asking for those same five.
        let Outcome::NeedsClientId { scopes: asked, .. } = added.outcome else {
            panic!("{:?}", added.outcome)
        };
        assert_eq!(asked.len(), 5, "{asked:?}");
    }

    /// An account already reading a Microsoft mailbox over IMAP keeps doing so when it is added
    /// again to sign in, though discovery now offers Graph: only asking for Graph moves it.
    #[test]
    fn signing_an_imap_account_in_again_does_not_move_it_to_graph() {
        let store = store();
        let address = "someone@contoso.onmicrosoft.com";
        let incoming = |store: &SqliteStore| {
            store
                .account_by_address(address)
                .unwrap()
                .expect("the account")
                .plan
                .unwrap()
                .incoming
        };
        let again = |receive| {
            add(
                &store,
                address,
                None,
                false,
                false,
                receive,
                &ClientRegistry::default(),
                now(),
            )
            .unwrap()
        };
        add(
            &store,
            address,
            None,
            true,
            false,
            crate::account::Receive::Imap,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap();
        assert!(matches!(incoming(&store), Incoming::Imap { .. }));

        let id = again(crate::account::Receive::Usual).account;
        assert!(
            matches!(incoming(&store), Incoming::Imap { .. }),
            "moved unasked"
        );
        again(crate::account::Receive::Imap);
        assert!(
            matches!(incoming(&store), Incoming::Imap { .. }),
            "moved unasked"
        );

        let moved = add(
            &store,
            address,
            None,
            true,
            false,
            crate::account::Receive::Graph,
            &ClientRegistry::default(),
            now(),
        )
        .unwrap();
        assert_eq!(incoming(&store), Incoming::Graph, "asked for by name");
        assert_eq!(moved.account, id, "the same account");
    }

    /// The kind of Microsoft account follows the tenant a token names over the address's domain,
    /// and the domain when no token says.
    #[test]
    fn a_microsoft_account_is_personal_by_its_tenant_or_else_its_domain() {
        use mail_domain::presets::MicrosoftAccount::{Organisation, Personal};
        let jwt = |tid: &str| {
            use base64::Engine as _;
            let claims = format!(r#"{{"tid":"{tid}"}}"#);
            Credential::OAuth {
                access: SecretText::new(format!(
                    "e30.{}.sig",
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims)
                )),
                refresh: SecretText::new("r".to_owned()),
                expires_at: porter_core::UnixSeconds(0),
                granted: Default::default(),
            }
        };
        let consumer = jwt(mail_domain::presets::MICROSOFT_CONSUMER_TENANT);
        let work = jwt("0b5f3c2e-1d4a-4f6b-9c8d-7e6f5a4b3c2d");
        let cases = [
            ("me@outlook.com", None, Personal),
            ("me@hotmail.com", None, Personal),
            ("me@contoso.example", None, Organisation),
            ("me@contoso.onmicrosoft.com", None, Organisation),
            // A personal account signed in with an address on any domain.
            ("me@example.test", Some(&consumer), Personal),
            ("me@outlook.com", Some(&work), Organisation),
        ];
        for (address, credential, want) in cases {
            assert_eq!(microsoft_account_of(address, credential), want, "{address}");
        }
        assert_eq!(
            microsoft_preset_for("me@outlook.com", None, now())
                .plan
                .incoming,
            mail_domain::presets::microsoft_preset("me@outlook.com", now())
                .plan
                .incoming,
            "a personal account keeps IMAP"
        );
        assert_eq!(
            microsoft_preset_for("me@contoso.example", None, now())
                .plan
                .incoming,
            Incoming::Graph
        );
    }
}

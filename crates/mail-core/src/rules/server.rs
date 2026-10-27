//! `mailo sieve` and `mailo vacation`: rules and an away reply that run on the server.
//!
//! Rules run on the server only where it offers ManageSieve (RFC 5804) — see
//! [`mail_proto::sieve::endpoint`]. Gmail and Microsoft offer none, so on those accounts rules
//! run here.
//!
//! The vacation reply is kept where the account's server keeps one ([`away_server`]): in this
//! client's Sieve script beside the rules; in a JMAP account's `VacationResponse` (RFC 8621
//! §8), which the server sends itself, so a JMAP account's script carries no reply and the two
//! never both answer; or, for an account that reads through Microsoft Graph, in Exchange's
//! automatic replies. Gmail's is reached only through the Gmail API, which this client does not
//! use, so a Gmail account has none. A vacation reply this client sent itself would stop
//! whenever the laptop closed, so there is none of that either.

use crate::error::{CoreError, TimeError};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_domain::{AccountPlan, AuthPlan, DateRange, Incoming, IsDefault, Vacation};
use mail_proto::sieve::{
    Active, NoSieve, Places, SieveJob, SieveOutcome, Takeover, Unmappable, compile, endpoint,
};
use mail_runtime::sieve::{Pushed, SieveAuth};
use mail_runtime::{AccountSecrets, ClientRegistry, RuntimeError};
use mail_store::{SqliteStore, Store};
use porter_core::{Credential, SecretKey, SecretPurpose};
use std::path::PathBuf;
use std::sync::Arc;

/// The name this client's script goes by on a server, and who the script says wrote it.
///
/// One name, owned by this client: a script with any other name was written by someone else,
/// and is never replaced, deactivated or deleted without being told to.
const SCRIPT_NAME: &str = "mailo";

/// What `mailo sieve …` asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SieveCmd {
    /// What the server runs, and how it compares with what this client would put there.
    Status { account: Option<String> },
    /// Compile the account's rules and vacation reply and install them.
    Push {
        account: Option<String>,
        /// `--replace-active`: switch off a script someone else made. Never without being told.
        takeover: Takeover,
    },
}

/// What `mailo vacation …` asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VacationCmd {
    Show {
        account: Option<String>,
    },
    On {
        account: Option<String>,
        subject: String,
        /// Read by `run`, not the parser.
        body_file: PathBuf,
        days: u16,
        /// `--from` and `--until`, as typed: a date, or a date and a time, in the reader's zone.
        from: Option<String>,
        until: Option<String>,
    },
    Off {
        account: Option<String>,
    },
}

/// A date or a date and time, in `zone`, as an instant.
pub fn instant<Tz: TimeZone>(text: &str, zone: &Tz) -> Result<DateTime<Utc>, TimeError> {
    let local = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
        .or_else(|_| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|d| d.and_time(Default::default()))
        })
        .map_err(|_| TimeError::NotADate {
            text: text.to_owned(),
        })?;
    zone.from_local_datetime(&local)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| TimeError::NoSuchInstant {
            text: text.to_owned(),
        })
}

/// The reply as the account would send it: to mail addressed to any of its addresses, from its
/// usual one.
pub fn vacation_for(
    account: &crate::sync::Configured,
    subject: &str,
    body: &str,
    days: u16,
    during: DateRange,
) -> Vacation {
    let mut addresses = vec![account.address.to_lowercase()];
    for identity in &account.plan.identities {
        let address = identity.from.email.to_lowercase();
        if !addresses.contains(&address) {
            addresses.push(address);
        }
    }
    let from = account
        .plan
        .identities
        .iter()
        .find(|i| i.default == IsDefault::Default)
        .map(|i| i.from.email.clone());
    Vacation {
        account: account.id.clone(),
        subject: subject.to_owned(),
        body: body.to_owned(),
        days,
        addresses,
        from,
        during,
    }
}

/// A vacation reply as kept, and whether it is being sent now.
#[derive(Debug, Clone, PartialEq)]
pub struct KeptVacation {
    pub address: String,
    pub vacation: Vacation,
    pub in_effect: bool,
}

/// Where an account's vacation reply is kept on its server, which sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AwayServer {
    /// In this client's Sieve script, beside the rules.
    Sieve,
    /// The account's `VacationResponse`, where JMAP keeps it.
    Jmap,
    /// Exchange's automatic replies, through Microsoft Graph's mailbox settings.
    Graph,
}

/// Where `plan`'s vacation reply goes, or why it has nowhere to go.
pub fn away_server(plan: &AccountPlan) -> Result<AwayServer, NoSieve> {
    match plan.incoming {
        Incoming::Jmap { .. } => Ok(AwayServer::Jmap),
        Incoming::Graph => Ok(AwayServer::Graph),
        _ => endpoint(plan).map(|_| AwayServer::Sieve),
    }
}

/// What pushing to the server came to, after a change that was kept here either way: kept here
/// is not the same as running there.
#[derive(Debug)]
pub enum PushDone {
    Pushed {
        address: String,
        pushed: Pushed,
    },
    /// The reply was set where a JMAP or Graph server keeps one.
    Set {
        address: String,
        on: AwayServer,
    },
    /// The change is kept here; the server did not take it, for this reason.
    NotPushed(CoreError),
}

/// What a vacation command did.
#[derive(Debug)]
pub enum VacationDone {
    Show {
        address: String,
        /// `None` when the account has no reply.
        vacation: Option<KeptVacation>,
    },
    On {
        kept: KeptVacation,
        pushed: PushDone,
    },
    Off {
        address: String,
        /// `None` when the account's server takes no Sieve, so there was nothing to push.
        pushed: Option<PushDone>,
    },
}

/// Where the server's copy of this client's script stands against what would be installed now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptState {
    UpToDate,
    /// Nothing is installed and nothing would be.
    NothingToRun,
    OutOfDate,
}

/// One script on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptSeen {
    pub name: String,
    pub active: bool,
}

/// What the server runs, and how it compares with this client's rules.
#[derive(Debug, Clone, PartialEq)]
pub struct SieveStatus {
    pub address: String,
    pub host: String,
    pub port: u16,
    /// The server's own name for itself, when it gave one.
    pub implementation: Option<String>,
    pub extensions: Vec<String>,
    pub scripts: Vec<ScriptSeen>,
    pub script: ScriptState,
    /// Enabled rules left to this client, by name, with why.
    pub local_only: Vec<(String, Unmappable)>,
}

/// What a sieve command did.
#[derive(Debug)]
pub enum SieveDone {
    Pushed { address: String, pushed: Pushed },
    Status(SieveStatus),
}

impl crate::mail::RuleOps<'_> {
    /// Put the account's rules and vacation reply on its server, as `takeover` allows, and say
    /// what the server did. See [`pushed`].
    pub async fn push(
        &self,
        account: &crate::sync::Configured,
        takeover: Takeover,
    ) -> Result<Pushed, CoreError> {
        let mail = self.0;
        pushed(
            mail.store(),
            mail.secrets().as_ref(),
            account,
            takeover,
            &mail.saved_clients(),
            mail.now(),
        )
        .await
    }

    /// A vacation command, and what it did.
    pub async fn run_vacation(&self, command: &VacationCmd) -> Result<VacationDone, CoreError> {
        let mail = self.0;
        run_vacation(
            mail.store(),
            mail.secrets(),
            command,
            &mail.saved_clients(),
            mail.now(),
        )
        .await
    }

    /// A sieve command, and what it did.
    pub async fn run_sieve(&self, command: &SieveCmd) -> Result<SieveDone, CoreError> {
        let mail = self.0;
        run_sieve(
            mail.store(),
            mail.secrets().as_ref(),
            command,
            &mail.saved_clients(),
            mail.now(),
        )
        .await
    }

    /// Put the account's kept vacation reply where its server keeps one: its Sieve script, its
    /// JMAP `VacationResponse`, or Exchange's automatic replies. See [`put_away`].
    pub async fn put_away(&self, account: &crate::sync::Configured) -> PushDone {
        let mail = self.0;
        put_away(
            mail.store(),
            mail.secrets(),
            account,
            &mail.saved_clients(),
            mail.now(),
        )
        .await
    }
}

/// A vacation command over `store`, signing in through `secrets`.
pub async fn run_vacation(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    command: &VacationCmd,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<VacationDone, CoreError> {
    match command {
        VacationCmd::Show { account } => {
            let account = super::pick(store, account.as_deref())?;
            Ok(VacationDone::Show {
                vacation: store
                    .vacation(account.id)?
                    .map(|v| kept(&account.address, v, now)),
                address: account.address,
            })
        }
        VacationCmd::On {
            account,
            subject,
            body_file,
            days,
            from,
            until,
        } => {
            let account = super::pick(store, account.as_deref())?;
            // Refused before anything is kept: a reply nobody will send is worse than none,
            // because the user believes it is going out.
            away_server(&account.plan).map_err(|why| CoreError::NoSieve {
                address: account.address.clone(),
                why,
            })?;
            let body = std::fs::read_to_string(body_file)
                .map_err(|e| CoreError::cannot(format!("read {}", body_file.display()), e))?;
            let during = DateRange {
                from: from
                    .as_deref()
                    .map(|t| instant(t, &chrono::Local))
                    .transpose()?,
                to: until
                    .as_deref()
                    .map(|t| instant(t, &chrono::Local))
                    .transpose()?,
            };
            if let (Some(a), Some(b)) = (during.from, during.to)
                && b <= a
            {
                return Err(CoreError::UntilBeforeFrom);
            }
            let vacation = vacation_for(&account, subject, &body, *days, during);
            store.put_vacation(account.id.clone(), Some(&vacation), now)?;
            let kept = kept(&account.address, vacation, now);
            let pushed = put_away(store, secrets, &account, saved, now).await;
            Ok(VacationDone::On { kept, pushed })
        }
        VacationCmd::Off { account } => {
            let account = super::pick(store, account.as_deref())?;
            let had = store.vacation(account.id.clone())?.is_some();
            store.put_vacation(account.id.clone(), None, now)?;
            // Off says nothing to a server that was never told: a reply set elsewhere stays.
            let pushed = if off_reaches_server(had, away_server(&account.plan).is_ok()) {
                Some(put_away(store, secrets, &account, saved, now).await)
            } else {
                None
            };
            Ok(VacationDone::Off {
                address: account.address,
                pushed,
            })
        }
    }
}

/// Whether switching the reply off is said to the server: only when a reply was kept here to
/// switch off, and the server keeps one.
fn off_reaches_server(kept_here: bool, server_keeps_one: bool) -> bool {
    kept_here && server_keeps_one
}

/// Put the account's kept reply where its server keeps one, saying so either way: in its Sieve
/// script with the rules, or set where a JMAP or Graph server keeps it ([`set_away`]).
pub async fn put_away(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    account: &crate::sync::Configured,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> PushDone {
    let set = |on, done: Result<(), CoreError>| match done {
        Ok(()) => PushDone::Set {
            address: account.address.clone(),
            on,
        },
        Err(why) => PushDone::NotPushed(why),
    };
    match away_server(&account.plan) {
        Ok(AwayServer::Jmap) => set(AwayServer::Jmap, set_jmap(store, secrets, account).await),
        Ok(AwayServer::Graph) => set(
            AwayServer::Graph,
            set_graph(store, secrets.as_ref(), account, saved, now).await,
        ),
        // An account with no server to keep one has nothing to push to, and the push says why.
        Ok(AwayServer::Sieve) | Err(_) => {
            push_now(
                store,
                secrets.as_ref(),
                account,
                Takeover::Refuse,
                saved,
                now,
            )
            .await
        }
    }
}

/// Why a vacation reply was not set on a server that keeps it.
fn not_set(e: RuntimeError) -> CoreError {
    CoreError::context("the vacation reply was not set", e)
}

/// Make the account's `VacationResponse` the reply kept here, or switch it off when none is.
async fn set_jmap(
    store: &Arc<SqliteStore>,
    secrets: Arc<dyn AccountSecrets>,
    account: &crate::sync::Configured,
) -> Result<(), CoreError> {
    let vacation = store.vacation(account.id.clone())?;
    let mut engine = mail_runtime::JmapEngine::new(
        account.id.clone(),
        account.plan.clone(),
        store.clone(),
        secrets,
    )?;
    engine
        .set_vacation(vacation.as_ref())
        .await
        .map_err(not_set)
}

/// Make the mailbox's automatic replies the reply kept here, or switch them off when none is.
async fn set_graph(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: &crate::sync::Configured,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<(), CoreError> {
    let vacation = store.vacation(account.id.clone())?;
    let AuthPlan::OAuth { issuer, .. } = &account.plan.auth else {
        return Err(not_set(RuntimeError::Secrets(mail_runtime::Failure::said(
            "an account that reads through Microsoft Graph signs in with Microsoft",
        ))));
    };
    let client = mail_runtime::clients::client(saved, *issuer).ok_or_else(|| {
        not_set(RuntimeError::Secrets(mail_runtime::Failure::said(
            "no OAuth client id is configured, so Microsoft Graph cannot be signed in to",
        )))
    })?;
    let reach = mail_runtime::tokens::GraphReach::of(&account.plan);
    let token = mail_runtime::tokens::graph_token(account.id.clone(), client, reach, secrets, now)
        .await
        .map_err(not_set)?;
    let Credential::OAuth { access, .. } = token else {
        return Err(not_set(RuntimeError::Secrets(mail_runtime::Failure::said(
            "the Microsoft Graph credential kept for this account is not a token",
        ))));
    };
    let http = mail_runtime::http::http_client().map_err(not_set)?;
    let body = mail_proto::automatic_replies::automatic_replies(vacation.as_ref(), now);
    mail_runtime::graph::replies::set_automatic_replies(&http, access.expose(), &body)
        .await
        .map_err(not_set)
}

/// Push after a change, saying so either way: kept here is not the same as running there.
async fn push_now(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: &crate::sync::Configured,
    takeover: Takeover,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> PushDone {
    match pushed(store, secrets, account, takeover, saved, now).await {
        Ok(pushed) => PushDone::Pushed {
            address: account.address.clone(),
            pushed,
        },
        Err(why) => PushDone::NotPushed(why),
    }
}

fn kept(address: &str, vacation: Vacation, now: DateTime<Utc>) -> KeptVacation {
    KeptVacation {
        address: address.to_owned(),
        in_effect: vacation.active_at(now),
        vacation,
    }
}

/// A sieve command over `store`, signing in through `secrets`.
pub async fn run_sieve(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    command: &SieveCmd,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<SieveDone, CoreError> {
    match command {
        SieveCmd::Push { account, takeover } => {
            let account = super::pick(store, account.as_deref())?;
            let pushed = pushed(store, secrets, &account, *takeover, saved, now).await?;
            Ok(SieveDone::Pushed {
                address: account.address,
                pushed,
            })
        }
        SieveCmd::Status { account } => {
            let account = super::pick(store, account.as_deref())?;
            status(store, secrets, &account, saved, now)
                .await
                .map(SieveDone::Status)
        }
    }
}

/// The account's own sign-in: the server's ManageSieve takes the same credential as its mail.
async fn auth(
    account: &crate::sync::Configured,
    secrets: &dyn AccountSecrets,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<SieveAuth, CoreError> {
    let stored: Credential = secrets
        .get(&SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::IncomingPassword,
        })
        .await
        .map_err(|e| CoreError::unread_credential(&account.address, &account.plan.auth, &e))?;
    let credential = crate::sync::signed_in(account, stored, secrets, saved, now).await?;
    Ok(SieveAuth {
        username: account.plan.username(),
        credential,
        script_name: SCRIPT_NAME.to_owned(),
    })
}

/// Compile the account's rules and vacation reply and install them, with the account's own
/// sign-in, returning what the server did rather than words about it: the window says it its
/// own way. What a sieve push runs, over the secret store it is given.
pub async fn pushed(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: &crate::sync::Configured,
    takeover: Takeover,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<Pushed, CoreError> {
    let at = endpoint(&account.plan).map_err(|why| CoreError::NoSieve {
        address: account.address.clone(),
        why,
    })?;
    let rules = store.rules(account.id.clone())?;
    let vacation = scripted_vacation(store, account)?;
    let places = Places::from_caps(&account.caps);
    let auth = auth(account, secrets, saved, now).await?;
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    mail_runtime::sieve::push(
        &at,
        &auth,
        &rules,
        vacation.as_ref(),
        &places,
        takeover,
        now,
        &mut cancel,
    )
    .await
    .map_err(|e| CoreError::context(format!("{}:{}", at.host, at.port), e))
}

/// The reply the account's Sieve script carries: the kept one, unless the server keeps the reply
/// elsewhere ([`away_server`]), where a second copy in the script would answer twice.
fn scripted_vacation(
    store: &SqliteStore,
    account: &crate::sync::Configured,
) -> Result<Option<Vacation>, CoreError> {
    if away_server(&account.plan) != Ok(AwayServer::Sieve) {
        return Ok(None);
    }
    Ok(store.vacation(account.id.clone())?)
}

async fn status(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: &crate::sync::Configured,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<SieveStatus, CoreError> {
    let at = endpoint(&account.plan).map_err(|why| CoreError::NoSieve {
        address: account.address.clone(),
        why,
    })?;
    let auth = auth(account, secrets, saved, now).await?;
    let (_tx, mut cancel) = tokio::sync::watch::channel(false);
    let outcome = mail_runtime::sieve::manage(&at, &auth, SieveJob::Status, &mut cancel)
        .await
        .map_err(|e| CoreError::context(format!("{}:{}", at.host, at.port), e))?;
    let SieveOutcome::Status {
        caps,
        scripts,
        ours,
    } = outcome
    else {
        return Err(CoreError::StatusExpected);
    };
    let scripts = scripts
        .iter()
        .map(|script| ScriptSeen {
            name: script.name.clone(),
            active: script.active == Active::Yes,
        })
        .collect();
    let rules = store.rules(account.id.clone())?;
    let vacation = scripted_vacation(store, account)?;
    let compiled = compile(
        &rules,
        vacation.as_ref(),
        &caps.sieve,
        &Places::from_caps(&account.caps),
        SCRIPT_NAME,
        now,
    );
    let script = match (&ours, compiled.is_empty()) {
        (Some(held), _) if *held == compiled.script => ScriptState::UpToDate,
        (None, true) => ScriptState::NothingToRun,
        (Some(_), _) | (None, false) => ScriptState::OutOfDate,
    };
    Ok(SieveStatus {
        address: account.address.clone(),
        host: at.host.clone(),
        port: at.port,
        implementation: caps.implementation,
        extensions: caps.sieve,
        scripts,
        script,
        local_only: compiled.local_only,
    })
}

#[cfg(test)]
mod off_tests {
    use super::off_reaches_server;

    #[test]
    fn off_is_said_to_the_server_only_for_a_reply_kept_here() {
        assert!(off_reaches_server(true, true));
        assert!(
            !off_reaches_server(false, true),
            "a reply set elsewhere stays"
        );
        assert!(!off_reaches_server(true, false));
    }
}

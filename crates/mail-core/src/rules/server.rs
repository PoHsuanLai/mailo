//! `mailo sieve` and `mailo vacation`: rules and an away reply that run on the server.
//!
//! Only where the account's server offers ManageSieve (RFC 5804) — see
//! [`mail_proto::sieve::endpoint`]. Gmail and Microsoft offer none; their own filters and
//! vacation settings are reached through their own APIs, which this client does not use, so on
//! those accounts rules run here and there is no vacation reply. A vacation reply this client
//! sent itself would stop whenever the laptop closed, so there is none of that either.

use crate::error::{CoreError, TimeError};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_domain::{DateRange, IsDefault, Vacation};
use mail_proto::sieve::{
    Active, Places, SieveJob, SieveOutcome, Takeover, Unmappable, compile, endpoint,
};
use mail_runtime::sieve::{Pushed, SieveAuth};
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::{SqliteStore, Store};
use porter_core::{Credential, SecretKey, SecretPurpose};
use std::path::PathBuf;

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

/// What pushing to the server came to, after a change that was kept here either way: kept here
/// is not the same as running there.
#[derive(Debug)]
pub enum PushDone {
    Pushed {
        address: String,
        pushed: Pushed,
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
    /// A vacation command, and what it did.
    pub async fn run_vacation(&self, command: &VacationCmd) -> Result<VacationDone, CoreError> {
        let mail = self.0;
        run_vacation(
            mail.store(),
            mail.secrets().as_ref(),
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
}

/// A vacation command over `store`, signing in through `secrets`.
pub async fn run_vacation(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
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
            endpoint(&account.plan).map_err(|why| CoreError::NoSieve {
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
            let pushed = push_now(store, secrets, &account, Takeover::Refuse, saved, now).await;
            Ok(VacationDone::On { kept, pushed })
        }
        VacationCmd::Off { account } => {
            let account = super::pick(store, account.as_deref())?;
            store.put_vacation(account.id.clone(), None, now)?;
            let pushed = if endpoint(&account.plan).is_ok() {
                Some(push_now(store, secrets, &account, Takeover::Refuse, saved, now).await)
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
    // An account of the desktop's accountd: its relay for the ManageSieve server it lists, and no
    // credential of ours.
    if let Some(grant) = account.plan.grant() {
        let link = secrets
            .link()
            .ok_or_else(|| CoreError::AccountServiceUnreachable {
                address: account.address.clone(),
            })?;
        let endpoint = account
            .plan
            .endpoint(porter_core::Family::Sieve)
            .ok_or_else(|| CoreError::NoManageSieve {
                address: account.address.clone(),
            })?;
        return Ok(SieveAuth {
            username: account.plan.username(),
            credential: Credential::Password(porter_core::SecretText::new("")),
            relay: Some(mail_runtime::sieve::Relay {
                link,
                grant: grant.clone(),
                endpoint: endpoint.clone(),
            }),
            script_name: SCRIPT_NAME.to_owned(),
        });
    }
    let stored: Credential = secrets
        .get(&SecretKey {
            account: account.id.clone(),
            purpose: SecretPurpose::IncomingPassword,
        })
        .await
        .map_err(|_| CoreError::NoCredential {
            address: account.address.clone(),
            auth: account.plan.auth.clone(),
        })?;
    let credential = crate::sync::signed_in(account, stored, secrets, saved, now).await?;
    Ok(SieveAuth {
        username: account.plan.username(),
        credential,
        relay: None,
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
    let vacation = store.vacation(account.id.clone())?;
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
    let vacation = store.vacation(account.id.clone())?;
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

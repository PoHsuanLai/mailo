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
    Active, Deleted, Places, SieveJob, SieveOutcome, Takeover, VacationPlaced, compile, endpoint,
};
use mail_runtime::sieve::{Pushed, SieveAuth};
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::{SqliteStore, Store};
use porter_core::{Credential, SecretKey, SecretPurpose};
use std::fmt::Write as _;
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

impl crate::mail::RuleOps<'_> {
    /// `mailo vacation …`, returning what to print.
    pub async fn run_vacation(&self, command: &VacationCmd) -> Result<String, CoreError> {
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

    /// `mailo sieve …`, returning what to print.
    pub async fn run_sieve(&self, command: &SieveCmd) -> Result<String, CoreError> {
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

/// `mailo vacation …` over `store`, signing in through `secrets`, returning what to print.
pub async fn run_vacation(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    command: &VacationCmd,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    match command {
        VacationCmd::Show { account } => {
            let account = super::pick(store, account.as_deref())?;
            Ok(match store.vacation(account.id)? {
                None => format!("{}: no vacation reply\n", account.address),
                Some(v) => shown(&account.address, &v, now),
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
            let mut out = shown(&account.address, &vacation, now);
            out.push_str(&push_now(store, secrets, &account, Takeover::Refuse, saved, now).await);
            Ok(out)
        }
        VacationCmd::Off { account } => {
            let account = super::pick(store, account.as_deref())?;
            store.put_vacation(account.id.clone(), None, now)?;
            let mut out = format!("{}: vacation reply off\n", account.address);
            if endpoint(&account.plan).is_ok() {
                out.push_str(
                    &push_now(store, secrets, &account, Takeover::Refuse, saved, now).await,
                );
            }
            Ok(out)
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
) -> String {
    match push(store, secrets, account, takeover, saved, now).await {
        Ok(said) => said,
        Err(why) => format!(
            "  kept here, but not on the server yet: {why}\n  `mailo sieve push` tries again\n"
        ),
    }
}

fn shown(address: &str, v: &Vacation, now: DateTime<Utc>) -> String {
    let mut out = format!("{address}: vacation reply {:?}", v.subject);
    let when = |t: DateTime<Utc>| {
        t.with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
            .to_string()
    };
    match (v.during.from, v.during.to) {
        (None, None) => out.push_str(", from now until turned off"),
        (Some(a), None) => out.push_str(&format!(", from {}", when(a))),
        (None, Some(b)) => out.push_str(&format!(", until {}", when(b))),
        (Some(a), Some(b)) => out.push_str(&format!(", {} to {}", when(a), when(b))),
    }
    let _ = write!(out, ", once every {} day(s) per sender", v.days);
    if !v.active_at(now) {
        out.push_str(" (not in effect now)");
    }
    out.push('\n');
    out
}

/// `mailo sieve …` over `store`, signing in through `secrets`, returning what to print.
pub async fn run_sieve(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    command: &SieveCmd,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    match command {
        SieveCmd::Push { account, takeover } => {
            let account = super::pick(store, account.as_deref())?;
            push(store, secrets, &account, *takeover, saved, now).await
        }
        SieveCmd::Status { account } => {
            let account = super::pick(store, account.as_deref())?;
            status(store, secrets, &account, saved, now).await
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

async fn push(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: &crate::sync::Configured,
    takeover: Takeover,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
    pushed(store, secrets, account, takeover, saved, now)
        .await
        .map(|pushed| said(&account.address, &pushed))
}

/// Compile the account's rules and vacation reply and install them, with the account's own
/// sign-in, returning what the server did rather than words about it: the window says it its
/// own way. What `mailo sieve push` runs, over the secret store it is given.
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

/// What a push did, for a person.
pub fn said(address: &str, pushed: &Pushed) -> String {
    let mut out = String::new();
    match &pushed.outcome {
        SieveOutcome::Installed { displaced, .. } => {
            let _ = writeln!(
                out,
                "{address}: the server now runs {} rule(s){}",
                pushed.compiled.mapped.len(),
                match pushed.compiled.vacation {
                    VacationPlaced::Dated | VacationPlaced::Undated => " and the vacation reply",
                    _ => "",
                }
            );
            if let Some(theirs) = displaced {
                let _ = writeln!(
                    out,
                    "  {theirs:?} is no longer active; it is still on the server"
                );
            }
        }
        SieveOutcome::Refused { active, .. } => {
            let _ = writeln!(
                out,
                "{address}: nothing installed. The server runs a script called {active:?}, made \
                 elsewhere, and a server runs one script at a time. Merge its rules into \
                 `mailo rules`, then `mailo sieve push --replace-active`"
            );
        }
        SieveOutcome::Removed { deleted, .. } => {
            let _ = writeln!(
                out,
                "{address}: nothing here for the server to run{}",
                match deleted {
                    Deleted::Ours => "; this client's script is taken down",
                    Deleted::NothingThere => "",
                }
            );
        }
        SieveOutcome::Status { .. } => {}
    }
    for (name, why) in &pushed.compiled.local_only {
        let _ = writeln!(out, "  {name:?} runs in this client only: {why}");
    }
    match pushed.compiled.vacation {
        VacationPlaced::Undated => out.push_str(
            "  the server cannot test dates, so the reply is on until a push after its end \
             (`mailo sieve push`, or `vacation off`)\n",
        ),
        VacationPlaced::Outside => out.push_str(
            "  the reply is outside its dates and the server cannot test them: it is left out, \
             and a push during them puts it in\n",
        ),
        VacationPlaced::Unsupported => {
            out.push_str("  the server's Sieve has no vacation extension: no reply is sent\n");
        }
        VacationPlaced::Absent | VacationPlaced::Dated => {}
    }
    out
}

async fn status(
    store: &SqliteStore,
    secrets: &dyn AccountSecrets,
    account: &crate::sync::Configured,
    saved: &ClientRegistry,
    now: DateTime<Utc>,
) -> Result<String, CoreError> {
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
    let mut out = format!(
        "{}: ManageSieve at {}:{}{}\n  extensions: {}\n",
        account.address,
        at.host,
        at.port,
        caps.implementation
            .map(|i| format!(" ({i})"))
            .unwrap_or_default(),
        caps.sieve.join(" ")
    );
    if scripts.is_empty() {
        out.push_str("  no scripts\n");
    }
    for script in &scripts {
        let _ = writeln!(
            out,
            "  script {:?}{}",
            script.name,
            if script.active == Active::Yes {
                " (active)"
            } else {
                ""
            }
        );
    }
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
    let current = match (&ours, compiled.is_empty()) {
        (Some(held), _) if *held == compiled.script => "up to date",
        (None, true) => "up to date: nothing to run there",
        (Some(_), _) | (None, false) => {
            "out of date: `mailo sieve push` installs the rules as they are now"
        }
    };
    let _ = writeln!(out, "  this client's script: {current}");
    for (name, why) in &compiled.local_only {
        let _ = writeln!(out, "  {name:?} runs in this client only: {why}");
    }
    Ok(out)
}

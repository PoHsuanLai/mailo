//! `mailo sieve` and `mailo vacation`: rules and an away reply that run on the server.
//!
//! Only where the account's server offers ManageSieve (RFC 5804) — see
//! [`mail_proto::sieve::endpoint`]. Gmail and Microsoft offer none; their own filters and
//! vacation settings are reached through their own APIs, which this client does not use, so on
//! those accounts rules run here and there is no vacation reply. A vacation reply this client
//! sent itself would stop whenever the laptop closed, so there is none of that either.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use mail_domain::{Credential, DateRange, IsDefault, SecretKey, SecretPurpose, Vacation};
use mail_proto::sieve::{
    Active, Deleted, Places, SieveJob, SieveOutcome, Takeover, VacationPlaced, compile, endpoint,
};
use mail_runtime::sieve::{Pushed, SieveAuth};
use mail_runtime::{KeyringSecrets, OAuthRegistry, Secrets};
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;
use std::path::PathBuf;

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

pub fn parse_sieve(args: &[String]) -> Result<SieveCmd, String> {
    let mut account = None;
    let mut takeover = Takeover::Refuse;
    let mut words = args.get(1..).unwrap_or_default().iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--account" => {
                account = Some(words.next().ok_or("--account needs an address")?.clone());
            }
            "--replace-active" => takeover = Takeover::Replace,
            other => return Err(format!("unexpected {other:?}\n\n{}", crate::cli::usage())),
        }
    }
    match args.first().map(String::as_str) {
        None | Some("status") if takeover == Takeover::Refuse => Ok(SieveCmd::Status { account }),
        Some("push") => Ok(SieveCmd::Push { account, takeover }),
        _ => Err(crate::cli::usage()),
    }
}

pub fn parse_vacation(args: &[String]) -> Result<VacationCmd, String> {
    let mut account = None;
    let mut subject = None;
    let mut body_file = None;
    let mut days = Vacation::DEFAULT_DAYS;
    let (mut from, mut until) = (None, None);
    let mut words = args.get(1..).unwrap_or_default().iter();
    let value = |flag: &str, words: &mut std::slice::Iter<'_, String>| {
        words
            .next()
            .cloned()
            .ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(word) = words.next() {
        match word.as_str() {
            "--account" => account = Some(value("--account", &mut words)?),
            "--subject" => subject = Some(value("--subject", &mut words)?),
            "--body-file" => body_file = Some(PathBuf::from(value("--body-file", &mut words)?)),
            "--days" => {
                days = value("--days", &mut words)?
                    .parse()
                    .ok()
                    .filter(|d| *d > 0)
                    .ok_or("--days needs a whole number of days, at least one")?;
            }
            "--from" => from = Some(value("--from", &mut words)?),
            "--until" => until = Some(value("--until", &mut words)?),
            other => return Err(format!("unexpected {other:?}\n\n{}", crate::cli::usage())),
        }
    }
    match args.first().map(String::as_str) {
        None | Some("show") => Ok(VacationCmd::Show { account }),
        Some("off") => Ok(VacationCmd::Off { account }),
        Some("on") => Ok(VacationCmd::On {
            account,
            subject: subject.ok_or("vacation on needs --subject")?,
            body_file: body_file.ok_or("vacation on needs --body-file with the reply's text")?,
            days,
            from,
            until,
        }),
        Some(other) => Err(format!(
            "vacation does not know {other:?}\n\n{}",
            crate::cli::usage()
        )),
    }
}

/// A date or a date and time, in `zone`, as an instant.
pub fn instant<Tz: TimeZone>(text: &str, zone: &Tz) -> Result<DateTime<Utc>, String> {
    let local = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M"))
        .or_else(|_| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d").map(|d| d.and_time(Default::default()))
        })
        .map_err(|_| format!("{text:?} is not a date: write 2026-10-08 or 2026-10-08T09:00"))?;
    zone.from_local_datetime(&local)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .ok_or_else(|| format!("{text} does not exist in this time zone"))
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
        account: account.id,
        subject: subject.to_owned(),
        body: body.to_owned(),
        days,
        addresses,
        from,
        during,
    }
}

pub fn run_vacation(
    store: &SqliteStore,
    command: &VacationCmd,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let failed = |e: mail_store::StoreError| e.to_string();
    match command {
        VacationCmd::Show { account } => {
            let account = super::pick(store, account.as_deref())?;
            Ok(match store.vacation(account.id).map_err(failed)? {
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
            endpoint(&account.plan).map_err(|why| format!("{}: {why}", account.address))?;
            let body = std::fs::read_to_string(body_file)
                .map_err(|e| format!("cannot read {}: {e}", body_file.display()))?;
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
                return Err("--until has to be after --from".to_owned());
            }
            let vacation = vacation_for(&account, subject, &body, *days, during);
            store
                .put_vacation(account.id, Some(&vacation), now)
                .map_err(failed)?;
            let mut out = shown(&account.address, &vacation, now);
            out.push_str(&push_now(store, &account, Takeover::Refuse, saved, now));
            Ok(out)
        }
        VacationCmd::Off { account } => {
            let account = super::pick(store, account.as_deref())?;
            store.put_vacation(account.id, None, now).map_err(failed)?;
            let mut out = format!("{}: vacation reply off\n", account.address);
            if endpoint(&account.plan).is_ok() {
                out.push_str(&push_now(store, &account, Takeover::Refuse, saved, now));
            }
            Ok(out)
        }
    }
}

/// Push after a change, saying so either way: kept here is not the same as running there.
fn push_now(
    store: &SqliteStore,
    account: &crate::sync::Configured,
    takeover: Takeover,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> String {
    match push(store, account, takeover, saved, now) {
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

pub fn run_sieve(
    store: &SqliteStore,
    command: &SieveCmd,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    match command {
        SieveCmd::Push { account, takeover } => {
            let account = super::pick(store, account.as_deref())?;
            push(store, &account, *takeover, saved, now)
        }
        SieveCmd::Status { account } => {
            let account = super::pick(store, account.as_deref())?;
            status(store, &account, saved, now)
        }
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot reach the server: {e}"))
}

/// The account's own sign-in: the server's ManageSieve takes the same credential as its mail.
async fn auth(
    account: &crate::sync::Configured,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<SieveAuth, String> {
    let secrets = KeyringSecrets;
    let stored: Credential = secrets
        .get(&SecretKey {
            account: account.id,
            purpose: SecretPurpose::IncomingPassword,
        })
        .map_err(|_| crate::view::no_credential(&account.address, &account.plan.auth))?;
    let credential = crate::sync::signed_in(account, stored, &secrets, saved, now).await?;
    Ok(SieveAuth {
        username: account.plan.username(),
        credential,
    })
}

fn push(
    store: &SqliteStore,
    account: &crate::sync::Configured,
    takeover: Takeover,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    pushed(store, account, takeover, saved, now).map(|pushed| said(&account.address, &pushed))
}

/// Compile the account's rules and vacation reply and install them, with the account's own
/// sign-in, returning what the server did rather than words about it: the window says it its
/// own way. What `mailo sieve push` runs.
pub(crate) fn pushed(
    store: &SqliteStore,
    account: &crate::sync::Configured,
    takeover: Takeover,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<Pushed, String> {
    let at = endpoint(&account.plan).map_err(|why| format!("{}: {why}", account.address))?;
    let rules = store.rules(account.id).map_err(|e| e.to_string())?;
    let vacation = store.vacation(account.id).map_err(|e| e.to_string())?;
    let places = Places::from_caps(&account.caps);
    runtime()?.block_on(async {
        let auth = auth(account, saved, now).await?;
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
        .map_err(|e| format!("{}:{}: {e}", at.host, at.port))
    })
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

fn status(
    store: &SqliteStore,
    account: &crate::sync::Configured,
    saved: &OAuthRegistry,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let at = endpoint(&account.plan).map_err(|why| format!("{}: {why}", account.address))?;
    let outcome = runtime()?.block_on(async {
        let auth = auth(account, saved, now).await?;
        let (_tx, mut cancel) = tokio::sync::watch::channel(false);
        mail_runtime::sieve::manage(&at, &auth, SieveJob::Status, &mut cancel)
            .await
            .map_err(|e| format!("{}:{}: {e}", at.host, at.port))
    })?;
    let SieveOutcome::Status {
        caps,
        scripts,
        ours,
    } = outcome
    else {
        return Err("the server answered a status request with something else".to_owned());
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
    let rules = store.rules(account.id).map_err(|e| e.to_string())?;
    let vacation = store.vacation(account.id).map_err(|e| e.to_string())?;
    let compiled = compile(
        &rules,
        vacation.as_ref(),
        &caps.sieve,
        &Places::from_caps(&account.caps),
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

//! The account's rules and vacation reply compiled to Sieve and installed on its server, as
//! `mailo sieve push` does: what every save on the Rules page ends with.
//!
//! What pushes is a context, [`Pusher`], so a test can answer for the server; the real one signs
//! in the way the command line does (`rules::server::pushed`). It runs on a blocking thread. A
//! server with no ManageSieve is not asked.

use chrono::{DateTime, Utc};
use mail_client::Client;
use mail_client::Pushed;
use mail_domain::AccountPlan;
use std::sync::Arc;

use super::super::data::AccountRow;
use mail_client::sync::Configured;

/// Install an account's script: [`mail_client::RuleOps::push`], with the takeover decided. Given the
/// client by `Arc`, which is what a `Mail` is made over.
pub(in crate::ui) type Push =
    Arc<dyn Fn(&Client, &Configured, DateTime<Utc>) -> Result<Pushed, String> + Send + Sync>;

/// What puts rules on a server. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Pusher(pub Push);

impl Pusher {
    /// The server, signed in to as `mailo sieve push` signs in. Never replaces a script someone
    /// else made: the refusal says so, and the command line is where that is chosen.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|client, account, _now| {
            client
                .net()
                .push_rules(account)
                .map_err(crate::ui::remedy::told)
        }))
    }

    /// Not in a test build: the real one reads the keyring and opens a socket, and a test that
    /// forgot to provide its own must fail in words rather than reach either.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|_, account, _| {
            Err(format!(
                "a test pushed {} without providing a Pusher; the real one reaches the server",
                account.address
            ))
        }))
    }
}

/// Whether the account's server can run rules at all, and why not, in a sentence.
pub(in crate::ui) fn reach(plan: &AccountPlan) -> Result<(), String> {
    mail_proto::sieve::endpoint(plan)
        .map(|_| ())
        .map_err(|why| why.to_string())
}

/// The account as the command line's push takes it: what the server was last seen to support.
pub(in crate::ui) fn configured(
    client: &Client,
    row: &AccountRow,
    now: DateTime<Utc>,
) -> Configured {
    Configured {
        id: row.id.clone(),
        address: row.address.clone(),
        plan: row.plan.clone(),
        caps: client.stored().caps(row.id.clone(), now),
        keep: mail_client::offline::Keep::default(),
    }
}

/// Push `row`'s rules and reply through `push`, and say what the server did. Blocking.
pub(in crate::ui) fn put(
    client: &Client,
    row: &AccountRow,
    push: &Push,
    now: DateTime<Utc>,
) -> Result<String, String> {
    reach(&row.plan)?;
    let account = configured(client, row, now);
    let pushed = push(client, &account, now)?;
    Ok(crate::said::rules::said(&account.address, &pushed))
}

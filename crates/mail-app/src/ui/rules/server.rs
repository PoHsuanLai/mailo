//! The account's rules and vacation reply compiled to Sieve and installed on its server, as
//! `mailo sieve push` does: what every save on the Rules page ends with.
//!
//! What pushes is a context, [`Pusher`], so a test can answer for the server; the real one signs
//! in the way the command line does (`rules::server::pushed`). It runs on a blocking thread. A
//! server with no ManageSieve is not asked.
//!
//! A vacation reply kept where a JMAP or Graph server keeps one (`rules::server::away_server`)
//! is set there instead, through [`AwaySetter`], a context for the same reason.

use chrono::{DateTime, Utc};
use mail_client::Client;
use mail_core::Pushed;
use mail_core::rules::server::AwayServer;
use mail_domain::AccountPlan;
use std::sync::Arc;

use super::super::data::AccountRow;
use mail_core::sync::Configured;

/// Install an account's script: [`mail_core::RuleOps::push`], with the takeover decided. Given the
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

/// Set an account's kept vacation reply where its JMAP or Graph server keeps one:
/// [`mail_core::RuleOps::put_away`]. Given the client by `Arc`, which is what a `Mail` is made over.
pub(in crate::ui) type SetAway =
    Arc<dyn Fn(&Client, &Configured) -> Result<(), String> + Send + Sync>;

/// What sets a vacation reply on a JMAP or Graph server. The real one unless a test provided its
/// own; a Sieve server's reply goes with the rules, through [`Pusher`].
#[derive(Clone)]
pub(in crate::ui) struct AwaySetter(pub SetAway);

impl AwaySetter {
    /// The server, signed in to as `mailo vacation on` signs in.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        use mail_core::rules::server::PushDone;
        Self(Arc::new(|client, account| {
            match client.net().put_away(account) {
                PushDone::NotPushed(why) => Err(crate::ui::remedy::told(why)),
                PushDone::Set { .. } | PushDone::Pushed { .. } => Ok(()),
            }
        }))
    }

    /// Not in a test build, as [`Pusher::server`] is not.
    #[cfg(test)]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|_, account| {
            Err(format!(
                "a test set {}'s vacation reply without providing an AwaySetter; the real one \
                 reaches the server",
                account.address
            ))
        }))
    }
}

/// Where the account's vacation reply is kept on its server, or why it has nowhere to go, in a
/// sentence.
pub(in crate::ui) fn reach_away(plan: &AccountPlan) -> Result<AwayServer, String> {
    mail_core::rules::server::away_server(plan).map_err(|why| why.to_string())
}

/// Set `row`'s kept reply on its JMAP or Graph server through `set`. Blocking.
pub(in crate::ui) fn set_away(
    client: &Client,
    row: &AccountRow,
    set: &SetAway,
    now: DateTime<Utc>,
) -> Result<(), String> {
    set(client, &configured(client, row, now))
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
        keep: mail_core::offline::Keep::default(),
        every: mail_core::offline::Every::default(),
        window: mail_core::offline::Window::default(),
        remove: mail_core::offline::Remove::default(),
        junk: mail_core::junk::Policy::default(),
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

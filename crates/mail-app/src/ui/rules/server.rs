//! "Put on server": the account's rules and vacation reply compiled to Sieve and installed, as
//! `mailo sieve push` does, then said the way it says it.
//!
//! What pushes is a context, [`Pusher`], so a test can answer for the server; the real one signs
//! in the way the command line does (`rules::server::pushed`). It runs on a blocking thread,
//! spawned from the click. A server with no ManageSieve has no button, only the reason.

use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::prelude::*;
use ds::root::common::Common;
use mail_domain::AccountPlan;
use mail_runtime::sieve::Pushed;
use mail_store::SqliteStore;
use std::sync::Arc;

use super::super::common::Told;
use super::super::data::AccountRow;
use super::super::press::{available, on_primary};
use mail_core::sync::Configured;

/// Install an account's script: the signature of [`mail_core::rules::server::pushed`], with the
/// takeover and the saved sign-in clients decided.
pub(in crate::ui) type Push =
    Arc<dyn Fn(&SqliteStore, &Configured, DateTime<Utc>) -> Result<Pushed, String> + Send + Sync>;

/// What puts rules on a server. The real one unless a test provided its own.
#[derive(Clone)]
pub(in crate::ui) struct Pusher(pub Push);

impl Pusher {
    /// The server, signed in to as `mailo sieve push` signs in. Never replaces a script someone
    /// else made: the refusal says so, and the command line is where that is chosen.
    #[cfg(not(test))]
    pub(in crate::ui) fn server() -> Self {
        Self(Arc::new(|store, account, now| {
            mail_core::rules::server::pushed(
                store,
                account,
                mail_proto::sieve::Takeover::Refuse,
                &mail_core::account::saved_clients(),
                now,
            )
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
    store: &SqliteStore,
    row: &AccountRow,
    now: DateTime<Utc>,
) -> Configured {
    Configured {
        id: row.id.clone(),
        address: row.address.clone(),
        plan: row.plan.clone(),
        caps: super::super::ops::caps_here(store, row.id.clone(), now),
        keep: mail_core::offline::Keep::default(),
    }
}

/// Push `row`'s rules and reply through `push`, and say what the server did. Blocking.
pub(in crate::ui) fn put(
    store: &SqliteStore,
    row: &AccountRow,
    push: &Push,
    now: DateTime<Utc>,
) -> Result<String, String> {
    reach(&row.plan)?;
    let account = configured(store, row, now);
    let pushed = push(store, &account, now)?;
    Ok(mail_core::rules::server::said(&account.address, &pushed))
}

/// Where a push stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Pushing {
    Ready,
    Running,
    Said(String),
    Failed(String),
}

/// The server's group of the page: Put on Server and what the last push said, or the reason
/// there is no server to put rules on.
#[component]
pub(super) fn ServerPart(row: AccountRow) -> Element {
    let mut pushing = use_signal(|| Pushing::Ready);
    if let Err(why) = reach(&row.plan) {
        return rsx! {
            FieldGroup { title: "On the server",
                FieldRow {
                    label: "No copy on a server",
                    help: Some(TextLine::from(format!("{why}."))),
                }
            }
        };
    }
    let busy = pushing() == Pushing::Running;
    let label = format!("Put {}'s rules on the server", row.address);
    let mut start = {
        let row = row.clone();
        move |_| {
            if *pushing.peek() == Pushing::Running {
                return;
            }
            pushing.set(Pushing::Running);
            let store = consume_context::<Arc<SqliteStore>>();
            let push = try_consume_context::<Pusher>()
                .unwrap_or_else(Pusher::server)
                .0;
            let row = row.clone();
            spawn(async move {
                let done =
                    tokio::task::spawn_blocking(move || put(&store, &row, &push, Utc::now())).await;
                pushing.set(match done {
                    Ok(Ok(said)) => Pushing::Said(said),
                    Ok(Err(why)) => Pushing::Failed(why),
                    Err(error) => {
                        Pushing::Failed(format!("It stopped before it finished: {error}"))
                    }
                });
            });
        }
    };
    let help = match pushing() {
        Pushing::Running => "Putting them on the server…",
        _ => "Runs your rules and vacation reply on the server while this computer is off.",
    };
    rsx! {
        FieldGroup { title: "On the server",
            FieldRow {
                label: "Put on server",
                help: Some(TextLine::from(help)),
                Button {
                    label: "Put on Server",
                    icon: Icon::Send,
                    availability: available(!busy),
                    onclick: on_primary(move || start(())),
                    common: Common { aria_label: Some(label.to_string()), ..Common::default() },
                }
            }
            if let Pushing::Said(said) = pushing() {
                {
                    let (head, rest) = said.split_once('\n').unwrap_or((said.as_str(), ""));
                    let rest = rest.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
                    rsx! {
                        FieldRow {
                            label: head.to_owned(),
                            help: (!rest.is_empty()).then(|| TextLine::from(rest)),
                        }
                    }
                }
            }
        }
        if let Pushing::Failed(why) = pushing() {
            Told { said: Some(Err(why)) }
        }
    }
}

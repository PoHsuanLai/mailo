//! "Search … on the server", at the end of a search's list.
//!
//! A search lists what the store holds. Where the list ends, each account in view that has a
//! server to search offers to search it (`mail_core::server_search`); pressed, the account's server
//! is asked off the drawing thread, what it finds is kept as headers, and the conversations it
//! named follow the list under "From the server", each with the chip that says so. What the
//! server could not be asked is said instead of searched for.
//!
//! It runs by itself only when "Search the server automatically" is on, from the effect that
//! sees a search settle (F140: an effect's task is polled, a render's is not), and only once per
//! line and account. What searches is a root context, [`ServerSearcher`], so a test can answer
//! for a server.

use super::press::on_primary;
use chrono::{DateTime, Utc};
use dioxus::prelude::*;
use ds::prelude::*;
use ds::style::tokens::control_size::ControlSize;
use mail_core::Searched;
use mail_core::{SqliteStore, Store};
use mail_domain::ThreadId;
use porter_core::AccountId;
use std::sync::Arc;

/// Search one account's server for a typed line: the signature of [`mail_core::server_search::search`].
pub type Search = Arc<
    dyn Fn(Arc<SqliteStore>, AccountId, &str, DateTime<Utc>) -> Result<Searched, String>
        + Send
        + Sync,
>;

/// What searches an account's server. The server itself unless a test provided its own.
#[derive(Clone)]
pub struct ServerSearcher(pub Search);

impl std::fmt::Debug for ServerSearcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ServerSearcher")
    }
}

impl ServerSearcher {
    /// The server, through [`mail_core::server_search::search`].
    #[cfg(not(test))]
    fn server() -> Self {
        Self(Arc::new(|store, account, input, _now| {
            let mail = crate::edge::mail(&store);
            crate::edge::block_on(mail_core::server_search::search(&mail, account, input))
                .map_err(String::from)
        }))
    }

    /// Not in a test build: the real search reads the keyring and opens a socket, and a test
    /// that forgot to provide its own must fail in words rather than reach either.
    #[cfg(test)]
    fn server() -> Self {
        Self(Arc::new(|_, _, input, _| {
            Err(format!(
                "a test searched the server for {input:?} without providing a ServerSearcher"
            ))
        }))
    }
}

/// One account's server asked for one line, and how that went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Asked {
    pub input: String,
    pub account: AccountId,
    pub answer: Answer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Answer {
    Running,
    /// The conversations the server named, newest first; how many of their messages this search
    /// brought; how many more the server matched.
    Found {
        threads: Vec<ThreadId>,
        fetched: usize,
        more: u64,
    },
    /// Not asked: what the server cannot be asked faithfully.
    Unsaid(String),
    Failed(String),
}

/// The answer for `input` on `account`, if it was asked.
pub(super) fn answer_for<'a>(
    asked: &'a [Asked],
    input: &str,
    account: AccountId,
) -> Option<&'a Answer> {
    asked
        .iter()
        .find(|a| a.input == input && a.account == account)
        .map(|a| &a.answer)
}

/// Every conversation the servers named for `input`, newest first, less those in `listed`.
pub(super) fn found_threads(asked: &[Asked], input: &str, listed: &[ThreadId]) -> Vec<ThreadId> {
    let mut out: Vec<ThreadId> = Vec::new();
    for a in asked.iter().filter(|a| a.input == input) {
        if let Answer::Found { threads, .. } = &a.answer {
            for thread in threads {
                if !listed.contains(thread) && !out.contains(thread) {
                    out.push(*thread);
                }
            }
        }
    }
    out
}

/// What the line under an account's search says once it has an answer.
pub(super) fn said(address: &str, answer: &Answer) -> Option<String> {
    Some(match answer {
        Answer::Running => format!("Searching {address} on the server…"),
        Answer::Found { threads, .. } if threads.is_empty() => {
            format!("Nothing found on {address}'s server.")
        }
        Answer::Found {
            threads,
            fetched,
            more,
        } => {
            let found = match threads.len() {
                1 => "1 conversation".to_owned(),
                n => format!("{n} conversations"),
            };
            let more = if *more > 0 {
                format!("; {more} more there")
            } else {
                String::new()
            };
            format!("{found} found on {address}'s server, {fetched} new here{more}.")
        }
        Answer::Unsaid(what) => format!("Not searched on {address}'s server: {what}."),
        Answer::Failed(why) => why.clone(),
    })
}

/// Ask `account`'s server for `input`, off the drawing thread, and put its answer in `asked`.
///
/// From a click, or from [`ServerSearch`]'s effect: never from a render (F140).
pub(super) fn start(
    mut asked: Signal<Vec<Asked>>,
    mut revision: Signal<u64>,
    input: String,
    account: AccountId,
) {
    if matches!(
        answer_for(&asked.peek(), &input, account.clone()),
        Some(Answer::Running)
    ) {
        return;
    }
    {
        let mut write = asked.write();
        write.retain(|a| !(a.input == input && a.account == account));
        write.push(Asked {
            input: input.clone(),
            account: account.clone(),
            answer: Answer::Running,
        });
    }
    let search = try_consume_context::<ServerSearcher>().unwrap_or_else(ServerSearcher::server);
    let store = consume_context::<Arc<SqliteStore>>();
    let searched_for = account.clone();
    spawn(async move {
        let line = input.clone();
        // `spawn_blocking`: the search waits on the application's runtime (`edge::block_on`), which
        // an async task must not.
        let done = tokio::task::spawn_blocking(move || {
            let searched = (search.0)(store.clone(), searched_for, &line, Utc::now());
            searched.map(|searched| match searched {
                Searched::Unsaid(unsaid) => Answer::Unsaid(unsaid.to_string()),
                Searched::Found(hits) => {
                    let mut threads: Vec<ThreadId> = Vec::new();
                    for message in &hits.messages {
                        if let Ok(m) = store.message(*message)
                            && !threads.contains(&m.thread)
                        {
                            threads.push(m.thread);
                        }
                    }
                    Answer::Found {
                        threads,
                        fetched: hits.fetched,
                        more: hits.more,
                    }
                }
            })
        })
        .await;
        let answer = match done {
            Ok(Ok(answer)) => answer,
            Ok(Err(why)) => Answer::Failed(why),
            Err(e) => Answer::Failed(format!("the search of the server stopped: {e}")),
        };
        if let Some(entry) = asked
            .write()
            .iter_mut()
            .find(|a| a.input == input && a.account == account)
        {
            entry.answer = answer;
        }
        revision += 1;
    });
}

/// The end of a search's list: per account in view with a server, its button or its answer.
#[component]
pub(super) fn ServerSearch(
    /// The settled search line.
    input: String,
    /// The accounts in view that have a server, with their addresses.
    accounts: Vec<(AccountId, String)>,
    asked: Signal<Vec<Asked>>,
    revision: Signal<u64>,
) -> Element {
    // Automatically, where the user turned it on: once per line and account.
    let settings = crate::ui::prefs::use_settings();
    let automatic = use_memo(move || {
        mail_core::server_search::Automatic::from(settings.read().search.server_automatically)
    });
    use_effect(use_reactive(
        (&input, &accounts),
        move |(wanted, everyone)| {
            if automatic() != mail_core::server_search::Automatic::On {
                return;
            }
            for (account, _) in &everyone {
                if answer_for(&asked.peek(), &wanted, account.clone()).is_none() {
                    start(asked, revision, wanted.clone(), account.clone());
                }
            }
        },
    ));
    rsx! {
        // The line the buttons search for, which the list settles a beat after the typing: a
        // press is for this line, and its answer shows while the line is still this one.
        div { class: "server-search", "data-line": "{input}",
            for (account, address) in accounts {
                {
                    let answer = answer_for(&asked.read(), &input, account.clone()).cloned();
                    let line = input.clone();
                    let bad = matches!(answer, Some(Answer::Failed(_)));
                    let ask = !matches!(answer, Some(Answer::Running));
                    rsx! {
                        div { key: "{account}", class: "server-search-one",
                            if let Some(said) = answer.as_ref().and_then(|a| said(&address, a)) {
                                p { class: if bad { "status bad" } else { "status" }, "{said}" }
                            }
                            if ask {
                                Button {
                                    size: ControlSize::Small,
                                    label: if answer.is_some() {
                                        format!("Search {address} on the server again")
                                    } else {
                                        format!("Search {address} on the server")
                                    },
                                    icon: Icon::Search,
                                    onclick: on_primary(move || start(asked, revision, line.clone(), account.clone())),
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;

    fn id(n: u128) -> ThreadId {
        ThreadId::from_uuid(uuid::Uuid::from_u128(n))
    }

    fn acct_account() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
    }

    #[test]
    fn what_each_answer_says() {
        let found = |threads: Vec<ThreadId>, fetched, more| Answer::Found {
            threads,
            fetched,
            more,
        };
        let cases: Vec<(Answer, &str)> = vec![
            (Answer::Running, "Searching me@example.test on the server…"),
            (
                found(vec![], 0, 0),
                "Nothing found on me@example.test's server.",
            ),
            (
                found(vec![id(1)], 1, 0),
                "1 conversation found on me@example.test's server, 1 new here.",
            ),
            (
                found(vec![id(1), id(2)], 0, 48),
                "2 conversations found on me@example.test's server, 0 new here; 48 more there.",
            ),
            (
                Answer::Unsaid("is:pinned (kept on this computer)".to_owned()),
                "Not searched on me@example.test's server: is:pinned (kept on this computer).",
            ),
        ];
        for (answer, line) in cases {
            assert_eq!(said("me@example.test", &answer).as_deref(), Some(line));
        }
    }

    #[test]
    fn found_conversations_leave_out_what_is_listed_and_other_lines() {
        let asked = vec![
            Asked {
                input: "lunch".to_owned(),
                account: acct_account(),
                answer: Answer::Found {
                    threads: vec![id(1), id(2), id(3)],
                    fetched: 2,
                    more: 0,
                },
            },
            Asked {
                input: "budget".to_owned(),
                account: acct_account(),
                answer: Answer::Found {
                    threads: vec![id(9)],
                    fetched: 1,
                    more: 0,
                },
            },
        ];
        assert_eq!(found_threads(&asked, "lunch", &[id(2)]), vec![id(1), id(3)]);
        assert_eq!(found_threads(&asked, "lunc", &[]), Vec::<ThreadId>::new());
    }
}

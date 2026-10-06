//! A message body that is still on the server, and an attachment being saved.
//!
//! The states are `mail_core::fetch`'s pure machines (`Body`, `Download`); this file is what the
//! reader draws for each, in words a person reads, and the two places that start the work.
//!
//! What can fail is a network call, so both calls go through [`Fetchers`]: the window runs the
//! real ones, a test provides its own and never touches a network.

use dioxus::prelude::*;
use ds::prelude::*;
use ds::root::common::Common;
use mail_core::fetch::{Body as BodyState, BodyEffect, BodyEvent};
use mail_domain::{Incoming, MessageId, Retry};
use mail_store::{SqliteStore, Store as _};
use porter_core::AccountId;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::ui::press::on_primary;

type BodyFetch = dyn Fn(Arc<SqliteStore>, MessageId) -> Result<(), (Retry, String)> + Send + Sync;
type PartFetch = dyn Fn(&Arc<SqliteStore>, MessageId, &str) -> Result<(), String> + Send + Sync;

/// The two network calls the reader makes. Provided as context by a test; the window uses
/// [`Fetchers::default`].
#[derive(Clone)]
pub(in crate::ui) struct Fetchers {
    /// Download one message's body into the store.
    pub body: Arc<BodyFetch>,
    /// Download one attachment (by IMAP section or Graph id) into the store.
    pub part: Arc<PartFetch>,
}

impl Default for Fetchers {
    fn default() -> Self {
        Fetchers {
            body: Arc::new(|store, message| {
                mail_core::sync::fetch_body(store, message, chrono::Utc::now())
            }),
            part: Arc::new(|store, message, section| {
                mail_core::sync::fetch_part(store, message, section, chrono::Utc::now())
            }),
        }
    }
}

pub(super) fn fetchers() -> Fetchers {
    try_consume_context::<Fetchers>().unwrap_or_default()
}

// --- Copy -------------------------------------------------------------------------------------

/// What the person can do about a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Again {
    /// Try again.
    Offer,
    /// Trying again cannot help: the account has to sign in, or the account does not work this way.
    Withhold,
}

impl Again {
    fn of(retry: &Retry) -> Again {
        match retry {
            Retry::Now | Retry::After(_) => Again::Offer,
            Retry::NeedsReauth | Retry::Fatal(_) => Again::Withhold,
        }
    }
}

/// What the body area shows for one state.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum BodyFace {
    /// The calm prompt with a Download Message button.
    Prompt,
    /// A `Loadable` in this phase; its Retry is offered when `Again::Offer`.
    Pane(Phase, Again),
}

pub(super) const PROMPT: &str = "This message hasn't been downloaded yet.";
pub(super) const DOWNLOAD_MESSAGE: &str = "Download Message";
pub(super) const BODY_FAILED: &str = "Couldn't download this message";

/// Its first line, trimmed to a length a secondary line can hold.
pub(super) fn one_line(why: &str) -> String {
    let line = why.lines().map(str::trim).find(|l| !l.is_empty());
    let line = line.unwrap_or_default();
    let mut chars = line.chars();
    let short: String = chars.by_ref().take(120).collect();
    if chars.next().is_some() {
        format!("{}…", short.trim_end())
    } else {
        short
    }
}

/// The plain-language reason a body could not be fetched. `account` is the address it belongs to.
pub(super) fn body_reason(retry: &Retry, why: &str, account: &str) -> String {
    match retry {
        Retry::NeedsReauth => format!("Sign in to {account} again to download it."),
        Retry::Fatal(_) => "This account's messages download with the next sync.".to_owned(),
        Retry::Now | Retry::After(_) => {
            let detail = one_line(why);
            if detail.is_empty() {
                "Check your connection and try again.".to_owned()
            } else {
                format!("Check your connection and try again. ({detail})")
            }
        }
    }
}

/// What the body area draws: `op` is the running operation, made in the handler that began it.
pub(super) fn body_face(state: &BodyState, op: Operation, account: &str) -> BodyFace {
    match state {
        // A body that is here is not drawn by this pane; if the reader still has none to show
        // (the fetch said yes and stored nothing), the prompt is the honest answer.
        BodyState::Missing | BodyState::Here => BodyFace::Prompt,
        BodyState::Fetching => BodyFace::Pane(Phase::Loading(op), Again::Withhold),
        BodyState::Failed { retry, why } => BodyFace::Pane(
            Phase::Failed {
                title: BODY_FAILED.to_owned(),
                description: Some(TextLine::from(body_reason(retry, why, account))),
            },
            Again::of(retry),
        ),
    }
}

/// What a failed attachment download says under its row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    pub text: String,
    pub detail: String,
    pub again: Again,
}

pub(super) fn download_failure(name: &str, retry: &Retry, why: &str, account: &str) -> Failure {
    let detail = match retry {
        Retry::NeedsReauth => format!("Sign in to {account} again."),
        _ => one_line(why),
    };
    Failure {
        text: format!("Couldn't download {name}."),
        detail,
        again: Again::of(retry),
    }
}

/// The toast after a save: the folder the file went to, by its name.
pub(super) fn saved_toast(path: &Path) -> String {
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    match folder {
        Some(folder) => format!("Saved to {folder}"),
        None => "Saved".to_owned(),
    }
}

// --- Store lookups -----------------------------------------------------------------------------

/// The address an account signs in with.
pub(super) fn account_address(store: &SqliteStore, account: AccountId) -> String {
    store
        .connection()
        .query_row(
            "SELECT address FROM accounts WHERE id = ?1",
            [account.to_string()],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "this account".to_owned())
}

/// Whether opening a headers-only message of this account should fetch it at once: IMAP and
/// Graph can; POP3, JMAP and local accounts get their bodies with a sync.
pub(super) fn fetches_on_open(store: &SqliteStore, account: AccountId) -> bool {
    let plan: Option<String> = store
        .connection()
        .query_row(
            "SELECT plan FROM accounts WHERE id = ?1",
            [account.to_string()],
            |r| r.get(0),
        )
        .ok();
    plan.and_then(|p| serde_json::from_str::<mail_domain::AccountPlan>(&p).ok())
        .is_some_and(|plan| opens_with_fetch(&plan.incoming))
}

/// IMAP and Graph fetch one message on their own; the rest wait for a sync.
pub(super) fn opens_with_fetch(incoming: &Incoming) -> bool {
    matches!(incoming, Incoming::Imap { .. } | Incoming::Graph)
}

/// Fetch a part still on the server, then write it into `dir`.
pub(super) fn fetch_then_save(
    store: &Arc<SqliteStore>,
    fetchers: &Fetchers,
    message: MessageId,
    index: usize,
    dir: &Path,
) -> Result<PathBuf, String> {
    let stored = store.message(message).map_err(|e| e.to_string())?;
    if let Some(attachment) = stored.attachments.get(index)
        && let mail_domain::PartContent::Remote { section } = &attachment.content
    {
        (fetchers.part)(store, message, section)?;
    }
    mail_core::attach::save(store, message, index, dir)
}

// --- The body area ----------------------------------------------------------------------------

/// The body of a message whose body is not stored: a prompt, a spinner, or what went wrong.
///
/// `landed` is the reader's, moved when the body arrives so the reader reads it again.
#[component]
pub(super) fn BodyPane(message: MessageId, account: AccountId, mut landed: Signal<u64>) -> Element {
    let store = use_context::<Arc<SqliteStore>>();
    let address = use_hook(|| account_address(&store, account.clone()));
    // Mail fetches a body when its message is opened; so does this, where the account can. The
    // pane starts as Fetching with no operation yet, and the effect below starts it: an
    // operation's token is made where the work starts, not while drawing.
    let mut state = use_signal(|| {
        if fetches_on_open(&store, account) {
            BodyState::Fetching
        } else {
            BodyState::Missing
        }
    });
    let mut op = use_signal(Operation::default);
    let mut begin = move || {
        op.set(Operation::Running(PendingToken::start()));
        let store = consume_context::<Arc<SqliteStore>>();
        let fetch = fetchers().body;
        spawn(async move {
            // `spawn_blocking`: the fetch opens sockets and builds its own runtime.
            // Claimed for as long as it runs, so a body pass that starts meanwhile leaves it to us.
            let done = tokio::task::spawn_blocking(move || {
                let _claim = mail_runtime::wanted::claim(message);
                fetch(store, message)
            })
            .await;
            let event = match done {
                Ok(Ok(())) => BodyEvent::Arrived,
                Ok(Err((retry, why))) => BodyEvent::Failed { retry, why },
                Err(error) => BodyEvent::Failed {
                    retry: Retry::Now,
                    why: error.to_string(),
                },
            };
            let arrived = event == BodyEvent::Arrived;
            let (to, _) = state.peek().step(event);
            state.set(to);
            op.set(Operation::Idle);
            if arrived {
                landed += 1;
            }
        });
    };
    use_effect(move || {
        if matches!(*state.peek(), BodyState::Fetching) && matches!(*op.peek(), Operation::Idle) {
            begin();
        }
    });
    let mut ask = move || {
        if matches!(*state.peek(), BodyState::Here) {
            state.set(BodyState::Missing);
        }
        let (to, effect) = state.peek().step(BodyEvent::Ask);
        state.set(to);
        if effect == Some(BodyEffect::Fetch) {
            begin();
        }
    };
    match body_face(&state(), op(), &address) {
        BodyFace::Prompt => rsx! {
            div { class: "body-pane",
                EmptyState {
                    title: PROMPT,
                    action: rsx! {
                        Button {
                            label: DOWNLOAD_MESSAGE,
                            common: Common { aria_label: Some(DOWNLOAD_MESSAGE.to_owned()), ..Common::default() },
                            onclick: on_primary(ask),
                        }
                    },
                }
            }
        },
        BodyFace::Pane(phase, again) => rsx! {
            div { class: "body-pane",
                Loadable {
                    phase,
                    onretry: match again {
                        Again::Offer => Some(EventHandler::new(move |()| ask())),
                        Again::Withhold => None,
                    },
                    common: Common { aria_label: Some("Message body".to_owned()), ..Common::default() },
                }
            }
        },
    }
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod tests;

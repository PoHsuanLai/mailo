//! Finding and reading: search, a conversation's text, and a conversation's preview.

use super::{Provider, outcome};
use crate::intents::APP;
use crate::intents::wire::{
    AppRefusal, EntityId, EntityRef, Hit, Invocation, Label, Labelled, Outcome, Output, Preview,
    Snip, Target,
};
use chrono::{Local, Utc};
use mail_core::when::{Stamp, stamp};
use mail_domain::{Address, Message, ThreadId, ThreadSummary};
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

/// The most a search answers with.
const LIMIT: usize = 25;
/// The most a read answers with, in characters: the limit the manifest declares for it.
const READ: usize = 20_000;
/// The Space labels are private to when the call names none (a search, a preview).
const SPACE: &str = "desktop";

fn thread_id(thread: ThreadId) -> EntityId {
    EntityId {
        app: APP.to_owned(),
        kind: "mail.thread".to_owned(),
        key: thread.to_string(),
    }
}

/// Somebody's name, or their address when they gave none.
fn who(address: &Address) -> String {
    address
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| address.email.clone())
}

fn third_party(value: String, space: &str) -> Labelled<String> {
    Labelled {
        value,
        label: Label::mail(space),
    }
}

fn entity_of(summary: &ThreadSummary, space: &str) -> EntityRef {
    EntityRef {
        id: thread_id(summary.id),
        title: third_party(summary.subject.clone(), space),
        subtitle: third_party(
            format!(
                "{}, {}",
                who(&summary.from),
                stamp(summary.last_date, &Local, Stamp::Full)
            ),
            space,
        ),
    }
}

impl Provider {
    /// The conversations `text` finds, in mailo's own search language: the top results first,
    /// then the rest, newest first.
    fn found(&self, text: &str) -> Result<Vec<ThreadSummary>, AppRefusal> {
        let labels = mail_core::query::known_labels(&self.store);
        let named = mail_core::query::named(&labels);
        let searched = mail_core::search::search_list(
            text,
            self.store.as_ref(),
            &mail_core::search::Affinity::default(),
            &Local,
            &named,
            mail_core::search::first(LIMIT.saturating_add(mail_core::search::STRIP)),
            Utc::now(),
        )
        .map_err(AppRefusal::Failed)?;
        let top: Vec<ThreadSummary> = searched.top.into_iter().map(|(found, _)| found).collect();
        let rest: Vec<ThreadSummary> = searched
            .rows
            .into_iter()
            .filter(|row| !top.iter().any(|hit| hit.id == row.id))
            .collect();
        Ok(top.into_iter().chain(rest).take(LIMIT).collect())
    }

    /// `Search`: the live search of a kind mailo does not index.
    pub(super) fn search_hits(&self, text: &str) -> Vec<Hit> {
        self.found(text)
            .unwrap_or_default()
            .iter()
            .map(|summary| Hit {
                entity: entity_of(summary, SPACE),
                why: None,
            })
            .collect()
    }

    /// The action: the same search, answered as the ids found.
    pub(super) fn search_action(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let query = invocation.text("query").ok_or(AppRefusal::NeedsParam {
            param: "query".to_owned(),
            options: Vec::new(),
        })?;
        let ids: Vec<EntityId> = self
            .found(query)?
            .iter()
            .map(|summary| thread_id(summary.id))
            .collect();
        let said = match ids.len() {
            0 => "Nothing found".to_owned(),
            1 => "Found 1 conversation".to_owned(),
            n => format!("Found {n} conversations"),
        };
        let found = Labelled {
            value: Output::Entities(ids),
            label: Label::own(APP),
        };
        Ok(outcome(Some(said), None, Some(found)))
    }

    /// A conversation's messages as text, oldest first, as many as fit.
    pub(super) fn read(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let Target::Entities(named) = &invocation.target else {
            return Err(AppRefusal::Unsupported);
        };
        let [id] = named.as_slice() else {
            return Err(AppRefusal::Unsupported);
        };
        let (thread, _) = self.loaded(id)?;
        let text: String = thread_text(&self.store, &thread)
            .chars()
            .take(READ)
            .collect();
        let read = Labelled {
            value: Output::Text(text),
            label: Label::mail(&invocation.space),
        };
        Ok(outcome(None, None, Some(read)))
    }

    /// `mail.thread.open`: show the conversation in mailo's window. The launcher's Enter on a
    /// mail hit; it answers nothing, the window is the answer.
    pub(super) fn open(&self, invocation: &Invocation) -> Result<Outcome, AppRefusal> {
        let Target::Entities(named) = &invocation.target else {
            return Err(AppRefusal::Unsupported);
        };
        let [id] = named.as_slice() else {
            return Err(AppRefusal::Unsupported);
        };
        let (_, thread) = self.loaded(id)?;
        (self.opener.0)(thread, invocation.activation.as_deref()).map_err(AppRefusal::Failed)?;
        Ok(outcome(None, None, None))
    }

    /// The thread an entity names, with its messages, or why not.
    fn loaded(&self, id: &EntityId) -> Result<(mail_domain::Thread, ThreadId), AppRefusal> {
        let thread = match (id.kind.as_str(), id.key.parse()) {
            ("mail.thread", Ok(key)) => ThreadId::from_uuid(key),
            _ => return Err(AppRefusal::NotFound(id.clone())),
        };
        self.store
            .thread(thread)
            .map(|loaded| (loaded, thread))
            .map_err(|_| AppRefusal::NotFound(id.clone()))
    }

    /// What `Preview` shows for a conversation: its subject and its newest messages.
    pub(super) fn preview_of(&self, id: &EntityId) -> Preview {
        let Ok((thread, _)) = self.loaded(id) else {
            return Preview::None;
        };
        let newest: Vec<Message> = thread
            .messages
            .iter()
            .rev()
            .take(5)
            .filter_map(|id| self.store.message(*id).ok())
            .collect();
        Preview::Thread {
            subject: third_party(thread.summary.subject.clone(), SPACE),
            messages: newest
                .iter()
                .rev()
                .map(|message| Snip {
                    from: third_party(who(&message.from), SPACE),
                    snippet: third_party(
                        message
                            .body
                            .text()
                            .unwrap_or("(body not fetched yet)")
                            .chars()
                            .take(200)
                            .collect(),
                        SPACE,
                    ),
                    at: message.date.timestamp(),
                })
                .collect(),
        }
    }
}

/// The messages of `thread` as one text: who, when, then the words.
fn thread_text(store: &SqliteStore, thread: &mail_domain::Thread) -> String {
    let mut out = format!("{}\n", thread.summary.subject);
    for id in &thread.messages {
        let Ok(message) = store.message(*id) else {
            continue;
        };
        let _ = write!(
            out,
            "\n--- {} <{}>  {}\n{}\n",
            message.from.name.as_deref().unwrap_or(""),
            message.from.email,
            stamp(message.date, &Local, Stamp::Full),
            message
                .body
                .text()
                .map_or("(body not fetched yet)", str::trim_end),
        );
    }
    out
}

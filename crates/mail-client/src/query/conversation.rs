//! One conversation, whole: its summary and every message in it.

use crate::{ClientError, Query, Topics};
use chrono::{DateTime, Utc};
use mail_domain::{Message, ThreadId, ThreadSummary};
use mail_store::{SqliteStore, Store, StoreError};

/// The conversation `ThreadId` is, with its messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conversation(pub ThreadId);

/// A conversation as it is held: its summary, and its messages in the order it keeps them,
/// oldest first. A message the store no longer has is left out, as a list leaves out a
/// conversation it no longer has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Messages {
    pub summary: ThreadSummary,
    pub messages: Vec<Message>,
}

impl Query for Conversation {
    /// `None` when there is no such conversation: it was deleted, or its last message moved
    /// into another.
    type Output = Option<Messages>;

    /// The labels too: a message carries the labels it is filed under.
    const READS: Topics = Topics::SUMMARIES.union(Topics::LABELS);

    fn run(
        &self,
        store: &SqliteStore,
        _now: DateTime<Utc>,
    ) -> Result<Option<Messages>, ClientError> {
        let thread = match store.thread(self.0) {
            Ok(thread) => thread,
            Err(StoreError::NoThread(_)) => return Ok(None),
            Err(why) => return Err(why.into()),
        };
        let mut messages = Vec::with_capacity(thread.messages.len());
        for id in thread.messages {
            match store.message(id) {
                Ok(message) => messages.push(message),
                Err(StoreError::NoMessage(_)) => {}
                Err(why) => return Err(why.into()),
            }
        }
        Ok(Some(Messages {
            summary: thread.summary,
            messages,
        }))
    }
}

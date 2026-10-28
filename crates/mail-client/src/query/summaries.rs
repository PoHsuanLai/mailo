//! The summaries of some conversations named by id: what a list holds for rows it was not asked
//! for, such as the matches a server's search found.

use crate::{ClientError, Query, Topics};
use chrono::{DateTime, Utc};
use mail_domain::{ThreadId, ThreadSummary};
use mail_store::{SqliteStore, Store, StoreError};

/// The summaries of `ThreadId`s, in the order asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summaries(pub Vec<ThreadId>);

impl Query for Summaries {
    fn bucket(&self) -> u64 {
        crate::live::bucket_of(&self.0)
    }

    /// One for each conversation the store has; one it has not (deleted, or moved into another)
    /// is left out.
    type Output = Vec<ThreadSummary>;

    const READS: Topics = Topics::SUMMARIES;

    fn run(
        &self,
        store: &SqliteStore,
        _now: DateTime<Utc>,
    ) -> Result<Vec<ThreadSummary>, ClientError> {
        let mut found = Vec::with_capacity(self.0.len());
        for id in &self.0 {
            match store.thread(*id) {
                Ok(thread) => found.push(thread.summary),
                Err(StoreError::NoThread(_)) => {}
                Err(why) => return Err(why.into()),
            }
        }
        Ok(found)
    }
}

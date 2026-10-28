//! How fresh each account is judged to be besides its link: when the store last recorded a
//! finished pass of it, and what its server allows between passes.

use crate::{ClientError, Query, Topics};
use chrono::{DateTime, Utc};
use mail_core::sync::fresh::Pace;
use mail_store::SqliteStore;
use porter_core::AccountId;

/// The [`Pace`] of each of the accounts asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paces(pub Vec<AccountId>);

impl Query for Paces {
    fn bucket(&self) -> u64 {
        crate::live::bucket_of(&self.0)
    }

    /// Each account asked, with its pace, in the order asked.
    type Output = Vec<(AccountId, Pace)>;

    /// A pass finished is a write that declares nothing, which publishes every topic.
    const READS: Topics = Topics::ACCOUNTS.union(Topics::CLOCK);

    fn run(
        &self,
        store: &SqliteStore,
        _now: DateTime<Utc>,
    ) -> Result<Vec<(AccountId, Pace)>, ClientError> {
        Ok(self
            .0
            .iter()
            .map(|account| (account.clone(), Pace::of(store, account.clone())))
            .collect())
    }
}

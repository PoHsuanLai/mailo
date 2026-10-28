//! Push through a relay for a Graph account (migration 0033). See [`crate::GraphPush`].

use super::SqliteStore;
use super::row::{from_time, time};
use crate::StoreError;
use crate::graph_push::{GraphPush, Registered, Subscribed};
use porter_core::AccountId;
use rusqlite::{OptionalExtension, params};

impl SqliteStore {
    /// What is held about `account`'s push. The default (no relay, nothing set up) when nothing
    /// ever was.
    pub fn graph_push(&self, account: AccountId) -> Result<GraphPush, StoreError> {
        type Row = (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        );
        let row: Option<Row> = self
            .reader()
            .query_row(
                "SELECT relay, registered, key, subscription, expires FROM graph_push
                 WHERE account = ?1",
                [account.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let Some((relay, registered, key, subscription, expires)) = row else {
            return Ok(GraphPush::default());
        };
        let registered = match (registered, key) {
            (Some(relay), Some(key)) => Some(Registered { relay, key }),
            _ => None,
        };
        let subscription = match (&registered, subscription, expires) {
            (Some(_), Some(id), Some(expires)) => Some(Subscribed {
                id,
                expires: time("graph_push.expires", &expires)?,
            }),
            _ => None,
        };
        Ok(GraphPush {
            relay,
            registered,
            subscription,
        })
    }

    /// Push `account` through the relay at `relay`, or not at all with `None`. What was set up
    /// is kept: the next listen finds out whether it still serves. An account the store does
    /// not hold is not recorded.
    pub fn set_push_relay(
        &self,
        account: AccountId,
        relay: Option<&str>,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO graph_push (account, relay)
             SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM accounts WHERE id = ?1)
             ON CONFLICT (account) DO UPDATE SET relay = excluded.relay",
            params![account.to_string(), relay],
        )?;
        Ok(())
    }

    /// `account` was registered with `relay` as `key`. Any subscription made for the key before
    /// is forgotten with it.
    pub fn push_registered(
        &self,
        account: AccountId,
        registered: &Registered,
    ) -> Result<(), StoreError> {
        self.connection().execute(
            "INSERT INTO graph_push (account, registered, key)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM accounts WHERE id = ?1)
             ON CONFLICT (account) DO UPDATE SET registered = excluded.registered,
                 key = excluded.key, subscription = NULL, expires = NULL",
            params![account.to_string(), registered.relay, registered.key],
        )?;
        Ok(())
    }

    /// `account`'s subscription is `subscribed` now, or there is none.
    pub fn push_subscribed(
        &self,
        account: AccountId,
        subscribed: Option<&Subscribed>,
    ) -> Result<(), StoreError> {
        let (id, expires): (Option<&str>, Option<String>) = match subscribed {
            Some(one) => (Some(&one.id), Some(from_time(one.expires))),
            None => (None, None),
        };
        self.connection().execute(
            "UPDATE graph_push SET subscription = ?2, expires = ?3 WHERE account = ?1",
            params![account.to_string(), id, expires],
        )?;
        Ok(())
    }
}

//! A [`Mail`] around a test's store, for driving `cli::run`.
//!
//! The handle is the program's own (`edge::mail`) with what a test must not leave to the machine
//! changed: the clock stands still at `now`, and the OAuth clients are named here rather than
//! left to whatever the edge was given.
//!
//! None, on purpose: these tests drive `account add` for Google and Microsoft addresses and read
//! the advice an OAuth account gets with no client at hand. A client (even
//! `mail_core::testing::fake_clients`) would start a sign-in instead, and wait on a browser.

use chrono::{DateTime, Utc};
use mail_core::{ClientRegistry, FixedClock, Mail, SqliteStore};
use std::sync::Arc;

/// The handle over `store`, at `now`, with no OAuth client at hand.
pub fn mail_at(store: &Arc<SqliteStore>, now: DateTime<Utc>) -> Mail {
    mail_app::edge::mail(store)
        .with_clients(ClientRegistry::default())
        .with_clock(Arc::new(FixedClock::at(now)))
}

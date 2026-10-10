//! A [`Mail`] around a test's store, for driving `cli::run`.
//!
//! The handle is the program's own (`edge::mail`) with what a test must not leave to the machine
//! changed: the clock stands still at `now`, and the OAuth clients are none, because this links
//! the ordinary library, `saved_clients`' `cfg!(test)` guard does not apply, and a real client
//! id would open a browser and wait.

use chrono::{DateTime, Utc};
use mail_core::{ClientRegistry, FixedClock, Mail, SqliteStore};
use std::sync::Arc;

/// The handle over `store`, at `now`, with no saved OAuth clients.
pub fn mail_at(store: &Arc<SqliteStore>, now: DateTime<Utc>) -> Mail {
    mail_app::edge::mail(store)
        .with_clients(ClientRegistry::default())
        .with_clock(Arc::new(FixedClock::at(now)))
}

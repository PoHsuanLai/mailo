//! Driving a JMAP account: one pass of [`mail_runtime::JmapEngine`].
//!
//! The pass itself is the engine's — the mailbox list, changes, headers, bodies, the outbox — so
//! all that is here is saying where it began and typing what comes back. Waiting between passes
//! is the scheduler's, over [`super::live`]'s event source.

use super::report::{Done, Emit, Failure, Progress};
use super::say;
use mail_runtime::JmapEngine;

/// One pass.
pub(super) async fn drive(
    engine: &mut JmapEngine,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    emit: Emit<'_>,
) -> Result<Done, Failure> {
    // One request round covers the account, so there is nothing finer to say than that it began.
    say(emit, Progress::Connecting);
    typed(engine.pass(cancel, now).await)
}

/// An engine's answer as data, its failure keeping the decision it carries.
fn typed(
    result: Result<mail_runtime::SyncReport, mail_runtime::RuntimeError>,
) -> Result<Done, Failure> {
    result
        .map(Done::from_report)
        .map_err(|e| Failure::of("", &e))
}

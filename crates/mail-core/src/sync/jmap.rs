//! Driving a JMAP account: one pass of [`mail_runtime::JmapEngine`].
//!
//! The pass itself is the engine's — the mailbox list, changes, headers, bodies, the outbox — so
//! all that is here is saying where it began and typing what comes back. Waiting between passes
//! is the scheduler's, over [`super::live`]'s event source.

use super::report::{Done, Emit, Failure, Progress};
use super::{Configured, junk_failed, say};
use mail_runtime::JmapEngine;

/// One pass, with the junk filter judging what it brought before the outbox drains, as an IMAP
/// pass does: the engine has no rules step of its own to do it in.
pub(super) async fn drive(
    engine: &mut JmapEngine,
    account: &Configured,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    emit: Emit<'_>,
) -> Result<Done, Failure> {
    // One request round covers the account, so there is nothing finer to say than that it began.
    say(emit, Progress::Connecting);
    let policy = account.junk;
    let mut filter_failed = None;
    let mut sift = |store: &mail_store::SqliteStore,
                    caps: &mail_domain::AccountCaps,
                    arrived: &[mail_domain::MessageId]| {
        if let Err(e) = crate::junk::sift(store, account.id.clone(), caps, arrived, &policy, now) {
            filter_failed = Some(e);
        }
    };
    let passed = typed(engine.pass(cancel, now, &mut sift).await);
    passed.map(|mut done| {
        if let Some(why) = filter_failed {
            junk_failed(&mut done, why);
        }
        done
    })
}

/// An engine's answer as data, its failure keeping the decision it carries.
fn typed(
    result: Result<mail_runtime::SyncReport, mail_runtime::RuntimeError>,
) -> Result<Done, Failure> {
    result
        .map(Done::from_report)
        .map_err(|e| Failure::of("", &e))
}

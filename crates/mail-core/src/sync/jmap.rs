//! Driving a JMAP account: [`super::drive`]'s loop, around [`mail_runtime::JmapEngine`].
//!
//! The pass itself is the engine's — the mailbox list, changes, headers, bodies, the outbox — so
//! all that is here is the loop a watch runs around it, with the same rules as every protocol's:
//! report each pass, stop on a refused credential, honour a rate limit, then wait the way the
//! server prefers.

use super::report::{Done, Emit, Failure, Progress, Told};
use super::{AfterPass, Announce, Configured, Mode, after_pass, say, waited_in_vain};
use mail_runtime::JmapEngine;
use std::time::Duration;

/// How long a watch sleeps after a push stream ended without news, or on a server with no push.
///
/// Push reports every change as it happens, so this is only the floor for a stream that closed.
/// Without push it is the poll interval, the same five minutes every preset carries.
const FLOOR_WITH_PUSH: Duration = Duration::from_secs(30);
const POLL_WITHOUT_PUSH: Duration = Duration::from_secs(300);

/// One pass, or passes until the process is stopped.
#[allow(clippy::too_many_arguments)]
pub(super) async fn drive(
    engine: &mut JmapEngine,
    account: &Configured,
    cancel: &mut mail_runtime::Cancel,
    now: chrono::DateTime<chrono::Utc>,
    mode: Mode,
    announce: Announce<'_>,
    emit: Emit<'_>,
    told: Told<'_>,
) -> Result<Done, Failure> {
    // One request round covers the account, so there is nothing finer to say than that it began.
    say(emit, Progress::Connecting);
    if mode == Mode::Once {
        return typed(engine.pass(cancel, now).await);
    }
    loop {
        let at = chrono::Utc::now();
        let report = typed(engine.pass(cancel, at).await);
        match after_pass(account, &report, at, announce, told) {
            AfterPass::Stop => return report,
            AfterPass::Hold(wait) => {
                tokio::time::sleep(wait).await;
                continue;
            }
            AfterPass::Wait => {}
        }
        let poll = if engine.pushes() {
            FLOOR_WITH_PUSH
        } else {
            POLL_WITHOUT_PUSH
        };
        if let Err(e) = engine.wait(cancel, poll).await {
            waited_in_vain(account, &e, told).await;
        }
    }
}

/// An engine's answer as data, its failure keeping the decision it carries.
fn typed(
    result: Result<mail_runtime::SyncReport, mail_runtime::RuntimeError>,
) -> Result<Done, Failure> {
    result
        .map(Done::from_report)
        .map_err(|e| Failure::of("", &e))
}

//! The loop that turns a sans-I/O [`Machine`] into work on a socket.
//!
//! This is the whole payoff of the `Progress`/`IoNeed` design, and it is deliberately the only
//! copy. A machine never asks for bytes itself, so cancellation lives here: the loop selects
//! between the transport and a cancel signal, and a machine that is mid-IDLE gets
//! [`IoReady::Interrupt`] and winds down in protocol rather than being dropped on the floor.
//!
//! Every wait on the transport here has a deadline, and running out of time is
//! [`RuntimeError::Stalled`], never a hang: a connection a NAT forgot reads as open for ever.

use crate::error::within;
use crate::{RuntimeError, Transport};
use mail_proto::{IoNeed, IoReady, Machine, Progress};
use std::time::Duration;
use tokio::sync::watch;

/// The longest one exchange may go without a byte moving before the connection is called dead.
///
/// Per read or write, not per operation: a large `FETCH` streams for as long as it streams, and
/// only silence counts. A server that is working answers within seconds; four minutes of nothing
/// is a connection that is gone.
pub const QUIET: Duration = Duration::from_secs(4 * 60);

/// How long the wind-down after an interrupt (`DONE`, `QUIT`) may take.
pub const WIND_DOWN: Duration = Duration::from_secs(60);

/// Told to stop.
///
/// A `watch` rather than a one-shot: several engines share one signal, and a late subscriber
/// still sees that cancellation already happened instead of waiting forever.
pub type Cancel = watch::Receiver<bool>;

/// Drive `machine` to completion over `transport`.
///
/// Returns the machine's output, or the failure that stopped it. On cancellation the machine is
/// given one chance to finish cleanly — that is what `IoReady::Interrupt` is for — and is then
/// abandoned if it asks for more work. Silence longer than [`QUIET`] is
/// [`RuntimeError::Stalled`].
pub async fn drive<M: Machine>(
    machine: &mut M,
    transport: &mut Transport,
    cancel: &mut Cancel,
) -> Result<M::Out, RuntimeError> {
    drive_within(machine, transport, cancel, QUIET).await
}

/// [`drive`], with `quiet` as the longest the connection may say nothing.
///
/// For an exchange that is quiet by design: a held IDLE says nothing until there is news, and
/// is ended from outside before this runs out (`AccountEngine::wait`).
pub async fn drive_within<M: Machine>(
    machine: &mut M,
    transport: &mut Transport,
    cancel: &mut Cancel,
    quiet: Duration,
) -> Result<M::Out, RuntimeError> {
    let mut progress = machine.start();
    let mut winding_down = false;

    loop {
        let needs = match progress {
            Progress::Done(out) => return Ok(out),
            Progress::Failed(e) => return Err(RuntimeError::Proto(e)),
            Progress::Need(needs) => needs,
        };

        if needs.is_empty() {
            // A machine that wants nothing and is not done cannot make progress, and spinning
            // here would burn a core silently. Treat it as the bug it is.
            return Err(RuntimeError::UnsupportedIo(
                "machine returned no needs without finishing".to_owned(),
            ));
        }

        let mut fed = None;
        for need in &needs {
            // TLS upgrade needs the transport by value, which `satisfy` cannot do.
            if let IoNeed::OpenTls { host, .. } = need {
                transport.upgrade(host).await?;
                fed = Some(IoReady::TlsOpen);
                break;
            }

            // Cancellation is checked against the transport, not around the whole loop: a
            // machine parked in IDLE is blocked precisely here, and that is where it must be
            // reachable.
            let outcome = tokio::select! {
                biased;
                changed = cancel.changed(), if !winding_down => {
                    // A closed sender means the owner went away: treat it as cancellation.
                    if changed.is_err() || *cancel.borrow() {
                        winding_down = true;
                        Some(IoReady::Interrupt)
                    } else {
                        continue;
                    }
                }
                result = within(patience(need, quiet), doing(need), transport.satisfy(need)) => {
                    result?
                }
            };

            if let Some(ready) = outcome {
                fed = Some(ready);
                break;
            }
        }

        let Some(ready) = fed else {
            // Every need was a write or a flush. Loop round and let the machine say what next.
            progress = machine.feed(IoReady::Woke);
            continue;
        };

        let interrupted = matches!(ready, IoReady::Interrupt);
        progress = machine.feed(ready);

        if interrupted {
            // One more exchange to let it send DONE or QUIT, then stop. A machine that keeps
            // asking for work after being interrupted does not get to hold the connection.
            match progress {
                Progress::Done(out) => return Ok(out),
                Progress::Failed(e) => return Err(RuntimeError::Proto(e)),
                Progress::Need(ref rest) => {
                    // Bounded too: a dead peer with a full send buffer would otherwise hold
                    // the cancellation itself.
                    let said = async {
                        for need in rest {
                            if let IoNeed::Write(bytes) = need {
                                transport.satisfy(&IoNeed::Write(bytes.clone())).await?;
                            }
                        }
                        Ok::<(), RuntimeError>(())
                    };
                    within(WIND_DOWN, "winding down", said).await?;
                    return Err(RuntimeError::Cancelled);
                }
            }
        }
    }
}

/// How long `need` may take: `quiet`, and a sleep the machine asked for on top of it.
fn patience(need: &IoNeed, quiet: Duration) -> Duration {
    match need {
        IoNeed::Sleep(asked) => quiet.saturating_add(*asked),
        _ => quiet,
    }
}

/// What a stalled `need` was doing, for [`RuntimeError::Stalled`].
fn doing(need: &IoNeed) -> &'static str {
    match need {
        IoNeed::Read => "waiting for its answer",
        IoNeed::Write(_) | IoNeed::Flush | IoNeed::Close | IoNeed::Deflate => "sending to it",
        IoNeed::Sleep(_) => "pausing",
        IoNeed::OpenTls { .. } => "securing the connection",
    }
}

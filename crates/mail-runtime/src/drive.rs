//! The loop that turns a sans-I/O [`Machine`] into work on a socket.
//!
//! This is the whole payoff of the `Progress`/`IoNeed` design, and it is deliberately the only
//! copy. A machine never asks for bytes itself, so cancellation lives here: the loop selects
//! between the transport and a cancel signal, and a machine that is mid-IDLE gets
//! [`IoReady::Interrupt`] and winds down in protocol rather than being dropped on the floor.

use crate::{RuntimeError, Transport};
use mail_proto::{IoNeed, IoReady, Machine, Progress};
use tokio::sync::watch;

/// Told to stop.
///
/// A `watch` rather than a one-shot: several engines share one signal, and a late subscriber
/// still sees that cancellation already happened instead of waiting forever.
pub type Cancel = watch::Receiver<bool>;

/// Drive `machine` to completion over `transport`.
///
/// Returns the machine's output, or the failure that stopped it. On cancellation the machine is
/// given one chance to finish cleanly — that is what `IoReady::Interrupt` is for — and is then
/// abandoned if it asks for more work.
pub async fn drive<M: Machine>(
    machine: &mut M,
    transport: &mut Transport,
    cancel: &mut Cancel,
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
                result = transport.satisfy(need) => result?,
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
                    for need in rest {
                        if let IoNeed::Write(bytes) = need {
                            transport.satisfy(&IoNeed::Write(bytes.clone())).await?;
                        }
                    }
                    return Err(RuntimeError::Cancelled);
                }
            }
        }
    }
}

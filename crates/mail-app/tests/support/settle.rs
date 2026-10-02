//! Waiting for the window, the same way in every native test.
//!
//! The tests run the harness on [`Clock::Virtual`](ds_harness::Clock::Virtual): `advance` moves
//! the harness's own clock, so what a timer, a hold or an animation has done depends only on what
//! the test did, never on how loaded the machine is. What that clock cannot reach is work off the
//! harness's thread: the store's reads run on a blocking thread, which takes real time however far
//! the virtual clock moves. [`settle_until`] is how a test waits for that work: it advances a
//! little virtual time per step, lets the other threads run, and gives up only on a bound of the
//! machine's own time, so a slow runner waits longer instead of failing.

#![allow(dead_code)]

use ds_harness::{Driver, Harness};
use std::time::{Duration, Instant};

/// The most real time a wait may take before the test calls it hung. Far above any runner's
/// slowest settle, far below the CI job's own timeout.
pub const WAIT_BOUND: Duration = Duration::from_secs(30);

/// Advance `harness` 10 ms of its clock at a time until `done` holds, and return the instant (on
/// the harness's clock) it first did. Panics with the document's HTML if `done` has not held
/// after [`WAIT_BOUND`] of real time.
pub fn settle_until(harness: &mut Harness, done: impl Fn(&Harness) -> bool) -> Instant {
    let deadline = Instant::now() + WAIT_BOUND;
    while Instant::now() < deadline {
        if done(harness) {
            return harness.now();
        }
        harness.advance(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        done(harness),
        "settle_until: no state held within {WAIT_BOUND:?}:\n{}",
        harness.html()
    );
    harness.now()
}

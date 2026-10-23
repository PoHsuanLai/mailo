//! Letting a `VirtualDom` finish what it was asked, in the four ways a test waits.
//!
//! Every test module used to carry its own `settle`, and they came in three kinds: render until
//! the dom goes quiet ([`settle_quiet`]), render for a fixed stretch of the clock
//! ([`settle_for`]), and render until the page says it is done ([`settle_until`]). What differed
//! between copies of one kind was a number.
//!
//! The fourth is for a test that looks at a state lasting only until one of quire's timers
//! fires, a row's exit under way: on the wall clock a loaded machine lets the timer run out
//! before the test's next step, so the test installs a [`VirtualClock`] before it builds the dom
//! and moves it itself ([`step_clock_until`], [`step_clock`]). Quire's timers then wait for the
//! test, and everything else (the store, the dom's own tasks) still runs on real time.

use super::dom::Seen;
use dioxus::dioxus_core::{NoOpMutations, VirtualDom};
use ds::base::time::clock::VirtualClock;
use std::time::Duration;

/// Render whatever the dom has pending until it has been quiet for 80 ms, for at most twelve
/// renders. What a test calls after an event whose work is a task or two.
pub(in crate::ui) async fn settle_quiet(dom: &mut VirtualDom) {
    settle_quiet_seen(dom).await;
}

/// [`settle_quiet`], handing back what the renders drew.
pub(in crate::ui) async fn settle_quiet_seen(dom: &mut VirtualDom) -> Seen {
    let mut seen = Seen::default();
    for _ in 0..12 {
        if tokio::time::timeout(Duration::from_millis(80), dom.wait_for_work())
            .await
            .is_err()
        {
            break;
        }
        let mut more = Seen::default();
        dom.render_immediate(&mut more);
        seen = seen.merge(more);
    }
    dom.render_immediate(&mut NoOpMutations);
    seen
}

/// Render for `span` of the clock, however quiet the dom is meanwhile: a test of something that
/// happens after a delay waits for the delay, and a quiet dom is not a finished one.
pub(in crate::ui) async fn settle_for(dom: &mut VirtualDom, seen: &mut Seen, span: Duration) {
    let until = tokio::time::Instant::now() + span;
    loop {
        let left = until.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        let _ = tokio::time::timeout(left, dom.wait_for_work()).await;
        dom.render_immediate(seen);
    }
}

/// Render until the dom has been quiet for `quiet` and `done` says it is finished, for at most
/// `bound` of the machine's own time (a paused tokio clock does not count). Panics with the page
/// if it is not, so a hang reads as one.
pub(in crate::ui) async fn settle_until(
    dom: &mut VirtualDom,
    seen: &mut Seen,
    quiet: Duration,
    bound: Duration,
    done: impl Fn(&VirtualDom) -> bool,
) {
    let started = std::time::Instant::now();
    loop {
        match tokio::time::timeout(quiet, dom.wait_for_work()).await {
            Ok(()) => dom.render_immediate(seen),
            Err(_) if done(dom) => return,
            Err(_) => {}
        }
        assert!(
            started.elapsed() < bound,
            "the page was still working after {bound:?}:\n{}",
            dioxus_ssr::render(dom)
        );
    }
}

/// Step quire's `clock` timer by timer until `done` holds, rendering until quiet after each
/// step, for at most `bound` of that clock. While no timer is waiting, the dom's other work (a
/// store write, a reload) is waited for on real time instead, for at most ten seconds of it.
/// Panics with the page if `done` never holds.
pub(in crate::ui) async fn step_clock_until(
    dom: &mut VirtualDom,
    clock: &VirtualClock,
    bound: Duration,
    done: impl Fn(&VirtualDom) -> bool,
) {
    let end = clock.elapsed().saturating_add(bound);
    let started = std::time::Instant::now();
    loop {
        settle_quiet(dom).await;
        if done(dom) {
            return;
        }
        match clock.next_due().filter(|due| *due <= end) {
            Some(due) => clock.advance_to(due),
            None => assert!(
                started.elapsed() < Duration::from_secs(10),
                "the page never got there within {bound:?} of quire's clock:\n{}",
                dioxus_ssr::render(dom)
            ),
        }
    }
}

/// Move quire's `clock` on by `span`, stopping at each timer due inside it, in order, and at
/// the span's end; at each stop render until quiet and hand `each` the page.
pub(in crate::ui) async fn step_clock(
    dom: &mut VirtualDom,
    clock: &VirtualClock,
    span: Duration,
    mut each: impl FnMut(&str),
) {
    let end = clock.elapsed().saturating_add(span);
    loop {
        settle_quiet(dom).await;
        each(&dioxus_ssr::render(dom));
        match clock.next_due().filter(|due| *due < end) {
            Some(due) => clock.advance_to(due),
            None if clock.elapsed() < end => clock.advance_to(end),
            None => return,
        }
    }
}

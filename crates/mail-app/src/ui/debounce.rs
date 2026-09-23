//! Searching once the box is still, and never drawing what a newer keystroke has superseded.
//!
//! A search over a large mailbox costs tens of milliseconds for a common word, and typing a
//! word is several keystrokes a second, so searching on every keystroke queues work nobody will
//! look at. [`Debounce`] is the rule, pure and driven by a clock the caller hands it: a query
//! settles [`QUIET`] after the last keystroke, and every keystroke has a generation, so a result
//! that comes back for an older one is dropped rather than drawn over the newer text.
//!
//! [`use_debounced`] runs it in a component the way the composer's autosave waits until the
//! page is still: one `use_future` that sleeps and then looks, with the edit counter compared
//! when it wakes.

use crate::view::Shell;
use dioxus::prelude::*;
use std::time::Duration;

/// How long the box must be still before its text is searched for.
pub(super) const QUIET: Duration = Duration::from_millis(150);

/// Text that has been still for [`QUIET`], and the keystroke it came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Settled {
    pub generation: u64,
    pub text: String,
}

/// Whether the text last seen has been searched for yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    /// Typed, and not yet still for long enough.
    Typing,
    /// Settled: there is nothing to wait for.
    Idle,
}

/// The debounce, as a state machine over an elapsed-time clock.
///
/// Times are durations since an origin the caller chooses, so a test can hand it any instant
/// it likes and nothing here reads a clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Debounce {
    quiet: Duration,
    typed: String,
    /// When `typed` last changed.
    since: Duration,
    generation: u64,
    waiting: Waiting,
}

impl Debounce {
    /// A box that holds `text`, already searched for.
    pub(super) fn new(quiet: Duration, text: &str) -> Self {
        Self {
            quiet,
            typed: text.to_owned(),
            since: Duration::ZERO,
            generation: 0,
            waiting: Waiting::Idle,
        }
    }

    /// What was last settled, or the text the box opened with.
    pub(super) fn settled(&self) -> Settled {
        Settled {
            generation: self.generation,
            text: self.typed.clone(),
        }
    }

    /// The box holds `text` at `now`.
    ///
    /// A change is a keystroke: it takes a new generation and restarts the wait. Emptying the
    /// box settles at once, because the place's own list costs nothing to show and nobody waits
    /// to see their search cleared.
    pub(super) fn saw(&mut self, text: &str, now: Duration) -> Option<Settled> {
        if text == self.typed {
            return None;
        }
        self.typed = text.to_owned();
        self.since = now;
        self.generation = self.generation.wrapping_add(1);
        if text.trim().is_empty() {
            self.waiting = Waiting::Idle;
            return Some(self.settled());
        }
        self.waiting = Waiting::Typing;
        None
    }

    /// The text to search for, once, when it has been still for the quiet period by `now`.
    pub(super) fn due(&mut self, now: Duration) -> Option<Settled> {
        if self.waiting == Waiting::Idle || now.saturating_sub(self.since) < self.quiet {
            return None;
        }
        self.waiting = Waiting::Idle;
        Some(self.settled())
    }

    /// How long to sleep before [`Debounce::due`] is worth asking again. With nothing typed,
    /// one quiet period: a keystroke during that sleep is still searched for on time, because
    /// its own quiet period ends after the sleep does.
    pub(super) fn wait(&self, now: Duration) -> Duration {
        match self.waiting {
            Waiting::Idle => self.quiet,
            Waiting::Typing => self.quiet.saturating_sub(now.saturating_sub(self.since)),
        }
    }

    /// Whether a result for `generation` is still the one the box is waiting for.
    pub(super) fn is_latest(&self, generation: u64) -> bool {
        generation == self.generation
    }
}

/// The debounce on one of the shell's text boxes, running.
#[derive(Clone, Copy)]
pub(super) struct Debounced {
    /// The text to search for. Changes once per pause, not once per keystroke.
    pub settled: Signal<Settled>,
    machine: CopyValue<Debounce>,
}

impl Debounced {
    /// Whether a result for `generation` may be drawn. A newer keystroke has superseded it
    /// when not.
    pub(super) fn is_latest(&self, generation: u64) -> bool {
        self.machine.peek().is_latest(generation)
    }
}

/// Debounce the box `pick` reads out of `shell`.
///
/// Every change reaches the machine as it happens, through an effect, so a result is known to
/// be stale from the keystroke on. The wait is the composer autosave's timer, reused: one
/// `use_future` owned by the component, sleeping and then looking. Not a `spawn` from the
/// render body and not a task per keystroke: F140 is a future spawned from a component body
/// that is never polled, and this is the shape of timer the window is known to run.
pub(super) fn use_debounced(shell: Signal<Shell>, pick: fn(&Shell) -> String) -> Debounced {
    let origin = use_hook(tokio::time::Instant::now);
    let mut machine = use_hook(|| CopyValue::new(Debounce::new(QUIET, &pick(&shell.peek()))));
    let mut settled = use_signal(|| machine.peek().settled());
    use_effect(move || {
        let text = pick(&shell.read());
        let done = machine.write().saw(&text, origin.elapsed());
        if let Some(done) = done {
            settled.set(done);
        }
    });
    use_future(move || async move {
        loop {
            let wait = machine.peek().wait(origin.elapsed());
            tokio::time::sleep(wait).await;
            let done = machine.write().due(origin.elapsed());
            if let Some(done) = done {
                settled.set(done);
            }
        }
    });
    Debounced { settled, machine }
}

#[cfg(test)]
#[path = "debounce_tests.rs"]
mod tests;

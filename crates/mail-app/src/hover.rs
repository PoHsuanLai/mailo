//! When a hover card opens, when it closes, and whether the window is warm.
//!
//! Pure. Every method takes the moment it happens, so a test drives it with a fake clock and
//! the window drives it with its own: the window only has to call [`Timer::tick`] at
//! [`Timer::deadline`].
//!
//! The rules, from the F6 mockup:
//!
//! - a card opens after [`OPEN`] of rest on one hook;
//! - it closes [`CLOSE`] after the pointer leaves, and the pointer may travel into the card in
//!   that time without closing it;
//! - after a close the window is warm for [`WARM`], and while it is warm (or while a card is
//!   showing) the next hook opens at once, so moving down a list scrubs through previews;
//! - entering a hook replaces the one before it, which is how the innermost hook wins: the
//!   caller reports the sender name inside a row, not the row.

/// A moment, in milliseconds since the window's own epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct At(pub u64);

impl At {
    fn after(self, millis: u64) -> At {
        At(self.0.saturating_add(millis))
    }
}

/// Rest before a card opens, in milliseconds.
pub const OPEN: u64 = 450;
/// Grace after the pointer leaves, in which it may reach the card.
pub const CLOSE: u64 = 150;
/// How long the window stays warm after a card closes.
pub const WARM: u64 = 400;

/// Where the one card is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase<K> {
    Idle,
    /// The pointer rests on `key`; the card opens at `due`.
    Waiting {
        key: K,
        due: At,
    },
    Open {
        key: K,
    },
    /// The pointer left; the card closes at `due` unless it comes back.
    Leaving {
        key: K,
        due: At,
    },
}

/// The hover state machine for one window. `K` names a hook: a row, a sender, a tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timer<K> {
    phase: Phase<K>,
    /// The window is warm until this moment.
    warm_until: Option<At>,
}

impl<K> Default for Timer<K> {
    fn default() -> Self {
        Timer {
            phase: Phase::Idle,
            warm_until: None,
        }
    }
}

impl<K: Clone + PartialEq> Timer<K> {
    /// The pointer came to rest on `key`.
    pub fn enter(&mut self, key: K, now: At) {
        match &self.phase {
            Phase::Open { key: open } | Phase::Leaving { key: open, .. } if *open == key => {
                self.phase = Phase::Open { key };
                return;
            }
            Phase::Waiting { key: waiting, .. } if *waiting == key => return,
            _ => {}
        }
        let showing = matches!(self.phase, Phase::Open { .. } | Phase::Leaving { .. });
        self.phase = if showing || self.warm(now) {
            Phase::Open { key }
        } else {
            Phase::Waiting {
                key,
                due: now.after(OPEN),
            }
        };
    }

    /// The pointer left the hook it was on.
    pub fn leave(&mut self, now: At) {
        self.phase = match std::mem::replace(&mut self.phase, Phase::Idle) {
            Phase::Waiting { .. } | Phase::Idle => Phase::Idle,
            Phase::Open { key } => Phase::Leaving {
                key,
                due: now.after(CLOSE),
            },
            leaving @ Phase::Leaving { .. } => leaving,
        };
    }

    /// The pointer reached the card itself.
    pub fn enter_card(&mut self) {
        if let Phase::Leaving { key, .. } = &self.phase {
            self.phase = Phase::Open { key: key.clone() };
        }
    }

    /// The pointer left the card. The same grace as leaving the hook.
    pub fn leave_card(&mut self, now: At) {
        self.leave(now);
    }

    /// Close at once: a click, Esc, or an action that made the card stale.
    pub fn dismiss(&mut self, now: At) {
        if matches!(self.phase, Phase::Open { .. } | Phase::Leaving { .. }) {
            self.warm_until = Some(now.after(WARM));
        }
        self.phase = Phase::Idle;
    }

    /// Let time pass. Opens a card whose rest is over and closes one whose grace is.
    pub fn tick(&mut self, now: At) {
        match &self.phase {
            Phase::Waiting { key, due } if *due <= now => {
                self.phase = Phase::Open { key: key.clone() };
            }
            Phase::Leaving { due, .. } if *due <= now => {
                self.phase = Phase::Idle;
                self.warm_until = Some(now.after(WARM));
            }
            _ => {}
        }
    }

    /// The hook whose card is showing, including during its close grace.
    pub fn open(&self) -> Option<&K> {
        match &self.phase {
            Phase::Open { key } | Phase::Leaving { key, .. } => Some(key),
            Phase::Idle | Phase::Waiting { .. } => None,
        }
    }

    /// Whether a card closed less than [`WARM`] ago.
    pub fn warm(&self, now: At) -> bool {
        self.warm_until.is_some_and(|until| now < until)
    }

    /// The next moment [`Self::tick`] or [`Self::warm`] would answer differently, if any.
    ///
    /// The window sleeps until then. A warm window has one too, so the one `warm` signal the
    /// shell holds is cleared on time rather than on the next pointer movement.
    pub fn deadline(&self, now: At) -> Option<At> {
        let phase = match &self.phase {
            Phase::Waiting { due, .. } | Phase::Leaving { due, .. } => Some(*due),
            Phase::Idle | Phase::Open { .. } => None,
        };
        let warm = self.warm_until.filter(|until| now < *until);
        match (phase, warm) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests;

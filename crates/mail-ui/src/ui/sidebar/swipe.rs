//! Arc's Space swipe as a pure machine: two fingers moving sideways over the sidebar carry its
//! content with them, and the swipe ends on the Space beside this one, or back on this one.
//!
//! What it hears is the window's scroll, as the host publishes every wheel and touchpad event
//! (`ds::host::gesture`): a touchpad's pixels on a Mac, a precision touchpad on Windows and
//! libinput on Wayland and X11 alike, and a wheel's detents. Nothing here is one platform's
//! gesture.
//!
//! - **Its axis.** A gesture is locked to the axis it moved most on once it has moved
//!   [`LOCK_PX`], the engine's own touchpad lock (design/11 §11.3.9), so the gesture a list
//!   scrolls for is never also a swipe. A wheel's detent locks at once.
//! - **Following.** Locked across, the content follows the fingers 1:1 towards a Space there is,
//!   and as a short rubber band towards an end of the list ([`drawn`]).
//! - **The switch.** Past [`COMMIT_SHARE`] of the sidebar's width, or moving faster than
//!   [`FLICK`] once past [`FLICK_MIN`], it switches; a sideways wheel's detent is a step of its
//!   own. Fingers that lift under both spring back.
//! - **One Space a gesture.** Decided, it hears nothing more until the hand has been still for
//!   `DelayToken::SwipeQuiet` (the same quiet quire's swipe to dismiss waits for): a touchpad's
//!   momentum after the lift, which a Mac sends as more of the same scroll, never chains a second
//!   switch. Fingers that touch down again (`Began`) are a new gesture at once.
//!
//! Where a source carries no phases (a Windows touchpad, an X11 smooth scroll), the quiet is the
//! gesture's end as well: it springs back then.

use ds::base::geometry::units::Px;
use ds::base::machine::{Elapsed, Machine};
use ds::base::time::stamp::Stamp;
use ds::host::gesture::GesturePhase;
use ds::motion::rubber::resist;
use ds::style::appearance::motion::MotionLevel;
use ds::style::tokens::delay::DelayToken;

/// How far a gesture moves before its axis is locked, px: the engine's touchpad lock.
pub(super) const LOCK_PX: f32 = 10.0;

/// The share of the sidebar's width a swipe carries the content before it switches: a third.
pub(super) const COMMIT_SHARE: f32 = 1.0 / 3.0;

/// How fast a swipe must be moving to switch short of the distance, px/s.
pub(super) const FLICK: f32 = 900.0;

/// How far a fast swipe must have carried the content before its speed counts, px: a twitch is
/// not a flick.
pub(super) const FLICK_MIN: f32 = 24.0;

/// A wheel's sideways turn at least this far is one detent, a step of its own, px. A detent is
/// 60 px as the host reports it; a Windows precision touchpad's fractions of one are far less,
/// and follow as a touchpad does.
pub(super) const DETENT_PX: f32 = 30.0;

/// The furthest the content is drawn towards an end of the list, px: a small band.
pub(super) const RUBBER_MAX: f32 = 24.0;

/// How long the last moves are that a flick's speed is measured over, ms: quire's release
/// window.
fn speed_window() -> u64 {
    u64::try_from(DelayToken::ReleaseWindow.delay().as_millis()).unwrap_or(u64::MAX)
}

/// The Space a swipe goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Toward {
    /// The Space before this one in the sidebar's order: the content carried to the right.
    Before,
    /// The Space after it: the content carried to the left.
    After,
}

impl Toward {
    /// The way content carried `raw` (right positive) goes.
    fn of(raw: f32) -> Toward {
        if raw > 0.0 {
            Toward::Before
        } else {
            Toward::After
        }
    }
}

/// How a scroll arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Turn {
    /// A continuous run: a touchpad's pixels, or a wheel's fraction of a detent.
    Glide,
    /// A wheel's whole detent: a step.
    Detent,
}

/// What moves the machine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum SwipeIn {
    /// A scroll over the sidebar: `dx` rightwards and `dy` downwards, as the content travels.
    Scroll {
        dx: Px,
        dy: Px,
        phase: GesturePhase,
        turn: Turn,
    },
    /// The quiet after the last scroll has run out.
    Elapsed,
}

impl From<Elapsed> for SwipeIn {
    fn from(_: Elapsed) -> Self {
        SwipeIn::Elapsed
    }
}

/// What the sidebar does after a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum SwipeOut {
    /// Switch to the Space beside this one.
    Switch(Toward),
}

/// What a step reads of the sidebar: how wide it is and which Spaces stand beside this one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Room {
    pub width: Px,
    /// Whether a Space stands before this one.
    pub before: bool,
    /// Whether a Space stands after this one.
    pub after: bool,
}

impl Room {
    /// Whether there is a Space `toward`.
    fn has(&self, toward: Toward) -> bool {
        match toward {
            Toward::Before => self.before,
            Toward::After => self.after,
        }
    }
}

/// Where the content was carried to, and when.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    raw: f32,
    at: Stamp,
}

/// The last few samples of a swipe, newest last, for its speed.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Recent([Option<Sample>; 4]);

impl Recent {
    fn push(self, sample: Sample) -> Recent {
        let [_, a, b, c] = self.0;
        Recent([a, b, c, Some(sample)])
    }

    /// The speed over the samples within the release window of the newest, px/s: nothing over
    /// one sample, or over no time.
    fn speed(&self) -> f32 {
        let Some(last) = self.0[3] else {
            return 0.0;
        };
        let window = speed_window();
        let Some(oldest) = self
            .0
            .iter()
            .flatten()
            .find(|sample| last.at.since(sample.at) <= window)
        else {
            return 0.0;
        };
        let ms = last.at.since(oldest.at);
        if ms == 0 {
            return 0.0;
        }
        (last.raw - oldest.raw) * 1000.0 / ms as f32
    }

    /// The speed at a release at `at`: nothing when the hand had stopped before it lifted.
    fn released(&self, at: Stamp) -> f32 {
        match self.0[3] {
            Some(last) if at.since(last.at) <= speed_window() => self.speed(),
            _ => 0.0,
        }
    }
}

/// Where a gesture is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// No gesture.
    Idle,
    /// Moving, its axis not locked yet: how far on each, and `raw` across, signed.
    Undecided { across: f32, along: f32, raw: f32 },
    /// Locked across: the content carried `raw` (right positive).
    Across { raw: f32, recent: Recent },
    /// A vertical scroll, the list's; or a swipe already decided. Heard and let pass until the
    /// hand is still, or touches down again.
    Spent,
}

/// The Space swipe: where its gesture is, and until when it waits for the hand to be still.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct SpaceSwipe {
    stage: Stage,
    quiet_until: Stamp,
}

impl Default for SpaceSwipe {
    fn default() -> Self {
        SpaceSwipe {
            stage: Stage::Idle,
            quiet_until: Stamp(0),
        }
    }
}

impl SpaceSwipe {
    /// How far the fingers carried the content across (right positive), while they hold it.
    pub(super) fn held(&self) -> Option<Px> {
        match self.stage {
            Stage::Across { raw, .. } => Some(Px(raw)),
            Stage::Idle | Stage::Undecided { .. } | Stage::Spent => None,
        }
    }

    fn at(stage: Stage, quiet_until: Stamp) -> SpaceSwipe {
        SpaceSwipe { stage, quiet_until }
    }
}

impl Machine for SpaceSwipe {
    type In = SwipeIn;
    type Out = SwipeOut;
    type Params = ();
    type Ctx = Room;

    fn step(self, input: SwipeIn, at: Stamp, _: &(), room: &Room) -> (Self, Vec<SwipeOut>) {
        let SwipeIn::Scroll {
            dx,
            dy,
            phase,
            turn,
        } = input
        else {
            return match self.wake() {
                Some(until) if at >= until => (SpaceSwipe::default(), Vec::new()),
                _ => (self, Vec::new()),
            };
        };
        let quiet = at.after_span(DelayToken::SwipeQuiet.delay());
        let fresh = Stage::Undecided {
            across: 0.0,
            along: 0.0,
            raw: 0.0,
        };
        let from = match (phase, self.stage) {
            // The system took the gesture away: forget it, and spring back.
            (GesturePhase::Cancelled, _) => return (SpaceSwipe::default(), Vec::new()),
            // Fingers touching down are a new gesture, whatever the last one's tail was doing.
            (GesturePhase::Began, _) | (_, Stage::Idle) => fresh,
            (_, stage) => stage,
        };
        let stage = match from {
            Stage::Undecided { across, along, raw } => {
                let (across, along, raw) = (across + dx.0.abs(), along + dy.0.abs(), raw + dx.0);
                let locked = turn == Turn::Detent || across.max(along) >= LOCK_PX;
                match (locked, across > along) {
                    (false, _) => Stage::Undecided { across, along, raw },
                    (true, true) => Stage::Across {
                        raw,
                        recent: Recent::default().push(Sample { raw, at }),
                    },
                    (true, false) => Stage::Spent,
                }
            }
            Stage::Across { raw, recent } => {
                let raw = raw + dx.0;
                Stage::Across {
                    raw,
                    recent: recent.push(Sample { raw, at }),
                }
            }
            Stage::Idle | Stage::Spent => Stage::Spent,
        };
        let lifted = phase == GesturePhase::Ended;
        let Stage::Across { raw, recent } = stage else {
            // Lifted before the axis was known: nothing was carried.
            let stage = match (lifted, stage) {
                (true, Stage::Undecided { .. }) => Stage::Spent,
                (_, stage) => stage,
            };
            return (SpaceSwipe::at(stage, quiet), Vec::new());
        };
        let toward = Toward::of(raw);
        let speed = if lifted {
            recent.released(at)
        } else {
            recent.speed()
        };
        let far = raw.abs() >= room.width.0 * COMMIT_SHARE;
        let fast = raw.abs() >= FLICK_MIN && Toward::of(speed) == toward && speed.abs() >= FLICK;
        if room.has(toward) && (turn == Turn::Detent || far || fast) {
            return (
                SpaceSwipe::at(Stage::Spent, quiet),
                vec![SwipeOut::Switch(toward)],
            );
        }
        // Lifted under both: back to this Space, and the momentum that follows is not heard.
        let stage = if lifted { Stage::Spent } else { stage };
        (SpaceSwipe::at(stage, quiet), Vec::new())
    }

    fn wake(&self) -> Option<Stamp> {
        match self.stage {
            Stage::Idle => None,
            Stage::Undecided { .. } | Stage::Across { .. } | Stage::Spent => Some(self.quiet_until),
        }
    }
}

/// Where the content is drawn for a swipe that carried it `raw` (right positive) in `room`: 1:1
/// towards a Space there is, up to the sidebar's width, and a short rubber band towards an end
/// of the list. Under Reduced motion it is not carried at all; the switch alone shows the swipe.
pub(super) fn drawn(raw: Px, room: &Room, level: MotionLevel) -> Px {
    if level == MotionLevel::Reduced {
        return Px(0.0);
    }
    let reach = raw.0.abs();
    let drawn = if room.has(Toward::of(raw.0)) {
        reach.min(room.width.0)
    } else {
        resist(Px(reach), level).0.min(RUBBER_MAX)
    };
    Px(drawn.copysign(raw.0))
}

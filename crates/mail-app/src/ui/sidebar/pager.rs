//! The Space swipe in the sidebar: [`super::swipe`]'s machine fed from the window's scroll, the
//! content it carries drawn and sprung home, and the slide a switch plays.
//!
//! The scroll is heard from the host's gesture bus (`ds::host::gesture`), not from the
//! document's `onwheel`: the window's scroll engine takes a wheel over a scroller before the
//! document sees it, and the bus hears every one, with the touchpad's phases where the platform
//! gives them. A gesture is the sidebar's when it starts over it; the vertical ones go on
//! scrolling the lists as before, since nothing here takes the wheel from the engine.
//!
//! A switch is the Spaces' own (`Handle::switch_index`), as a click on a dot in the foot is, so the
//! Space shows where it was left the same way. Every switch, a swipe's or a click's, plays quire's
//! Space slide (`Anim::SlideR`, `Anim::SlideL`) on the sidebar's content, from the side the Space
//! stands on; under Reduced motion quire's stylesheet makes it a fade.

use super::swipe::{DETENT_PX, Room, SpaceSwipe, SwipeIn, SwipeOut, Toward, Turn, drawn};
use crate::ui::space::{Handle, SpaceId};
use dioxus::core::queue_effect;
use dioxus::prelude::*;
use ds::base::geometry::units::{Point, Px, Rect};
use ds::base::machine::Machine;
use ds::components::app::spaces::SlideIn;
use ds::host::gesture::{Gesture, ScrollSource, use_gestures};
use ds::host::measure::{RectProbe, use_rect};
use ds::motion::anim::Anim;
use ds::motion::detail::touch::Touch;
use ds::motion::machine::{MachineRef, use_machine};
use ds::motion::pulse_key::PulseKey;
use ds::motion::spring_spec::SpringSpec;
use ds::motion::timeline::spring::PxPerUnit;
use ds::motion::use_spring::{SpringMotion, use_spring_motion};
use ds::style::appearance::motion::MotionLevel;

/// What the last render drew of the carried content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carried {
    /// At its place, or springing home.
    Rest,
    /// Following the fingers.
    Held,
    /// Switched away from: the new Space's content arrives at its place, sliding in.
    Switched,
}

/// The slide the last switch played, and which Space it brought.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Arrival {
    seen: SpaceId,
    pulse: Option<PulseKey>,
}

/// One sidebar's Space swipe. `Copy`: signals and cells only.
#[derive(Clone, Copy)]
pub(super) struct Pager {
    machine: MachineRef<SpaceSwipe>,
    probe: RectProbe,
    spring: SpringMotion,
    carried: CopyValue<Carried>,
    arrival: CopyValue<Arrival>,
    handle: Handle,
}

/// The sidebar's Space swipe over `handle`'s Spaces. The sidebar hands its element's
/// `onmounted` to [`Pager::on_mounted`], which is where the swipe is heard and how wide it is.
pub(super) fn use_pager(handle: Handle) -> Pager {
    let probe = use_rect();
    let mut carried = use_hook(|| CopyValue::new(Carried::Rest));
    let machine = use_machine(
        |_| SpaceSwipe::default(),
        (),
        move || room(handle, probe),
        move |SwipeOut::Switch(toward), _| {
            let Some(to) = neighbour(handle, toward) else {
                return;
            };
            if handle.switch_index(to).is_some() {
                carried.set(Carried::Switched);
            }
        },
    );
    use_gestures(move |gesture| {
        let Gesture::Scroll {
            source,
            phase,
            by,
            at,
            held,
        } = gesture
        else {
            return;
        };
        // Under a modifier a wheel is a zoom, or the list's own sideways scroll: not a swipe.
        if !held.is_empty() {
            return;
        }
        // A gesture under way is followed wherever the pointer is; a new one starts over the
        // sidebar only.
        let under_way = machine.state().peek().wake().is_some();
        let over = probe.rect().is_some_and(|rect| inside(rect, at));
        if !(under_way || over) {
            return;
        }
        let turn = match source {
            ScrollSource::Wheel if by.x.0.abs() >= DETENT_PX => Turn::Detent,
            ScrollSource::Wheel | ScrollSource::Finger => Turn::Glide,
        };
        machine.send(SwipeIn::Scroll {
            dx: by.x,
            dy: by.y,
            phase,
            turn,
        });
    });
    Pager {
        machine,
        probe,
        spring: use_spring_motion(0.0, PxPerUnit(1.0)),
        carried,
        arrival: use_hook(|| {
            CopyValue::new(Arrival {
                seen: handle.current(),
                pulse: None,
            })
        }),
        handle,
    }
}

impl Pager {
    /// The sidebar's `onmounted`: its rect, kept through every layout, is where a swipe starts
    /// and how far one must carry the content.
    pub(super) fn on_mounted(&self, event: MountedEvent) {
        self.probe.on_mounted(event);
    }

    /// `--space-dx`, the carried content's offset, while it is off its place: following the
    /// fingers, then springing home from where they left it. Call once per render: it moves
    /// the spring.
    pub(super) fn offset(&self, level: MotionLevel) -> Option<String> {
        let state = *self.machine.state().read();
        let (spring, mut carried) = (self.spring, self.carried);
        let before = *carried.peek();
        let at = match state.held() {
            Some(raw) => {
                let at = drawn(raw, &room(self.handle, self.probe), level).0;
                carried.set(Carried::Held);
                queue_effect(move || spring.track(at));
                at
            }
            None => {
                carried.set(Carried::Rest);
                match before {
                    Carried::Held => {
                        queue_effect(move || spring.go(0.0, SpringSpec::for_touch(Touch::Remote)));
                        spring.frame().position()
                    }
                    // The Space the content was carried towards is on screen: it slides in at
                    // its place, and the old offset is not carried over to it.
                    Carried::Switched => {
                        queue_effect(move || spring.snap(0.0));
                        0.0
                    }
                    Carried::Rest => spring.frame().position(),
                }
            }
        };
        let rounded = (at * 100.0).round() / 100.0;
        (rounded != 0.0).then(|| format!("--space-dx:{rounded}px"))
    }

    /// The class and `data-pulse` of the slide the last switch plays on the sidebar's content,
    /// once a switch has happened: quire's Space slide from the side the new Space is on,
    /// restarted under its other alias by the next switch.
    pub(super) fn arrival(&self) -> Option<(String, &'static str)> {
        let now = self.handle.current();
        let mut arrival = self.arrival;
        let last = *arrival.peek();
        if last.seen == now {
            return last.pulse.and_then(PulseKey::attrs);
        }
        let anim = match self.handle.slide() {
            Some(SlideIn::Left) => Anim::SlideL,
            Some(SlideIn::Right) | None => Anim::SlideR,
        };
        let pulse = match last.pulse {
            Some(key) if key.anim() == anim => key.fired(),
            _ => PulseKey::rest(anim).fired(),
        };
        arrival.set(Arrival {
            seen: now,
            pulse: Some(pulse),
        });
        pulse.attrs()
    }
}

/// What a step reads of the sidebar: its width as laid out (the preferred width before it is),
/// and the Spaces beside the current one. While a part of a Space's menu is open a switch is
/// refused, so there is none to swipe to.
fn room(handle: Handle, probe: RectProbe) -> Room {
    let width = probe
        .rect()
        .map_or(Px(crate::ui::app::SIDEBAR_WIDTH), |rect| rect.size.width);
    let spaces = handle.spaces();
    let spaces = spaces.peek();
    let index = spaces.index_of(spaces.current().id).unwrap_or(0);
    let free = handle.editing().is_none();
    Room {
        width,
        before: free && index > 0,
        after: free && index + 1 < spaces.list().len(),
    }
}

/// The index of the Space `toward` the current one, if there is one.
fn neighbour(handle: Handle, toward: Toward) -> Option<usize> {
    let spaces = handle.spaces();
    let spaces = spaces.peek();
    let index = spaces.index_of(spaces.current().id)?;
    let to = match toward {
        Toward::Before => index.checked_sub(1)?,
        Toward::After => index + 1,
    };
    (to < spaces.list().len()).then_some(to)
}

/// Whether `at` is inside `rect`.
fn inside(rect: Rect, at: Point) -> bool {
    let (x, y) = (at.x.0 - rect.origin.x.0, at.y.0 - rect.origin.y.0);
    (0.0..=rect.size.width.0).contains(&x) && (0.0..=rect.size.height.0).contains(&y)
}

//! The Space swipe as data: the axis it locks to, the distance and the speed that switch, one
//! Space a gesture whatever momentum follows, and the ends of the list.

use super::swipe::{
    COMMIT_SHARE, DETENT_PX, FLICK_MIN, RUBBER_MAX, Room, SpaceSwipe, SwipeIn, SwipeOut, Toward,
    Turn, drawn,
};
use ds::base::geometry::units::Px;
use ds::base::machine::Machine;
use ds::base::time::stamp::Stamp;
use ds::host::gesture::GesturePhase::{self, Began, Cancelled, Changed, Ended};
use ds::style::appearance::motion::MotionLevel;

// The cases below lean on these: the twitch's 20 px is under a flick's least distance, and the
// host's 60 px detent is a step.
const _: () = assert!(FLICK_MIN > 20.0 && DETENT_PX < 60.0);

/// The sidebar at its preferred width, a Space on either side.
const MIDDLE: Room = Room {
    width: Px(232.0),
    before: true,
    after: true,
};

/// The only Space.
const ALONE: Room = Room {
    width: Px(232.0),
    before: false,
    after: false,
};

/// One scroll: `dx`, `dy`, its phase, and the ms since the last.
type Step = (f32, f32, GesturePhase, u64);

/// A gesture's steps run through a fresh machine in `room`, a glide's: its switches, and the
/// machine as the last step left it, at the time it was left.
fn run(room: Room, steps: &[Step]) -> (Vec<SwipeOut>, SpaceSwipe, Stamp) {
    run_from(
        SpaceSwipe::default(),
        Stamp(1_000),
        room,
        steps,
        Turn::Glide,
    )
}

fn run_from(
    start: SpaceSwipe,
    from: Stamp,
    room: Room,
    steps: &[Step],
    turn: Turn,
) -> (Vec<SwipeOut>, SpaceSwipe, Stamp) {
    let (mut swipe, mut at, mut outs) = (start, from, Vec::new());
    for &(dx, dy, phase, after) in steps {
        at = Stamp(at.0 + after);
        let input = SwipeIn::Scroll {
            dx: Px(dx),
            dy: Px(dy),
            phase,
            turn,
        };
        let (next, out) = swipe.step(input, at, &(), &room);
        swipe = next;
        outs.extend(out);
    }
    (outs, swipe, at)
}

/// `n` moves of `dx` across, 16 ms apart: a steady glide.
fn glide(n: usize, dx: f32) -> Vec<Step> {
    vec![(dx, 0.0, Changed, 16); n]
}

/// The machine once its quiet has run out.
fn quieted(swipe: SpaceSwipe, room: Room) -> SpaceSwipe {
    let until = swipe.wake().expect("a gesture waits for the quiet");
    swipe.step(SwipeIn::Elapsed, until, &(), &room).0
}

#[test]
fn a_gesture_locks_to_the_axis_it_moved_most_on() {
    // name, steps, held across once locked
    let cases: &[(&str, Vec<Step>, Option<f32>)] = &[
        (
            "sideways",
            vec![(4.0, 1.0, Began, 0), (8.0, 2.0, Changed, 16)],
            Some(12.0),
        ),
        (
            "down",
            vec![(1.0, 6.0, Began, 0), (2.0, 8.0, Changed, 16)],
            None,
        ),
        (
            "not far enough yet",
            vec![(3.0, 1.0, Began, 0), (3.0, 1.0, Changed, 16)],
            None,
        ),
        // Locked down by its first moves: the list's, however far it drifts sideways after.
        (
            "a diagonal mostly down",
            vec![(4.0, 12.0, Changed, 0), (60.0, 0.0, Changed, 16)],
            None,
        ),
    ];
    for (name, steps, want) in cases {
        let (outs, swipe, _) = run(MIDDLE, steps);
        assert!(outs.is_empty(), "{name}: switched");
        assert_eq!(swipe.held().map(|px| px.0), *want, "{name}");
    }
}

#[test]
fn a_vertical_scroll_never_switches() {
    let steps: Vec<Step> = vec![(0.0, -40.0, Changed, 16); 20];
    let (outs, swipe, _) = run(MIDDLE, &steps);
    assert!(outs.is_empty());
    assert_eq!(swipe.held(), None);
}

#[test]
fn past_a_third_of_the_width_it_switches_and_short_of_it_springs_back() {
    let third = MIDDLE.width.0 * COMMIT_SHARE;
    // Slow moves, 60 ms apart: under the flick, so distance alone decides.
    let slow = |total: f32| -> Vec<Step> {
        let mut steps = vec![(0.0, 0.0, Began, 0)];
        steps.extend(vec![(total / 8.0, 0.0, Changed, 60); 8]);
        steps.push((0.0, 0.0, Ended, 60));
        steps
    };
    // name, distance (right positive), switches
    let cases: &[(&str, f32, Vec<SwipeOut>)] = &[
        (
            "left past a third",
            -(third + 8.0),
            vec![SwipeOut::Switch(Toward::After)],
        ),
        (
            "right past a third",
            third + 8.0,
            vec![SwipeOut::Switch(Toward::Before)],
        ),
        ("left short of it", -(third - 16.0), vec![]),
        ("right short of it", third - 16.0, vec![]),
    ];
    for (name, total, want) in cases {
        let (outs, swipe, _) = run(MIDDLE, &slow(*total));
        assert_eq!(outs, *want, "{name}");
        assert_eq!(swipe.held(), None, "{name}: still held after the lift");
    }
}

#[test]
fn a_flick_switches_short_of_the_distance() {
    // name, steps, switches
    let cases: &[(&str, Vec<Step>, Vec<SwipeOut>)] = &[
        // 12 px a frame is 750 px/s: under the flick, so a short one springs back.
        ("steady", glide(4, -12.0), vec![]),
        // 20 px a frame is 1250 px/s.
        (
            "fast",
            glide(3, -20.0),
            vec![SwipeOut::Switch(Toward::After)],
        ),
        // Fast, but so short it is a twitch: under the flick's least distance.
        (
            "a twitch",
            vec![(-10.0, 0.0, Changed, 0), (-10.0, 0.0, Changed, 4)],
            vec![],
        ),
    ];
    for (name, steps, want) in cases {
        let mut steps = steps.clone();
        steps.push((0.0, 0.0, Ended, 0));
        assert_eq!(run(MIDDLE, &steps).0, *want, "{name}");
    }
    // Lifted after the hand had stopped: its old speed does not count.
    let mut stopped = glide(3, -12.0);
    stopped.push((0.0, 0.0, Ended, 400));
    assert_eq!(run(MIDDLE, &stopped).0, []);
}

#[test]
fn one_gesture_moves_one_space_and_its_momentum_is_ignored() {
    // A long fast swipe, then a Mac's momentum after the lift: Changed with no Began.
    let mut steps = glide(12, -30.0);
    steps.push((0.0, 0.0, Ended, 16));
    steps.extend(glide(40, -25.0));
    let (outs, swipe, at) = run(MIDDLE, &steps);
    assert_eq!(outs, [SwipeOut::Switch(Toward::After)]);
    assert_eq!(swipe.held(), None, "the tail moved the content");
    // The tail goes on while the hand is not still; then the next gesture is heard.
    assert!(swipe.wake().is_some_and(|until| until > at));
    let rested = quieted(swipe, MIDDLE);
    assert_eq!(rested, SpaceSwipe::default());
    let (again, _, _) = run_from(rested, at, MIDDLE, &glide(12, -30.0), Turn::Glide);
    assert_eq!(again, [SwipeOut::Switch(Toward::After)]);
}

#[test]
fn a_short_swipe_s_momentum_does_not_carry_it_further() {
    // Lifted short of the distance, then its momentum: it springs back, and the tail, however
    // long, is not a second try.
    let mut steps = vec![(0.0, 0.0, Began, 0)];
    steps.extend(vec![(-8.0, 0.0, Changed, 40); 4]);
    steps.push((0.0, 0.0, Ended, 16));
    steps.extend(glide(30, -6.0));
    let (outs, swipe, _) = run(MIDDLE, &steps);
    assert!(outs.is_empty());
    assert_eq!(swipe.held(), None);
}

#[test]
fn fingers_touching_down_again_start_a_new_gesture_at_once() {
    let mut steps = glide(12, -30.0);
    steps.push((0.0, 0.0, Ended, 16));
    steps.extend(glide(3, -25.0));
    steps.push((0.0, 0.0, Began, 16));
    steps.extend(glide(12, -30.0));
    let (outs, _, _) = run(MIDDLE, &steps);
    assert_eq!(outs, [SwipeOut::Switch(Toward::After); 2]);
}

#[test]
fn a_wheel_s_sideways_detent_is_one_step() {
    let detent = |dx: f32| vec![(dx, 0.0, Changed, 0)];
    let step = |room: Room, dx: f32| {
        run_from(
            SpaceSwipe::default(),
            Stamp(1_000),
            room,
            &detent(dx),
            Turn::Detent,
        )
    };
    assert_eq!(step(MIDDLE, -60.0).0, [SwipeOut::Switch(Toward::After)]);
    assert_eq!(step(MIDDLE, 60.0).0, [SwipeOut::Switch(Toward::Before)]);
    assert_eq!(step(ALONE, 60.0).0, [], "no Space before the first");
    // A wheel held over, turning detent after detent: one step until it rests.
    let (_, swipe, at) = step(MIDDLE, -60.0);
    let held = vec![(-60.0, 0.0, Changed, 40); 5];
    let (outs, swipe, _) = run_from(swipe, at, MIDDLE, &held, Turn::Detent);
    assert!(outs.is_empty());
    let (outs, _, _) = run_from(
        quieted(swipe, MIDDLE),
        at,
        MIDDLE,
        &detent(-60.0),
        Turn::Detent,
    );
    assert_eq!(outs, [SwipeOut::Switch(Toward::After)]);
    // A wheel turned down is the list's.
    let down = vec![(0.0, -60.0, Changed, 0)];
    let (outs, _, _) = run_from(SpaceSwipe::default(), Stamp(0), MIDDLE, &down, Turn::Detent);
    assert!(outs.is_empty());
}

#[test]
fn at_an_end_of_the_list_it_only_stretches() {
    let first = Room {
        before: false,
        ..MIDDLE
    };
    let last = Room {
        after: false,
        ..MIDDLE
    };
    // name, room, distance, switches
    let cases: &[(&str, Room, f32, Vec<SwipeOut>)] = &[
        ("right of the first", first, 200.0, vec![]),
        (
            "left of the first",
            first,
            -200.0,
            vec![SwipeOut::Switch(Toward::After)],
        ),
        ("left of the last", last, -200.0, vec![]),
        ("one Space, either way", ALONE, 200.0, vec![]),
        ("one Space, the other", ALONE, -200.0, vec![]),
    ];
    for (name, room, total, want) in cases {
        let mut steps = glide(10, total / 10.0);
        steps.push((0.0, 0.0, Ended, 16));
        assert_eq!(run(*room, &steps).0, *want, "{name}");
    }
}

#[test]
fn the_content_follows_the_fingers_and_stretches_at_an_end() {
    let standard = MotionLevel::Standard;
    // name, room, carried, drawn
    let cases: &[(&str, Room, f32, f32)] = &[
        ("towards a Space", MIDDLE, -50.0, -50.0),
        ("no further than the sidebar", MIDDLE, -400.0, -232.0),
        ("a little band at an end", ALONE, 20.0, 11.0),
        ("a band no wider than its most", ALONE, -300.0, -RUBBER_MAX),
    ];
    for (name, room, raw, want) in cases {
        let got = drawn(Px(*raw), room, standard).0;
        assert!((got - want).abs() < 1e-3, "{name}: {got}");
    }
    assert_eq!(drawn(Px(-80.0), &MIDDLE, MotionLevel::Reduced), Px(0.0));
    assert_eq!(drawn(Px(20.0), &ALONE, MotionLevel::Reduced), Px(0.0));
}

#[test]
fn a_quiet_ends_a_gesture_with_no_lift_and_a_cancel_forgets_it() {
    // A source with no phases (a Windows touchpad): the quiet is the lift, and it springs back.
    let (outs, swipe, _) = run(MIDDLE, &glide(4, -8.0));
    assert!(outs.is_empty());
    assert!(swipe.held().is_some());
    assert_eq!(quieted(swipe, MIDDLE), SpaceSwipe::default());
    // The quiet woken early ends nothing.
    let early = swipe.step(SwipeIn::Elapsed, Stamp(0), &(), &MIDDLE).0;
    assert_eq!(early, swipe);
    let mut cancelled = glide(4, -8.0);
    cancelled.push((0.0, 0.0, Cancelled, 16));
    let (outs, swipe, _) = run(MIDDLE, &cancelled);
    assert!(outs.is_empty());
    assert_eq!(swipe, SpaceSwipe::default());
    assert_eq!(swipe.wake(), None, "a rested swipe runs no timer");
}

use super::{Draft, MOST_DOTS, Nudge, Refused, Stride};
use crate::palette::Dot;
use crate::space::{CardAccent, PRESETS, Space};
use crate::view::{Motion, Theme};

fn space(dots: &[Dot]) -> Space {
    Space {
        name: "Work".to_owned(),
        dots: dots.to_vec(),
        ..Space::default()
    }
}

const ONE: Dot = Dot {
    hue: 100.0,
    chroma: 0.5,
};

fn close(got: f32, want: f32) -> bool {
    (got - want).abs() < 1e-4
}

#[test]
fn an_arrow_moves_a_dot_one_degree_or_one_hundredth_and_shift_ten_times_that() {
    // Each row starts from the same dot, so a row that moved the wrong axis, or by the
    // wrong amount, cannot be rescued by the row before it.
    const CASES: &[(&str, Nudge, Stride, f32, f32)] = &[
        ("right", Nudge::Right, Stride::One, 101.0, 0.5),
        ("left", Nudge::Left, Stride::One, 99.0, 0.5),
        ("up", Nudge::Up, Stride::One, 100.0, 0.51),
        ("down", Nudge::Down, Stride::One, 100.0, 0.49),
        ("shift right", Nudge::Right, Stride::Ten, 110.0, 0.5),
        ("shift left", Nudge::Left, Stride::Ten, 90.0, 0.5),
        ("shift up", Nudge::Up, Stride::Ten, 100.0, 0.6),
        ("shift down", Nudge::Down, Stride::Ten, 100.0, 0.4),
    ];
    for &(name, way, stride, hue, chroma) in CASES {
        let mut draft = Draft::open(0, space(&[ONE]));
        draft.nudge(0, way, stride);
        let got = draft.space.dots[0];
        assert!(
            close(got.hue, hue) && close(got.chroma, chroma),
            "{name}: got hue {} chroma {}, want {hue} {chroma}",
            got.hue,
            got.chroma
        );
    }
}

#[test]
fn hue_wraps_and_chroma_stops_at_its_ends() {
    const CASES: &[(&str, Dot, Nudge, Stride, f32, f32)] = &[
        (
            "left past zero",
            Dot {
                hue: 0.5,
                chroma: 0.5,
            },
            Nudge::Left,
            Stride::One,
            359.5,
            0.5,
        ),
        (
            "right past 360",
            Dot {
                hue: 355.0,
                chroma: 0.5,
            },
            Nudge::Right,
            Stride::Ten,
            5.0,
            0.5,
        ),
        (
            "up past full",
            Dot {
                hue: 10.0,
                chroma: 0.95,
            },
            Nudge::Up,
            Stride::Ten,
            10.0,
            1.0,
        ),
        (
            "down past none",
            Dot {
                hue: 10.0,
                chroma: 0.05,
            },
            Nudge::Down,
            Stride::Ten,
            10.0,
            0.0,
        ),
    ];
    for &(name, dot, way, stride, hue, chroma) in CASES {
        let mut draft = Draft::open(0, space(&[dot]));
        draft.nudge(0, way, stride);
        let got = draft.space.dots[0];
        assert!(
            close(got.hue, hue) && close(got.chroma, chroma),
            "{name}: got {got:?}, want hue {hue} chroma {chroma}"
        );
    }
}

#[test]
fn the_arrow_keys_are_the_only_keys_that_nudge() {
    const CASES: &[(&str, Option<Nudge>)] = &[
        ("ArrowLeft", Some(Nudge::Left)),
        ("ArrowRight", Some(Nudge::Right)),
        ("ArrowUp", Some(Nudge::Up)),
        ("ArrowDown", Some(Nudge::Down)),
        ("Escape", None),
        ("h", None),
        ("arrowleft", None),
    ];
    for &(key, want) in CASES {
        assert_eq!(Nudge::of_key(key), want, "{key}");
    }
}

#[test]
fn a_fourth_stop_is_refused_and_the_dots_are_unchanged() {
    let mut draft = Draft::open(0, space(&[ONE]));
    assert_eq!(draft.add(), Ok(()));
    assert_eq!(draft.add(), Ok(()));
    assert_eq!(draft.space.dots.len(), MOST_DOTS);
    let before = draft.space.dots.clone();
    assert_eq!(draft.add(), Err(Refused::Full));
    assert_eq!(draft.space.dots, before, "a refused add changed the dots");
    // The two that were added turned along the wheel from the one before, at its chroma.
    assert!(
        close(before[1].hue, 148.0) && close(before[2].hue, 196.0),
        "{before:?}"
    );
    assert_eq!(draft.active, 2, "the new dot is the one the keys act on");
}

#[test]
fn removing_the_last_stop_is_refused() {
    let mut draft = Draft::open(0, space(PRESETS[1]));
    assert_eq!(draft.remove(0), Ok(()));
    assert_eq!(draft.remove(0), Ok(()));
    assert_eq!(draft.space.dots.len(), 1);
    let before = draft.space.dots.clone();
    assert_eq!(draft.remove(0), Err(Refused::Last));
    assert_eq!(
        draft.space.dots, before,
        "a refused remove changed the dots"
    );
    assert_eq!(draft.remove(3), Err(Refused::Missing));
}

#[test]
fn escape_restores_the_saved_space_exactly() {
    let saved = Space {
        name: "Home".to_owned(),
        dots: PRESETS[3].to_vec(),
        grain: 60,
        theme: Theme::Dark,
        motion: Motion::Calm,
        card_accent: CardAccent::Postmark,
        ..Space::default()
    };
    let mut draft = Draft::open(2, saved.clone());
    draft.space.name = "Elsewhere".to_owned();
    draft.nudge(0, Nudge::Right, Stride::Ten);
    draft.add().unwrap_or_else(|why| panic!("{why:?}"));
    draft.space.grain = 5;
    draft.space.theme = Theme::Light;
    draft.space.motion = Motion::Extra;
    draft.space.card_accent = CardAccent::Hint;
    draft.preset(9);
    assert_ne!(draft.space, saved, "the edits above changed nothing");
    assert_eq!(draft.reverted(), saved);
    assert_eq!(draft.index, 2);
}

#[test]
fn the_pointer_places_hue_across_and_chroma_down() {
    let mut draft = Draft::open(0, space(&[ONE, ONE]));
    draft.place(1, 0.25, 0.25);
    let got = draft.space.dots[1];
    assert!(close(got.hue, 90.0) && close(got.chroma, 0.75), "{got:?}");
    assert_eq!(draft.space.dots[0], ONE, "the other dot moved");
    draft.place(1, 1.4, -0.3);
    let got = draft.space.dots[1];
    assert!(close(got.hue, 359.0) && close(got.chroma, 1.0), "{got:?}");
    assert_eq!(draft.active, 1);
}

use super::{Dot, derive, gradient, js_round};

const WORK: &[Dot] = &[
    Dot {
        hue: 268.0,
        chroma: 0.72,
    },
    Dot {
        hue: 318.0,
        chroma: 0.55,
    },
];
const HOME: &[Dot] = &[
    Dot {
        hue: 152.0,
        chroma: 0.62,
    },
    Dot {
        hue: 62.0,
        chroma: 0.55,
    },
    Dot {
        hue: 28.0,
        chroma: 0.5,
    },
];
const GREY: &[Dot] = &[Dot {
    hue: 250.0,
    chroma: 0.06,
}];
const HOT: &[Dot] = &[Dot {
    hue: 85.0,
    chroma: 1.0,
}];

struct Case {
    dots: &'static [Dot],
    line: &'static str,
}

const CASES: &[Case] = &[
    Case {
        dots: WORK,
        line: r##"{"space":"work","dark":false,"stops":["#e1eafe","#eee0f3"],"picked":["#8da8f0","#c19bcd"],"ink":"#12192f","soft":"#3f475c","faint":"#60697e","hover":"#d3d9e9","accent":"#44547d","accentSoft":"#e4e9f6","accentInk":"#FFFFFF","capped":false}"##,
    },
    Case {
        dots: WORK,
        line: r##"{"space":"work","dark":true,"stops":["#131928","#211924"],"picked":["#8da8f0","#c19bcd"],"ink":"#e4e8f1","soft":"#b8becc","faint":"#8b92a2","hover":"rgba(255,255,255,.06)","accent":"#a1b3e2","accentSoft":"#252b3b","accentInk":"#12161f","capped":false}"##,
    },
    Case {
        dots: HOME,
        line: r##"{"space":"home","dark":false,"stops":["#dbf1df","#f5e2d3","#f3dcd8"],"picked":["#7dbc8e","#d19f74","#d69991"],"ink":"#0c1f12","soft":"#3b4d40","faint":"#5c6e60","hover":"#d0ddd3","accent":"#376043","accentSoft":"#e1ede4","accentInk":"#FFFFFF","capped":false}"##,
    },
    Case {
        dots: HOME,
        line: r##"{"space":"home","dark":true,"stops":["#101d13","#251a11","#291c1a"],"picked":["#7dbc8e","#d19f74","#d69991"],"ink":"#e3eae4","soft":"#b5c1b8","faint":"#89968c","hover":"rgba(255,255,255,.06)","accent":"#95c1a0","accentSoft":"#203024","accentInk":"#0f1912","capped":false}"##,
    },
    Case {
        dots: GREY,
        line: r##"{"space":"grey","dark":false,"stops":["#e8eaec"],"picked":["#a7abb0"],"ink":"#191b1c","soft":"#464849","faint":"#68696b","hover":"#d6d9dd","accent":"#41576f","accentSoft":"#e1ebf5","accentInk":"#FFFFFF","capped":false}"##,
    },
    Case {
        dots: GREY,
        line: r##"{"space":"grey","dark":true,"stops":["#191a1b"],"picked":["#a7abb0"],"ink":"#e7e8e8","soft":"#bdbebf","faint":"#919293","hover":"rgba(255,255,255,.06)","accent":"#9eb7d2","accentSoft":"#202d3a","accentInk":"#0f171f","capped":false}"##,
    },
    Case {
        dots: HOT,
        line: r##"{"space":"hot","dark":false,"stops":["#fae8c3"],"picked":["#d6a20a"],"ink":"#251800","soft":"#544627","faint":"#766747","hover":"#e2d8c3","accent":"#6a5118","accentSoft":"#efe9dc","accentInk":"#FFFFFF","capped":false}"##,
    },
    Case {
        dots: HOT,
        line: r##"{"space":"hot","dark":true,"stops":["#221801"],"picked":["#d6a20a"],"ink":"#ede7db","soft":"#c7bda8","faint":"#9c917a","hover":"rgba(255,255,255,.06)","accent":"#ccb178","accentSoft":"#322a1a","accentInk":"#1a150b","capped":false}"##,
    },
];

fn assert_colour(space: &str, theme: &str, field: &str, got: &str, want: &str) {
    assert!(
        got.eq_ignore_ascii_case(want),
        "{space} {theme} {field}: got {got} want {want}",
    );
}

fn strings<'a>(value: &'a serde_json::Value, field: &str) -> Vec<&'a str> {
    value
        .get(field)
        .and_then(|item| item.as_array())
        .map(|items| items.iter().filter_map(|item| item.as_str()).collect())
        .unwrap_or_default()
}

#[test]
fn derive_matches_the_mockup() {
    assert_eq!(CASES.len(), 8, "the eight sample rows");
    for case in CASES {
        let want: serde_json::Value =
            serde_json::from_str(case.line).unwrap_or_else(|e| panic!("{e}: {}", case.line));
        let space = want["space"].as_str().unwrap_or("?");
        let dark = want["dark"].as_bool().unwrap_or(false);
        let theme = if dark { "dark" } else { "light" };
        let got = derive(case.dots, dark);

        let want_stops = strings(&want, "stops");
        assert_eq!(
            got.stops.len(),
            want_stops.len(),
            "{space} {theme} stops: got {} want {}",
            got.stops.len(),
            want_stops.len(),
        );
        for (index, (got_stop, want_stop)) in got.stops.iter().zip(want_stops).enumerate() {
            assert_colour(
                space,
                theme,
                &format!("stops[{index}]"),
                got_stop,
                want_stop,
            );
        }
        let want_picked = strings(&want, "picked");
        assert_eq!(
            got.picked.len(),
            want_picked.len(),
            "{space} {theme} picked: got {} want {}",
            got.picked.len(),
            want_picked.len(),
        );
        for (index, (got_pick, want_pick)) in got.picked.iter().zip(want_picked).enumerate() {
            assert_colour(
                space,
                theme,
                &format!("picked[{index}]"),
                got_pick,
                want_pick,
            );
        }
        for (field, got_value, key) in [
            ("ink", got.ink.as_str(), "ink"),
            ("soft", got.soft.as_str(), "soft"),
            ("faint", got.faint.as_str(), "faint"),
            ("hover", got.hover.as_str(), "hover"),
            ("accent", got.accent.as_str(), "accent"),
            ("accent_soft", got.accent_soft.as_str(), "accentSoft"),
            ("accent_ink", got.accent_ink.as_str(), "accentInk"),
        ] {
            let want_value = want[key].as_str().unwrap_or("");
            assert_colour(space, theme, field, got_value, want_value);
        }
        let want_capped = want["capped"].as_bool().unwrap_or(true);
        assert_eq!(got.capped, want_capped, "{space} {theme} capped");
        let want_pill = if dark {
            "rgba(255,255,255,.10)"
        } else {
            "rgba(255,255,255,.72)"
        };
        assert_colour(space, theme, "pill", &got.pill, want_pill);
    }

    let neutral = Dot {
        hue: 250.0,
        chroma: 0.06,
    };
    assert_eq!(
        derive(&[], false),
        derive(&[neutral], false),
        "empty dots, light",
    );
    assert_eq!(
        derive(&[], true),
        derive(&[neutral], true),
        "empty dots, dark",
    );
}

#[test]
fn every_pick_is_legible() {
    use crate::contrast::ratio;

    let mut failures = Vec::new();
    let mut cases = 0_u32;
    for dark in [false, true] {
        let theme = if dark { "dark" } else { "light" };
        let surface = if dark { "#1D211B" } else { "#F8F9F6" };
        let card_ink = if dark { "#E7EBE3" } else { "#1A1E1A" };
        for hue in (0..360).step_by(5) {
            for step in 0..=10 {
                let chroma = step as f32 / 10.0;
                let chroma_label = format!("{}.{}", step / 10, step % 10);
                for count in 1..=3 {
                    cases += 1;
                    let dots: Vec<Dot> = (0..count)
                        .map(|index| Dot {
                            hue: ((hue + index * 48) % 360) as f32,
                            chroma,
                        })
                        .collect();
                    let palette = derive(&dots, dark);
                    let case = format!("{theme} h={hue} c={chroma_label} dots={count}");
                    for (index, stop) in palette.stops.iter().enumerate() {
                        match ratio(&palette.ink, stop) {
                            Some(measured) if measured < 4.5 => failures
                                .push(format!("{case} stop {index}: ink {measured:.2} < 4.5")),
                            Some(_) => {}
                            None => failures.push(format!(
                                "{case} stop {index}: ink on {stop} is not a hex pair"
                            )),
                        }
                        match ratio(&palette.faint, stop) {
                            Some(measured) if measured < 3.0 => failures
                                .push(format!("{case} stop {index}: faint {measured:.2} < 3.0")),
                            Some(_) => {}
                            None => failures.push(format!(
                                "{case} stop {index}: faint on {stop} is not a hex pair"
                            )),
                        }
                    }
                    match ratio(&palette.accent, surface) {
                        Some(measured) if measured < 4.5 => {
                            failures.push(format!("{case}: accent {measured:.2} < 4.5"));
                        }
                        Some(_) => {}
                        None => failures.push(format!("{case}: accent is not a hex pair")),
                    }
                    match ratio(card_ink, &palette.accent_soft) {
                        Some(measured) if measured < 4.5 => {
                            failures.push(format!("{case}: card ink {measured:.2} < 4.5"));
                        }
                        Some(_) => {}
                        None => failures.push(format!("{case}: accent tint is not a hex pair")),
                    }
                }
            }
        }
    }
    assert_eq!(cases, 72 * 11 * 3 * 2, "the sweep missed a row");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn gradient_matches_the_mockup() {
    let cases = [
        (
            "one stop",
            &["#e8eaec"][..],
            "linear-gradient(135deg,#e8eaec,#e8eaec)",
        ),
        (
            "two stops",
            &["#e1eafe", "#eee0f3"][..],
            "linear-gradient(135deg,#e1eafe 0%,#eee0f3 100%)",
        ),
        (
            "three stops",
            &["#dbf1df", "#f5e2d3", "#f3dcd8"][..],
            "linear-gradient(135deg,#dbf1df 0%,#f5e2d3 50%,#f3dcd8 100%)",
        ),
    ];
    for (name, stops, want) in cases {
        let palette = super::Palette {
            stops: stops.iter().map(|stop| (*stop).to_owned()).collect(),
            picked: Vec::new(),
            ink: String::new(),
            soft: String::new(),
            faint: String::new(),
            hover: String::new(),
            pill: String::new(),
            accent: String::new(),
            accent_soft: String::new(),
            accent_ink: String::new(),
            capped: false,
        };
        assert_eq!(gradient(&palette), want, "{name}");
    }
}

#[test]
fn half_rounds_the_way_javascript_does() {
    let cases = [
        (1.5, 2.0),
        (2.5, 3.0),
        (-1.5, -1.0),
        (0.5, 1.0),
        (-0.5, 0.0),
    ];
    for (input, want) in cases {
        assert_eq!(js_round(input), want, "{input}");
    }
}

#[test]
fn the_readout_is_the_ratio_of_the_derived_tokens() {
    // A fixed pick, measured here the long way from `derive` and `ratio`. The readout must say
    // exactly these numbers: the editor shows nothing it computed on its own.
    use super::{POST_DARK, POST_LIGHT, readout};
    use crate::contrast::ratio;
    use crate::space::{CardAccent, Space};
    for dark in [false, true] {
        for accent in [CardAccent::Hint, CardAccent::Postmark] {
            let space = Space {
                dots: HOME.to_vec(),
                card_accent: accent,
                ..Space::default()
            };
            let palette = derive(HOME, dark);
            let post = if dark { POST_DARK } else { POST_LIGHT };
            let worst = |fore: &str| {
                palette
                    .stops
                    .iter()
                    .map(|stop| ratio(fore, stop).expect("hex"))
                    .fold(f64::INFINITY, f64::min)
            };
            let (tint_accent, tint) = match accent {
                CardAccent::Hint => (palette.accent.clone(), palette.accent_soft.clone()),
                CardAccent::Postmark => (post.accent.to_owned(), post.accent_soft.to_owned()),
            };
            let want = [
                worst(&palette.ink),
                worst(&palette.faint),
                ratio(&tint_accent, post.surface).expect("hex"),
                ratio(post.ink, &tint).expect("hex"),
            ];
            let checks = readout(&space, dark);
            let got: Vec<f64> = checks.iter().map(|check| check.measured).collect();
            assert_eq!(got, want, "dark={dark} {accent:?}");
            assert!(
                checks.iter().all(|check| check.passes()),
                "dark={dark} {accent:?}: {checks:?}"
            );
        }
    }
}

#[test]
fn a_swatch_is_the_pick_at_the_themes_lightness() {
    // Light is the same colour `derive` paints on the dot itself, so the field and the
    // stops list agree about what a pick looks like.
    for dot in HOME {
        assert_eq!(
            super::swatch(*dot, false),
            derive(&[*dot], false).picked[0],
            "{dot:?}"
        );
        assert_ne!(
            super::swatch(*dot, true),
            super::swatch(*dot, false),
            "{dot:?}"
        );
    }
}

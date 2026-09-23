use super::{Fade, appearance_script, frame_pairs, paint_script};
use crate::palette::{self, Dot};
use crate::space::{CardAccent, PRESETS, Space};
use crate::view::{Motion, Theme};

/// One JSON string at the start of `rest`, and what follows it.
fn string_at(rest: &str) -> (String, &str) {
    let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<String>();
    let value = match stream.next() {
        Some(Ok(value)) => value,
        other => panic!("not a JSON string at {rest:.40}: {other:?}"),
    };
    let used = stream.byte_offset();
    (value, &rest[used..])
}

/// Every `style.setProperty(name, value)` in `script`, in order, with the quoting undone.
pub(in crate::ui) fn set_properties(script: &str) -> Vec<(String, String)> {
    const CALL: &str = "document.documentElement.style.setProperty(";
    let mut out = Vec::new();
    let mut rest = script;
    while let Some(at) = rest.find(CALL) {
        let (name, after) = string_at(&rest[at + CALL.len()..]);
        let after = after
            .strip_prefix(", ")
            .unwrap_or_else(|| panic!("no second argument after {name}"));
        let (value, after) = string_at(after);
        assert!(after.starts_with(");"), "{name}: {after:.20}");
        out.push((name, value));
        rest = after;
    }
    out
}

fn spaces() -> Vec<(&'static str, Space)> {
    vec![
        ("system hint", Space::default()),
        (
            "dark postmark",
            Space {
                dots: PRESETS[1].to_vec(),
                theme: Theme::Dark,
                card_accent: CardAccent::Postmark,
                grain: 70,
                ..Space::default()
            },
        ),
        (
            "light hint",
            Space {
                dots: PRESETS[3].to_vec(),
                theme: Theme::Light,
                motion: Motion::Calm,
                ..Space::default()
            },
        ),
        (
            "system postmark",
            Space {
                dots: PRESETS[10].to_vec(),
                card_accent: CardAccent::Postmark,
                ..Space::default()
            },
        ),
    ]
}

#[test]
fn the_head_and_a_switch_set_the_same_pairs() {
    // Read back out of the two scripts, not compared as strings: the scripts differ on
    // purpose (the switch fades), and what must not differ is what each writes.
    for (name, space) in spaces() {
        let head = set_properties(&appearance_script(&space));
        for fade in [Fade::Cross, Fade::None] {
            let runtime = set_properties(&paint_script(&space, fade));
            assert_eq!(head, runtime, "{name} {fade:?}");
        }
        assert_eq!(head, frame_pairs(&space), "{name}");
        assert!(!head.is_empty(), "{name}: nothing was painted");
    }
}

#[test]
fn a_switch_sets_the_new_spaces_quoted_gradient() {
    let space = Space {
        dots: vec![
            Dot {
                hue: 152.0,
                chroma: 0.62,
            },
            Dot {
                hue: 62.0,
                chroma: 0.55,
            },
        ],
        theme: Theme::Dark,
        ..Space::default()
    };
    let gradient = palette::gradient(&palette::derive(&space.dots, true));
    let quoted = serde_json::to_string(&gradient).expect("a string serializes");
    let script = paint_script(&space, Fade::Cross);
    let needle = format!("setProperty(\"--f-grad\", {quoted});");
    assert!(script.contains(&needle), "{script}");
    assert!(
        script.contains("document.documentElement.dataset.theme = \"dark\";"),
        "{script}"
    );
}

#[test]
fn a_crossfade_freezes_the_old_layer_and_fades_the_other_in() {
    let space = Space::default();
    let cross = paint_script(&space, Fade::Cross);
    let freeze = cross
        .find("layers[front].style.background = getComputedStyle")
        .unwrap_or_else(|| panic!("the old gradient is not frozen:\n{cross}"));
    let tokens = cross
        .find("setProperty(\"--f-grad")
        .unwrap_or_else(|| panic!("no gradient:\n{cross}"));
    let fade = cross
        .find("layers[next].style.opacity = \"1\"")
        .unwrap_or_else(|| panic!("nothing fades in:\n{cross}"));
    assert!(
        freeze < tokens && tokens < fade,
        "freeze {freeze}, tokens {tokens}, fade {fade}: the old gradient has to be copied \
         before the tokens change, and the new layer shown after"
    );
    assert!(
        cross.contains("layers[front].style.opacity = \"0\""),
        "{cross}"
    );

    let live = paint_script(&space, Fade::None);
    assert!(
        !live.contains("style.opacity"),
        "the live preview fades: {live}"
    );
    assert!(
        live.contains("layers[front].style.background = \"\""),
        "a live repaint leaves a frozen gradient on the front layer: {live}"
    );
}

#[test]
fn postmark_writes_no_accent_and_clears_the_system_rule() {
    let hint = appearance_script(&Space::default());
    let postmark = appearance_script(&Space {
        card_accent: CardAccent::Postmark,
        ..Space::default()
    });
    let named = |script: &str, prefix: &str| {
        set_properties(script)
            .into_iter()
            .any(|(name, _)| name.starts_with(prefix))
    };
    assert!(named(&hint, "--f-accent-l"), "{hint}");
    assert!(hint.contains("--accent:var(--f-accent-l)"), "{hint}");
    assert!(!named(&postmark, "--f-accent"), "{postmark}");
    assert!(!named(&postmark, "--accent"), "{postmark}");
    assert!(
        postmark.contains("node.textContent = \"\";"),
        "Postmark left a Space's accent rule in place: {postmark}"
    );
}

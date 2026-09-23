use super::{PRESET_NAMES, PRESETS};

#[test]
fn presets_are_the_mockup_then_the_six_hues() {
    let mockup: &[&[(f32, f32)]] = &[
        &[(268.0, 0.72), (318.0, 0.55)],
        &[(152.0, 0.62), (62.0, 0.55), (28.0, 0.5)],
        &[(220.0, 0.7)],
        &[(20.0, 0.66), (55.0, 0.6)],
        &[(190.0, 0.6), (240.0, 0.55)],
        &[(340.0, 0.6), (290.0, 0.5)],
        &[(95.0, 0.5)],
        &[(250.0, 0.06)],
    ];
    assert_eq!(
        PRESETS.len(),
        mockup.len() + 6,
        "eight presets plus six hues"
    );
    for (index, dots) in mockup.iter().enumerate() {
        assert_eq!(PRESETS[index].len(), dots.len(), "preset {index}");
        for (got, &(hue, chroma)) in PRESETS[index].iter().zip(dots.iter()) {
            assert_eq!((got.hue, got.chroma), (hue, chroma), "preset {index}");
        }
    }
    let accents = [
        ("postmark", "#23508F", 0.7_f32),
        ("graphite", "#2E342C", 0.08),
        ("pine", "#1F6349", 0.7),
        ("indigo", "#3A3D96", 0.7),
        ("oxblood", "#8E2F31", 0.7),
        ("vermilion", "#C0402A", 0.7),
    ];
    for (index, (name, hex, chroma)) in accents.iter().enumerate() {
        let computed = oklch_hue(hex);
        println!("{name} {hex} hue {computed}");
        let preset = PRESETS[mockup.len() + index];
        assert_eq!(preset.len(), 1, "{name}");
        let stored = f64::from(preset[0].hue);
        assert!(
            (stored - computed).abs() < 1e-3,
            "{name}: stored {stored} computed {computed}",
        );
        assert_eq!(preset[0].chroma, *chroma, "{name} chroma");
    }
}

/// OKLCH hue of an `#rrggbb` swatch. Printed once so the preset literals can be checked.
fn oklch_hue(hex: &str) -> f64 {
    let channel = |start: usize| {
        let value =
            u8::from_str_radix(&hex[start..start + 2], 16).unwrap_or_else(|e| panic!("{hex}: {e}"));
        f64::from(value) / 255.0
    };
    let linear = |encoded: f64| {
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    };
    let red = linear(channel(1));
    let green = linear(channel(3));
    let blue = linear(channel(5));
    let l = 0.4122214708 * red + 0.5363325363 * green + 0.0514459929 * blue;
    let m = 0.2119034982 * red + 0.6806995451 * green + 0.1073969566 * blue;
    let s = 0.0883024619 * red + 0.2817188376 * green + 0.6299787005 * blue;
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    let a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
    let b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;
    let mut hue = b.atan2(a).to_degrees();
    if hue < 0.0 {
        hue += 360.0;
    }
    hue
}

#[test]
fn every_preset_has_a_name() {
    assert_eq!(PRESET_NAMES.len(), PRESETS.len(), "a preset with no name");
}

use super::{PRESET_NAMES, PRESETS};

#[test]
fn presets_are_the_mockups_eight() {
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
    assert_eq!(PRESETS.len(), mockup.len(), "eight presets");
    for (index, dots) in mockup.iter().enumerate() {
        assert_eq!(PRESETS[index].len(), dots.len(), "preset {index}");
        for (got, &(hue, chroma)) in PRESETS[index].iter().zip(dots.iter()) {
            assert_eq!((got.hue, got.chroma), (hue, chroma), "preset {index}");
        }
    }
}

#[test]
fn every_preset_has_quires_name() {
    // The editor names each preset as quire's `SpaceEditor` does (its `Preset::name`); the six
    // retired accent hues are gone with mailo's own editor.
    assert_eq!(
        PRESET_NAMES,
        [
            "Dusk", "Orchard", "Harbour", "Ember", "Lagoon", "Heather", "Moss", "Stone"
        ]
    );
    assert_eq!(PRESET_NAMES.len(), PRESETS.len(), "a preset with no name");
}

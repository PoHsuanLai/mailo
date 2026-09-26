use super::caps;
use crate::keymap::DEFAULTS;
use ds::Key;

#[test]
fn a_key_is_drawn_as_the_caps_that_press_it() {
    let cases: &[(&str, Vec<Key>)] = &[
        ("e", vec![Key::Char('e')]),
        ("J", vec![Key::Shift, Key::Char('j')]),
        ("#", vec![Key::Char('#')]),
        ("ArrowDown", vec![Key::Down]),
        ("Delete", vec![Key::Delete]),
        ("F5", Vec::new()),
    ];
    for (key, drawn) in cases {
        assert_eq!(caps(key), *drawn, "{key:?}");
    }
}

#[test]
fn every_shipped_key_has_caps() {
    for (action, keys) in DEFAULTS {
        for key in *keys {
            assert!(
                !caps(key).is_empty(),
                "{action:?}'s {key:?} is drawn as words"
            );
        }
    }
}

use super::{DEFAULTS, FILE_NAME, Keymap, Refused, bind, load, reset, save, spoken};
use crate::ui::view::Shortcut;

/// The keyboard as it was before it became a table: `view::shortcut`'s match, kept here
/// verbatim so the table is checked against it rather than against itself.
fn before(key: &str, typing: bool) -> Option<Shortcut> {
    if key == "Escape" {
        return Some(Shortcut::Back);
    }
    if typing {
        return None;
    }
    Some(match key {
        "j" | "ArrowDown" => Shortcut::Next,
        "k" | "ArrowUp" => Shortcut::Previous,
        "J" => Shortcut::ExtendNext,
        "K" => Shortcut::ExtendPrevious,
        "!" => Shortcut::Spam,
        "e" => Shortcut::Archive,
        "#" | "Delete" => Shortcut::Trash,
        "s" => Shortcut::ToggleStar,
        "u" => Shortcut::ToggleRead,
        "r" => Shortcut::Reply,
        "a" => Shortcut::ReplyAll,
        "f" => Shortcut::Forward,
        "p" => Shortcut::TogglePin,
        "m" => Shortcut::ToggleMute,
        "c" => Shortcut::Compose,
        _ => return None,
    })
}

/// Every key a keyboard names: each printable ASCII character, and the named keys.
fn every_key() -> Vec<String> {
    const NAMED: &[&str] = &[
        "Escape",
        "Enter",
        "Tab",
        "Backspace",
        "Delete",
        "Insert",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "ArrowDown",
        "ArrowUp",
        "ArrowLeft",
        "ArrowRight",
        "Shift",
        "Control",
        "Alt",
        "Meta",
        "CapsLock",
        "F1",
        "F5",
        "F12",
        "ContextMenu",
        "Unidentified",
        "",
        "é",
    ];
    (' '..='~')
        .map(String::from)
        .chain(NAMED.iter().map(|key| (*key).to_owned()))
        .collect()
}

#[test]
fn the_default_map_is_the_keyboard_as_it_was_for_every_key() {
    let map = Keymap::default();
    for key in every_key() {
        for typing in [false, true] {
            assert_eq!(
                map.action(&key, typing),
                before(&key, typing),
                "{key:?}, typing {typing}"
            );
            assert_eq!(
                crate::ui::view::shortcut(&key, typing),
                before(&key, typing),
                "view::shortcut {key:?}"
            );
        }
    }
}

#[test]
fn every_action_is_listed_once_and_no_key_twice() {
    let mut actions: Vec<String> = DEFAULTS.iter().map(|(a, _)| format!("{a:?}")).collect();
    let listed = actions.len();
    actions.sort();
    actions.dedup();
    assert_eq!(actions.len(), listed);
    let mut keys: Vec<&str> = DEFAULTS
        .iter()
        .flat_map(|(_, keys)| keys.iter().copied())
        .collect();
    let all = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), all);
}

#[test]
fn an_override_moves_the_action_and_frees_its_old_keys() {
    let map = bind(&Keymap::default(), Shortcut::Archive, "x").unwrap();
    assert_eq!(map.action("x", false), Some(Shortcut::Archive));
    assert_eq!(map.action("e", false), None, "the old key does nothing");
    assert_eq!(map.action("x", true), None, "still not while typing");
    assert_eq!(map.keys(Shortcut::Archive), ["x"]);
    assert!(map.is_changed(Shortcut::Archive));
    // Every other action is where it was.
    for (action, keys) in DEFAULTS.iter().filter(|(a, _)| *a != Shortcut::Archive) {
        assert_eq!(map.keys(*action), *keys, "{action:?}");
    }

    // The freed key can be given to another action.
    let map = bind(&map, Shortcut::ToggleStar, "e").unwrap();
    assert_eq!(map.action("e", false), Some(Shortcut::ToggleStar));
    assert_eq!(map.action("s", false), None);

    // Giving an action the key it ships with is a reset.
    let map = bind(&map, Shortcut::Archive, "x").unwrap();
    let map = bind(&map, Shortcut::ToggleStar, "s").unwrap();
    assert!(!map.is_changed(Shortcut::ToggleStar));
}

#[test]
fn a_key_another_action_holds_is_refused_by_that_action_s_name() {
    const CASES: &[(Shortcut, &str, Shortcut)] = &[
        (Shortcut::Archive, "s", Shortcut::ToggleStar),
        (Shortcut::Archive, "ArrowDown", Shortcut::Next),
        (Shortcut::Compose, "J", Shortcut::ExtendNext),
        (Shortcut::Reply, "Delete", Shortcut::Trash),
    ];
    for (action, key, by) in CASES {
        let refused = bind(&Keymap::default(), *action, key).unwrap_err();
        assert_eq!(
            refused,
            Refused::Taken {
                key: (*key).to_owned(),
                by: *by
            },
            "{action:?} on {key}"
        );
        assert!(
            refused.to_string().contains(super::name(*by)),
            "{refused} names {by:?}"
        );
    }
    assert_eq!(
        bind(&Keymap::default(), Shortcut::Archive, "s")
            .unwrap_err()
            .to_string(),
        "S is already Star or unstar’s key."
    );
}

#[test]
fn keys_the_window_keeps_are_refused_and_esc_s_action_cannot_move() {
    for key in ["Escape", "Enter", "Tab", " ", "Shift"] {
        assert_eq!(
            bind(&Keymap::default(), Shortcut::Archive, key),
            Err(Refused::Kept(key.to_owned())),
            "{key:?}"
        );
    }
    assert_eq!(
        bind(&Keymap::default(), Shortcut::Back, "q"),
        Err(Refused::Fixed(Shortcut::Back))
    );
}

#[test]
fn a_reset_is_refused_while_another_action_holds_a_default_key() {
    let map = bind(&Keymap::default(), Shortcut::Archive, "x").unwrap();
    let map = bind(&map, Shortcut::ToggleStar, "e").unwrap();
    assert_eq!(
        reset(&map, Shortcut::Archive),
        Err(Refused::Taken {
            key: "e".to_owned(),
            by: Shortcut::ToggleStar
        })
    );
    let map = reset(&map, Shortcut::ToggleStar).unwrap();
    let map = reset(&map, Shortcut::Archive).unwrap();
    assert_eq!(map, Keymap::default());
}

#[test]
fn a_keymap_round_trips_through_its_file() {
    let dir = tempfile::tempdir().unwrap();
    // Set in an order a sequential replay would refuse: Star takes `e` before Archive's last move.
    let map = bind(&Keymap::default(), Shortcut::Archive, "x").unwrap();
    let map = bind(&map, Shortcut::ToggleStar, "e").unwrap();
    let map = bind(&map, Shortcut::Archive, "y").unwrap();
    save(dir.path(), &map).unwrap();
    assert_eq!(load(dir.path()), map);
    let raw = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
    assert!(raw.contains("\"toggle_star\""), "{raw}");
}

#[test]
fn a_missing_or_damaged_file_is_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(load(dir.path()), Keymap::default(), "missing");
    const CASES: &[(&str, &str)] = &[
        ("empty", ""),
        ("prose", "not json {{{"),
        ("wrong shape", r#"{"bindings":{"archive":"x"}}"#),
        (
            "unknown action",
            r#"{"bindings":[{"action":"launch","key":"x"}]}"#,
        ),
        (
            "two meanings",
            r#"{"bindings":[{"action":"archive","key":"s"}]}"#,
        ),
        (
            "the same action twice",
            r#"{"bindings":[{"action":"archive","key":"x"},{"action":"archive","key":"y"}]}"#,
        ),
        (
            "a kept key",
            r#"{"bindings":[{"action":"archive","key":"Enter"}]}"#,
        ),
        (
            "Esc's action",
            r#"{"bindings":[{"action":"back","key":"q"}]}"#,
        ),
        ("no key", r#"{"bindings":[{"action":"archive","key":""}]}"#),
    ];
    for (name, body) in CASES {
        std::fs::write(dir.path().join(FILE_NAME), body).unwrap();
        assert_eq!(load(dir.path()), Keymap::default(), "{name}");
    }
    // And a sound one is read, so the cases above failed for their damage.
    std::fs::write(
        dir.path().join(FILE_NAME),
        r#"{"bindings":[{"action":"archive","key":"x"}]}"#,
    )
    .unwrap();
    assert_eq!(load(dir.path()).action("x", false), Some(Shortcut::Archive));
}

#[test]
fn keys_are_spoken_as_their_caps() {
    const CASES: &[(&str, &str)] = &[
        ("j", "J"),
        ("J", "Shift J"),
        ("#", "#"),
        (" ", "Space"),
        ("ArrowDown", "Down"),
        ("Delete", "Delete"),
    ];
    for (key, said) in CASES {
        assert_eq!(spoken(key), *said, "{key:?}");
    }
}

#[test]
fn the_user_s_keys_are_chordkit_overrides_and_the_defaults_are_not() {
    let map = Keymap::default();
    assert!(
        map.overrides().entries().is_empty(),
        "the defaults are declared, not overridden"
    );
    // Archive answers to x alone, and ExtendNext's capital is Shift with the letter.
    let map = bind(&map, Shortcut::Archive, "x").unwrap();
    let map = bind(&map, Shortcut::ExtendNext, "L").unwrap();
    let overrides = map.overrides();
    let ids: Vec<String> = overrides
        .entries()
        .iter()
        .map(|entry| entry.action.id())
        .collect();
    assert_eq!(ids, ["mail.extend-next", "mail.archive"]);
    assert!(
        overrides
            .entries()
            .iter()
            .all(|entry| entry.chords.len() == 1),
        "a changed action answers to its one key"
    );
}

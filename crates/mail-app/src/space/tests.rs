use super::{CardAccent, PRESETS, Pinned, Recall, Scope, Space, Spaces, load, new_space, save};
use crate::palette::Dot;
use crate::view::{Motion, Theme};
use mail_domain::{AccountId, ThreadId};
use std::collections::BTreeMap;
use std::path::Path;
use uuid::Uuid;

fn account(n: u128) -> AccountId {
    AccountId::from_uuid(Uuid::from_u128(n))
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| {
            entry
                .unwrap_or_else(|e| panic!("{e}"))
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn plain(name: &str) -> Space {
    Space {
        name: name.to_owned(),
        ..Space::default()
    }
}

#[test]
fn spaces_round_trip() {
    let work = account(1);
    let home = account(2);
    let cases = [
        ("empty", Spaces::default()),
        (
            "two spaces",
            Spaces {
                current: 1,
                recall: BTreeMap::from([(
                    0,
                    Recall {
                        place: "Archive".to_owned(),
                        open: Some(ThreadId::from_uuid(Uuid::from_u128(9))),
                        account: Some(work),
                    },
                )]),
                spaces: vec![
                    Space {
                        name: "Work".to_owned(),
                        dots: vec![
                            Dot {
                                hue: 268.0,
                                chroma: 0.5,
                            },
                            Dot {
                                hue: 318.0,
                                chroma: 0.25,
                            },
                        ],
                        grain: 35,
                        theme: Theme::Dark,
                        motion: Motion::Calm,
                        card_accent: CardAccent::Postmark,
                        scope: Scope::Accounts(vec![work]),
                        pins: vec![
                            Pinned::Person {
                                name: "Dana".to_owned(),
                                email: "dana@example.com".to_owned(),
                            },
                            Pinned::Search {
                                name: "Unread".to_owned(),
                                query: "is:unread".to_owned(),
                            },
                        ],
                        colors: BTreeMap::new(),
                    },
                    Space {
                        name: "Home".to_owned(),
                        dots: PRESETS[1].to_vec(),
                        grain: 55,
                        theme: Theme::Light,
                        motion: Motion::Standard,
                        card_accent: CardAccent::Hint,
                        scope: Scope::Accounts(vec![home]),
                        pins: Vec::new(),
                        colors: BTreeMap::new(),
                    },
                ],
            },
        ),
    ];
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let fresh = dir.path().join("mailo");
    for (name, spaces) in cases {
        save(&fresh, &spaces).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(load(&fresh), spaces, "{name}");
        assert_eq!(entries(&fresh), ["spaces.json"], "{name}");
    }
}

#[test]
fn a_missing_file_is_empty() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(load(dir.path()), Spaces::default());
}

#[test]
fn garbage_bytes_are_empty() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("spaces.json");
    const CASES: &[(&str, &[u8])] = &[
        ("empty", b""),
        ("prose", b"not json {{{"),
        ("binary", &[0xff, 0xfe, b'{']),
    ];
    for &(name, bytes) in CASES {
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(load(dir.path()), Spaces::default(), "{name}");
    }
}

#[test]
fn a_partial_file_keeps_the_fields_it_has() {
    let cases: &[(&str, &str, Spaces)] = &[
        (
            "theme only",
            r#"{"spaces":[{"theme":"light"}]}"#,
            Spaces {
                spaces: vec![Space {
                    theme: Theme::Light,
                    ..Space::default()
                }],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "unknown theme keeps the grain",
            r#"{"spaces":[{"name":"Work","theme":"sepia","grain":40}]}"#,
            Spaces {
                spaces: vec![Space {
                    name: "Work".to_owned(),
                    grain: 40,
                    theme: Theme::System,
                    ..Space::default()
                }],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "unknown card accent keeps the theme",
            r#"{"spaces":[{"name":"Work","card_accent":"rose","theme":"dark"}]}"#,
            Spaces {
                spaces: vec![Space {
                    name: "Work".to_owned(),
                    theme: Theme::Dark,
                    card_accent: CardAccent::Hint,
                    ..Space::default()
                }],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "unknown scope keeps the name",
            r#"{"spaces":[{"name":"Work","scope":{"kind":"nowhere"}}]}"#,
            Spaces {
                spaces: vec![plain("Work")],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "an extra field is ignored",
            r#"{"spaces":[{"name":"Work","future":true}],"later":1}"#,
            Spaces {
                spaces: vec![plain("Work")],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
    ];
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("spaces.json");
    for &(name, bytes, ref want) in cases {
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(load(dir.path()), *want, "{name}: {bytes}");
    }
}

#[test]
fn out_of_range_values_are_clamped() {
    let wide = Space {
        name: "Wide".to_owned(),
        dots: vec![
            Dot {
                hue: 10.0,
                chroma: 0.5,
            },
            Dot {
                hue: 20.0,
                chroma: 0.25,
            },
            Dot {
                hue: 30.0,
                chroma: 0.75,
            },
        ],
        ..Space::default()
    };
    let cases = [
        (
            "dots past three keep the first three",
            r#"{"spaces":[{"name":"Wide","dots":[
                {"hue":10,"chroma":0.5},
                {"hue":20,"chroma":0.25},
                {"hue":30,"chroma":0.75},
                {"hue":40,"chroma":1},
                {"hue":50,"chroma":0}
            ]}]}"#,
            Spaces {
                spaces: vec![wide],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "no dots becomes the neutral one",
            r#"{"spaces":[{"name":"Plain","dots":[]}]}"#,
            Spaces {
                spaces: vec![plain("Plain")],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "grain 250 clamps to 100",
            r#"{"spaces":[{"name":"Grainy","grain":250}]}"#,
            Spaces {
                spaces: vec![Space {
                    name: "Grainy".to_owned(),
                    grain: 100,
                    ..Space::default()
                }],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "a negative grain clamps to 0",
            r#"{"spaces":[{"name":"Quiet","grain":-5}]}"#,
            Spaces {
                spaces: vec![Space {
                    name: "Quiet".to_owned(),
                    grain: 0,
                    ..Space::default()
                }],
                current: 0,
                recall: BTreeMap::new(),
            },
        ),
        (
            "current past the last space",
            r#"{"current":9,"spaces":[{"name":"A"},{"name":"B"}]}"#,
            Spaces {
                current: 1,
                recall: BTreeMap::new(),
                spaces: vec![plain("A"), plain("B")],
            },
        ),
    ];
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("spaces.json");
    for (name, bytes, want) in cases {
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(load(dir.path()), want, "{name}: {bytes}");
    }
}

#[test]
fn first_run_follows_the_accounts() {
    let none = first_run_case(&[]);
    assert_eq!(none.spaces.len(), 1, "zero accounts");
    assert_eq!(none.current, 0, "zero accounts");
    assert_eq!(none.spaces[0].name, "Space 1", "zero accounts");
    assert_eq!(none.spaces[0].dots, PRESETS[0], "zero accounts");
    assert_eq!(none.spaces[0].scope, Scope::All, "zero accounts");
    assert!(none.spaces[0].pins.is_empty(), "zero accounts");
    assert_eq!(none.spaces[0].grain, 35, "zero accounts");
    assert_eq!(none.spaces[0].theme, Theme::System, "zero accounts");
    assert_eq!(
        none.spaces[0].card_accent,
        CardAccent::Hint,
        "zero accounts"
    );

    let one_id = account(7);
    let one = first_run_case(&[one_id]);
    assert_eq!(one.spaces.len(), 1, "one account");
    assert_eq!(one.spaces[0].name, "Space 1", "one account");
    assert_eq!(one.spaces[0].dots, PRESETS[0], "one account");
    assert_eq!(
        one.spaces[0].scope,
        Scope::Accounts(vec![one_id]),
        "one account"
    );
    assert!(one.spaces[0].pins.is_empty(), "one account");

    let ids = [account(1), account(2), account(3)];
    let three = first_run_case(&ids);
    assert_eq!(three.spaces.len(), 3, "three accounts");
    assert_eq!(three.current, 0, "three accounts");
    for (index, id) in ids.iter().enumerate() {
        let space = &three.spaces[index];
        let name = format!("Space {}", index + 1);
        assert_eq!(space.name, name, "{name}");
        assert_eq!(space.dots, PRESETS[index], "{name}");
        assert_eq!(space.scope, Scope::Accounts(vec![*id]), "{name}");
        assert!(space.pins.is_empty(), "{name}");
        assert_eq!(space.grain, 35, "{name}");
    }
}

fn first_run_case(accounts: &[AccountId]) -> Spaces {
    super::first_run(accounts)
}

#[test]
fn a_new_space_takes_the_next_preset_and_keeps_the_look() {
    let spaces = Spaces {
        spaces: vec![
            Space {
                theme: Theme::Dark,
                motion: Motion::Extra,
                ..plain("Work")
            },
            plain("Home"),
        ],
        current: 0,
        recall: BTreeMap::new(),
    };
    let made = new_space(&spaces);
    assert_eq!(made.name, "Space 3");
    assert_eq!(made.dots, PRESETS[2]);
    assert_eq!((made.theme, made.motion), (Theme::Dark, Motion::Extra));
    assert_eq!(made.scope, Scope::All);
}

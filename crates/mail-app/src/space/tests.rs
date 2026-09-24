use super::{PRESETS, Pinned, Recall, Scope, Space, Spaces, load, new_space, save};
use crate::view::{Motion, Theme};
use ds::{CardAccent, Dot, Grain, SpaceLook};
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

/// The first-run look, edited.
fn look(edit: impl FnOnce(&mut SpaceLook)) -> SpaceLook {
    let mut look = Space::default().look;
    edit(&mut look);
    look
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
                        look: SpaceLook {
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
                            grain: Grain(35),
                            theme: Theme::Dark,
                            card_accent: CardAccent::Postmark,
                        },
                        motion: Motion::Calm,
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
                        look: SpaceLook {
                            dots: PRESETS[1].to_vec(),
                            grain: Grain(55),
                            theme: Theme::Light,
                            card_accent: CardAccent::SpaceHue,
                        },
                        motion: Motion::Standard,
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
                    look: look(|l| l.theme = Theme::Light),
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
                    look: look(|l| {
                        l.grain = Grain(40);
                        l.theme = Theme::System;
                    }),
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
                    look: look(|l| {
                        l.theme = Theme::Dark;
                        l.card_accent = CardAccent::SpaceHue;
                    }),
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
        look: look(|l| {
            l.dots = vec![
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
            ];
        }),
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
                    look: look(|l| l.grain = Grain(100)),
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
                    look: look(|l| l.grain = Grain(0)),
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
    assert_eq!(none.spaces[0].look.dots, PRESETS[0], "zero accounts");
    assert_eq!(none.spaces[0].scope, Scope::All, "zero accounts");
    assert!(none.spaces[0].pins.is_empty(), "zero accounts");
    assert_eq!(none.spaces[0].look.grain, Grain(35), "zero accounts");
    assert_eq!(none.spaces[0].look.theme, Theme::System, "zero accounts");
    assert_eq!(
        none.spaces[0].look.card_accent,
        CardAccent::SpaceHue,
        "zero accounts"
    );

    let one_id = account(7);
    let one = first_run_case(&[one_id]);
    assert_eq!(one.spaces.len(), 1, "one account");
    assert_eq!(one.spaces[0].name, "Space 1", "one account");
    assert_eq!(one.spaces[0].look.dots, PRESETS[0], "one account");
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
        assert_eq!(space.look.dots, PRESETS[index], "{name}");
        assert_eq!(space.scope, Scope::Accounts(vec![*id]), "{name}");
        assert!(space.pins.is_empty(), "{name}");
        assert_eq!(space.look.grain, Grain(35), "{name}");
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
                look: look(|l| l.theme = Theme::Dark),
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
    assert_eq!(made.look.dots, PRESETS[2]);
    assert_eq!((made.look.theme, made.motion), (Theme::Dark, Motion::Extra));
    assert_eq!(made.scope, Scope::All);
}

/// A `spaces.json` exactly as mailo wrote it before the look was quire's: flat look fields, the
/// card accent spelled `hint`. It must still load to the same Spaces, and a save must keep the
/// flat shape and read back unchanged.
const BEFORE_QUIRE: &str = r#"{"spaces":[{"name":"Work","dots":[{"hue":268.0,"chroma":0.72},{"hue":318.0,"chroma":0.55}],"grain":35,"theme":"dark","motion":"calm","card_accent":"hint","scope":{"kind":"all"},"pins":[{"kind":"person","name":"Dana","email":"dana@example.com"}],"colors":{}},{"name":"Home","dots":[{"hue":152.0,"chroma":0.62}],"grain":55,"theme":"system","motion":"standard","card_accent":"postmark","scope":{"kind":"all"},"pins":[],"colors":{}}],"current":1,"recall":{}}
"#;

#[test]
fn a_spaces_file_from_before_quire_still_loads_and_round_trips() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    std::fs::write(dir.path().join("spaces.json"), BEFORE_QUIRE).unwrap_or_else(|e| panic!("{e}"));
    let read = load(dir.path());
    let want = Spaces {
        current: 1,
        recall: BTreeMap::new(),
        spaces: vec![
            Space {
                name: "Work".to_owned(),
                look: SpaceLook {
                    dots: vec![
                        Dot {
                            hue: 268.0,
                            chroma: 0.72,
                        },
                        Dot {
                            hue: 318.0,
                            chroma: 0.55,
                        },
                    ],
                    grain: Grain(35),
                    theme: Theme::Dark,
                    card_accent: CardAccent::SpaceHue,
                },
                motion: Motion::Calm,
                scope: Scope::All,
                pins: vec![Pinned::Person {
                    name: "Dana".to_owned(),
                    email: "dana@example.com".to_owned(),
                }],
                colors: BTreeMap::new(),
            },
            Space {
                name: "Home".to_owned(),
                look: SpaceLook {
                    dots: vec![Dot {
                        hue: 152.0,
                        chroma: 0.62,
                    }],
                    grain: Grain(55),
                    theme: Theme::System,
                    card_accent: CardAccent::Postmark,
                },
                motion: Motion::Standard,
                scope: Scope::All,
                pins: Vec::new(),
                colors: BTreeMap::new(),
            },
        ],
    };
    assert_eq!(read, want);

    save(dir.path(), &read).unwrap_or_else(|e| panic!("{e}"));
    let written: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("spaces.json")).unwrap_or_else(|e| panic!("{e}")),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let work = &written["spaces"][0];
    // Flat, as before: the look is not nested under a key of its own.
    assert!(work.get("look").is_none(), "{work}");
    assert_eq!(work["grain"], 35, "{work}");
    assert_eq!(work["theme"], "dark", "{work}");
    assert_eq!(work["motion"], "calm", "{work}");
    assert_eq!(work["card_accent"], "space_hue", "{work}");
    assert_eq!(work["dots"][0]["hue"], 268.0, "{work}");
    assert_eq!(load(dir.path()), want, "after saving");
}

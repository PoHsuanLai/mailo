use super::*;

fn item(key: &str, name: &str) -> MenuItem {
    MenuItem {
        key: key.to_owned(),
        tile: Tile::Glyph('·'),
        name: name.to_owned(),
        help: None,
        right: Right::None,
        group: None,
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

fn three() -> Vec<MenuItem> {
    vec![item("a", "alpha"), item("b", "beta"), item("g", "gamma")]
}

#[derive(Debug)]
enum Step {
    Key(MenuKey),
    Query(&'static str),
}

#[derive(Debug)]
enum Expect {
    Active(usize),
    Pick(&'static str),
    Close,
    Ignored,
    Shown(&'static [&'static str]),
}

#[test]
fn keys_and_the_filter() {
    let items = three();
    let cases: &[(&str, &[Step], Expect)] = &[
        ("down", &[Step::Key(MenuKey::Down)], Expect::Active(1)),
        (
            "down wraps",
            &[
                Step::Key(MenuKey::Down),
                Step::Key(MenuKey::Down),
                Step::Key(MenuKey::Down),
            ],
            Expect::Active(0),
        ),
        ("up wraps", &[Step::Key(MenuKey::Up)], Expect::Active(2)),
        (
            "up from the second",
            &[Step::Key(MenuKey::Down), Step::Key(MenuKey::Up)],
            Expect::Active(0),
        ),
        (
            "enter picks the highlighted row",
            &[Step::Key(MenuKey::Down), Step::Key(MenuKey::Enter)],
            Expect::Pick("b"),
        ),
        (
            "escape closes",
            &[Step::Key(MenuKey::Escape)],
            Expect::Close,
        ),
        (
            "a query that matches nothing",
            &[Step::Query("zzz")],
            Expect::Shown(&[]),
        ),
        (
            "clearing the query brings the rows back",
            &[Step::Query("zzz"), Step::Query("")],
            Expect::Shown(&["alpha", "beta", "gamma"]),
        ),
        (
            "enter on an empty filter does not pick",
            &[Step::Query("zzz"), Step::Key(MenuKey::Enter)],
            Expect::Ignored,
        ),
        (
            "down works again once the rows return",
            &[
                Step::Query("zzz"),
                Step::Query(""),
                Step::Key(MenuKey::Down),
            ],
            Expect::Active(1),
        ),
        (
            "typing filters",
            &[Step::Key(MenuKey::Character('b'))],
            Expect::Shown(&["beta"]),
        ),
        (
            "backspace restores",
            &[
                Step::Key(MenuKey::Character('b')),
                Step::Key(MenuKey::Backspace),
            ],
            Expect::Shown(&["alpha", "beta", "gamma"]),
        ),
    ];
    for (name, steps, expect) in cases {
        let mut state = MenuState::new(true);
        let mut event = MenuEvent::Ignored;
        for step in *steps {
            match step {
                Step::Key(key) => {
                    let shown: Vec<MenuItem> = state
                        .shown(&items)
                        .into_iter()
                        .map(|shown| shown.item.clone())
                        .collect();
                    event = state.on_key(*key, &shown);
                }
                Step::Query(query) => state.set_query((*query).to_owned()),
            }
        }
        match expect {
            Expect::Active(n) => assert_eq!(state.active(), *n, "{name}"),
            Expect::Pick(key) => {
                assert_eq!(event, MenuEvent::Pick((*key).to_owned()), "{name}")
            }
            Expect::Close => assert_eq!(event, MenuEvent::Close, "{name}"),
            Expect::Ignored => assert_eq!(event, MenuEvent::Ignored, "{name}"),
            Expect::Shown(names) => {
                let got: Vec<&str> = state
                    .shown(&items)
                    .iter()
                    .map(|shown| shown.item.name.as_str())
                    .collect();
                assert_eq!(got, *names, "{name}");
            }
        }
    }
}

#[test]
fn matched_characters_are_runs_not_markup() {
    let parts = pieces("alpha", &[0, 1]);
    assert_eq!(
        parts,
        vec![Piece::Mark("al".to_owned()), Piece::Plain("pha".to_owned())]
    );
}

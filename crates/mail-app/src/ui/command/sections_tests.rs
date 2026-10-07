//! The panel's sections, as tables: which row goes under which title, in what order, how many,
//! and what a place's, a Space's or a person's row is and does.

use super::*;
use crate::ui::menu::{Run, Tone};

fn row(key: &str, name: &str, group: &str) -> MenuItem {
    MenuItem {
        key: key.to_owned(),
        tile: Tile::Icon(Icon::Command),
        name: name.to_owned(),
        help: None,
        right: Right::None,
        group: Some(group.to_owned()),
        marks: Vec::new(),
        title: Vec::new(),
        detail: Vec::new(),
    }
}

fn titles(rows: &[MenuItem]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| (row.group.clone().unwrap_or_default(), row.key.clone()))
        .collect()
}

#[test]
fn the_sections_come_in_the_panels_order_under_its_titles() {
    // As `rows_of` gives them: top hit, mail, people, actions.
    let rows = vec![
        row("action:Archive", "Archive", "Top hit"),
        row("mail:1", "Lunch", "Mail"),
        row("person:dana@example.org", "Dana", "People"),
        row("action:Compose", "Compose", "Actions"),
    ];
    let places = vec![row("place:4", "Projects", "anything")];
    let got = titles(&sections(rows, places, "a"));
    let want: Vec<(String, String)> = [
        (TOP, "action:Archive"),
        (MAIL, "mail:1"),
        (COMMANDS, "action:Compose"),
        (PLACES, "place:4"),
        (PLACES, "person:dana@example.org"),
    ]
    .into_iter()
    .map(|(group, key)| (group.to_owned(), key.to_owned()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn an_empty_field_has_recent_mail_and_no_top_hit() {
    let rows = vec![
        row("mail:1", "Newest", "Top hit"),
        row("mail:2", "Older", "Recent"),
        row("action:Compose", "Compose", "Actions"),
    ];
    let got = titles(&sections(rows, Vec::new(), "  "));
    assert_eq!(
        got,
        vec![
            (RECENT.to_owned(), "mail:1".to_owned()),
            (RECENT.to_owned(), "mail:2".to_owned()),
            (COMMANDS.to_owned(), "action:Compose".to_owned()),
        ]
    );
}

#[test]
fn each_section_is_capped() {
    const CASES: &[(&str, &str, usize)] = &[
        ("mail", "Mail", MAIL_CAP),
        ("action", "Actions", COMMANDS_CAP),
        ("person", "People", PLACES_CAP),
    ];
    for (prefix, group, cap) in CASES {
        let rows: Vec<MenuItem> = (0..20)
            .map(|index| row(&format!("{prefix}:{index}"), "x", group))
            .collect();
        assert_eq!(sections(rows, Vec::new(), "x").len(), *cap, "{prefix}");
    }
}

#[test]
fn a_menu_row_has_a_glyph_and_its_sender_at_the_end() {
    let mail = MenuItem {
        tile: Tile::Avatar {
            letter: 'D',
            color: "#000".to_owned(),
        },
        detail: vec![
            Run {
                text: "Dana".to_owned(),
                marks: Vec::new(),
                tone: Tone::Strong,
            },
            Run {
                text: " · see you at noon".to_owned(),
                marks: Vec::new(),
                tone: Tone::Plain,
            },
        ],
        ..row("mail:1", "Lunch", "Mail")
    };
    let person = MenuItem {
        tile: Tile::Avatar {
            letter: 'D',
            color: "#000".to_owned(),
        },
        ..row("person:dana@example.org", "Dana Ng", "People")
    };
    let drawn = sections(vec![mail, person], Vec::new(), "d");
    let ends: Vec<(Tile, Option<String>)> =
        drawn.into_iter().map(|row| (row.tile, row.help)).collect();
    assert_eq!(
        ends,
        vec![
            (Tile::Icon(Icon::Mail), Some("Dana".to_owned())),
            (Tile::Icon(Icon::Group), Some("dana@example.org".to_owned())),
        ]
    );
}

fn place(name: &str) -> Destination {
    Destination {
        name: name.to_owned(),
        icon: Icon::Folder,
    }
}

#[test]
fn places_and_spaces_are_found_by_name_and_none_twice() {
    let places = [place("Inbox"), place("Projects"), place("Receipts")];
    let spaces = ["Work".to_owned(), "Personal".to_owned()];
    let offered = ["Go to Inbox".to_owned()];
    const CASES: &[(&str, usize, &[&str])] = &[
        ("proj", 0, &["place:1"]),
        // Inbox is a command's already.
        ("inbox", 0, &[]),
        ("pers", 0, &["space:1"]),
        // The Space the window is in is no place to go.
        ("work", 0, &[]),
        ("work", 1, &["space:0"]),
        ("", 0, &[]),
    ];
    for (query, current, want) in CASES {
        let got: Vec<String> = places_for(query, &places, &spaces, *current, &offered)
            .into_iter()
            .map(|row| row.key)
            .collect();
        assert_eq!(got, *want, "{query:?} from Space {current}");
    }
    let found = places_for("rec", &places, &spaces, 0, &offered);
    assert_eq!(found[0].name, "Receipts");
    assert_eq!(found[0].marks, vec![0, 1, 2]);
}

#[test]
fn a_key_reads_back_as_what_its_row_does() {
    let results = Results::default();
    const CASES: &[(&str, Option<Choice>)] = &[
        ("place:3", Some(Choice::Place(3))),
        ("space:1", Some(Choice::Space(1))),
        ("place:x", None),
        ("mail:not-in-the-answer", None),
    ];
    for (key, want) in CASES {
        assert_eq!(choice_of(&results, key), *want, "{key}");
    }
}

#[test]
fn tab_completes_a_row_as_its_text() {
    const CASES: &[(&str, &str, &str)] = &[
        ("action:Settings…", "Settings…", "Settings…"),
        ("mail:1", "Lunch on Friday", "Lunch on Friday"),
        ("person:dana@example.org", "Dana", "from:dana@example.org"),
        ("place:2", "Projects", "Projects"),
    ];
    for (key, name, want) in CASES {
        assert_eq!(completion(&row(key, name, "any")), *want, "{key}");
    }
}

//! The panel's sections, as tables: which row goes under which title, in what order, how many,
//! and what a place's, a Space's or a person's row is and does.

use super::*;
use mail_ui_kit::menu::{Run, Tile, Tone};

fn row(key: &str, name: &str, group: &str) -> MenuItem {
    MenuItem {
        group: Some(group.to_owned()),
        ..MenuItem::icon(key, Icon::Command, name)
    }
}

fn titles(rows: &[MenuItem]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| (row.group.clone().unwrap_or_default(), row.key.clone()))
        .collect()
}

/// The owner's order for an empty field: where you were, where you can go, what you can do.
#[test]
fn an_empty_field_lists_recent_then_places_then_commands() {
    // As `panel_rows` gives them: Recent first, then `rows_of`'s people and actions.
    let rows = vec![
        row("recent:1", "Lunch", items::RECENT),
        row("draft:2", "Notes", items::RECENT),
        row("person:dana@example.org", "Dana", items::PEOPLE),
        row(
            "action:compose",
            mail_ui_model::copy::menus::NEW_MESSAGE,
            items::ACTIONS,
        ),
        row(
            "action:settings",
            mail_ui_model::copy::menus::SETTINGS,
            items::ACTIONS,
        ),
    ];
    let places = vec![
        row("place:0", "Inbox", "anything"),
        row("place:7", "Trash", "anything"),
    ];
    let got = titles(&sections(rows, places, "  "));
    let want: Vec<(String, String)> = [
        (RECENT, "recent:1"),
        (RECENT, "draft:2"),
        (PLACES, "place:0"),
        (PLACES, "place:7"),
        (COMMANDS, "action:compose"),
        (COMMANDS, "action:settings"),
    ]
    .into_iter()
    .map(|(group, key)| (group.to_owned(), key.to_owned()))
    .collect();
    assert_eq!(got, want);
}

/// Typed, the mail is the list's: the panel has no Mail and no Top Hit, only the places, people
/// and commands the text names.
#[test]
fn typed_text_lists_places_and_people_then_commands_and_no_mail() {
    let rows = vec![
        row("recent:1", "Lunch", items::RECENT),
        row("person:dana@example.org", "Dana", items::PEOPLE),
        row(
            "action:compose",
            mail_ui_model::copy::menus::NEW_MESSAGE,
            items::ACTIONS,
        ),
    ];
    let places = vec![row("place:4", "Projects", "anything")];
    let got = titles(&sections(rows, places, "a"));
    let want: Vec<(String, String)> = [
        (PLACES_AND_PEOPLE, "place:4"),
        (PLACES_AND_PEOPLE, "person:dana@example.org"),
        (COMMANDS, "action:compose"),
    ]
    .into_iter()
    .map(|(group, key)| (group.to_owned(), key.to_owned()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn each_typed_section_is_capped_and_an_empty_field_is_not() {
    const CASES: &[(&str, &str, usize)] = &[
        ("action", items::ACTIONS, COMMANDS_CAP),
        ("person", items::PEOPLE, PLACES_CAP),
    ];
    for (prefix, group, cap) in CASES {
        let rows: Vec<MenuItem> = (0..20)
            .map(|index| row(&format!("{prefix}:{index}"), "x", group))
            .collect();
        assert_eq!(sections(rows, Vec::new(), "x").len(), *cap, "{prefix}");
    }
    let commands: Vec<MenuItem> = (0..20)
        .map(|index| row(&format!("action:{index}"), "x", items::ACTIONS))
        .collect();
    assert_eq!(sections(commands, Vec::new(), "").len(), 20);
}

#[test]
fn a_recent_conversation_and_a_person_keep_their_avatars() {
    let face = Tile::Avatar {
        letter: 'D',
        color: "#000".to_owned(),
    };
    let mail = MenuItem {
        tile: face.clone(),
        detail: vec![Run {
            text: "Dana".to_owned(),
            marks: Vec::new(),
            tone: Tone::Plain,
        }],
        ..row("recent:1", "Lunch", items::RECENT)
    };
    let person = MenuItem {
        tile: face.clone(),
        ..row("person:dana@example.org", "Dana Ng", items::PEOPLE)
    };
    let tiles =
        |drawn: Vec<MenuItem>| -> Vec<Tile> { drawn.into_iter().map(|row| row.tile).collect() };
    assert_eq!(
        tiles(sections(vec![mail], Vec::new(), "")),
        vec![face.clone()]
    );
    assert_eq!(tiles(sections(vec![person], Vec::new(), "d")), vec![face]);
}

fn place(name: &str, at_rest: bool) -> Destination {
    Destination {
        name: name.to_owned(),
        icon: Icon::Folder,
        at_rest,
    }
}

#[test]
fn places_and_spaces_are_found_by_name_and_listed_at_rest() {
    let places = [
        place("Inbox", true),
        place("Projects", false),
        place("Receipts", false),
        place("Spam", true),
    ];
    let spaces = ["Work".to_owned(), "Personal".to_owned()];
    const CASES: &[(&str, usize, &[&str])] = &[
        ("proj", 0, &["place:1"]),
        ("inbox", 0, &["place:0"]),
        ("pers", 0, &["space:1"]),
        // The Space the window is in is no place to go.
        ("work", 0, &[]),
        ("work", 1, &["space:0"]),
        // Nothing typed: the places listed at rest, in their order, and no Space.
        ("", 0, &["place:0", "place:3"]),
    ];
    for (query, current, want) in CASES {
        let got: Vec<String> = places_for(query, &places, &spaces, *current)
            .into_iter()
            .map(|row| row.key)
            .collect();
        assert_eq!(got, *want, "{query:?} from Space {current}");
    }
    let found = places_for("rec", &places, &spaces, 0);
    assert_eq!(found[0].name, "Receipts");
    assert_eq!(found[0].marks, vec![0, 1, 2]);
}

/// Typed text lights the first row only when it starts the row's name or a word of it.
#[test]
fn typed_text_lights_a_row_it_names() {
    const CASES: &[(&str, &str, bool)] = &[
        ("Inbox", "inb", true),
        (mail_ui_model::copy::menus::ADD_ACCOUNT, "acc", true),
        (mail_ui_model::copy::menus::ADD_ACCOUNT, "add acc", true),
        (
            mail_ui_model::copy::menus::FORWARD_AS_ATTACHMENT,
            "re",
            false,
        ),
        (mail_ui_model::copy::menus::SETTINGS, "lunch", false),
        ("Inbox", "  ", false),
    ];
    for (name, query, want) in CASES {
        assert_eq!(
            lights(&row("place:0", name, "any"), query),
            *want,
            "{query:?} {name}"
        );
    }
}

#[test]
fn a_key_reads_back_as_what_its_row_does() {
    let results = Results::default();
    const CASES: &[(&str, Option<Choice>)] = &[
        ("place:3", Some(Choice::Place(3))),
        ("space:1", Some(Choice::Space(1))),
        ("place:x", None),
        ("recent:not-a-thread", None),
    ];
    for (key, want) in CASES {
        assert_eq!(choice_of(&results, key), *want, "{key}");
    }
}

#[test]
fn tab_completes_a_row_as_its_text() {
    const CASES: &[(&str, &str, &str)] = &[
        (
            "action:settings",
            mail_ui_model::copy::menus::SETTINGS,
            mail_ui_model::copy::menus::SETTINGS,
        ),
        ("person:dana@example.org", "Dana", "from:dana@example.org"),
        ("place:2", "Projects", "Projects"),
    ];
    for (key, name, want) in CASES {
        assert_eq!(completion(&row(key, name, "any")), *want, "{key}");
    }
}

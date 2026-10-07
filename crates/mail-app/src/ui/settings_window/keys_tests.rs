use super::{Control, Pair, control_of, sections, value_of, with_value, word_label};
use ds_settings::schema::KeyKind;

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

fn toggle(a: &str, b: &str) -> KeyKind {
    KeyKind::Toggle {
        variants: [a.to_owned(), b.to_owned()],
    }
}

fn switch(on: &str, off: &str) -> Control {
    Control::Switch(Pair {
        on: on.to_owned(),
        off: off.to_owned(),
    })
}

#[test]
fn each_kind_gets_the_control_detent_gives_it() {
    let cases = [
        (toggle("on", "off"), switch("on", "off")),
        (toggle("off", "on"), switch("on", "off")),
        (toggle("show", "hide"), switch("show", "hide")),
        (
            toggle("traditional", "natural"),
            switch("natural", "traditional"),
        ),
        // A choice between two things is not a switch.
        (
            toggle("icons", "letters"),
            Control::Segments(words(&["icons", "letters"])),
        ),
        (
            toggle("off", "none"),
            Control::Segments(words(&["off", "none"])),
        ),
        (
            KeyKind::Segmented {
                variants: words(&["a", "b", "c"]),
            },
            Control::PopUp(words(&["a", "b", "c"])),
        ),
        (
            KeyKind::Segmented {
                variants: words(&["a", "b"]),
            },
            Control::Segments(words(&["a", "b"])),
        ),
        (
            KeyKind::Menu {
                variants: words(&["a", "b"]),
            },
            Control::PopUp(words(&["a", "b"])),
        ),
        (KeyKind::Text, Control::Shown),
    ];
    for (kind, expect) in cases {
        assert_eq!(control_of(&kind), expect, "{kind:?}");
    }
}

#[test]
fn a_word_reads_as_its_label_or_as_itself() {
    let schema = crate::settings::schema();
    let marks = schema
        .key
        .iter()
        .find(|key| key.path.0 == "window.provider_marks")
        .expect("provider marks");
    assert_eq!(word_label(marks, "icons"), "Icons");
    assert_eq!(word_label(marks, "top_right"), "Top right");
    assert_eq!(word_label(marks, ""), "");
}

#[test]
fn a_value_is_read_and_set_by_its_path() {
    let schema = crate::settings::schema();
    let spelling = schema
        .key
        .iter()
        .find(|key| key.path.0 == "compose.spelling")
        .expect("spelling")
        .clone();
    let empty = toml::Value::Table(toml::Table::new());
    assert_eq!(
        value_of(&empty, &spelling),
        spelling.default,
        "the default when unset"
    );

    let set = with_value(
        empty,
        "compose.spelling",
        toml::Value::String("off".to_owned()),
    );
    assert_eq!(
        value_of(&set, &spelling),
        toml::Value::String("off".to_owned())
    );
    let again = with_value(
        set,
        "compose.spelling",
        toml::Value::String("on".to_owned()),
    );
    assert_eq!(
        value_of(&again, &spelling),
        toml::Value::String("on".to_owned())
    );
}

#[test]
fn sections_keep_the_order_their_first_key_comes_in() {
    let titles: Vec<String> = sections(&crate::settings::schema())
        .into_iter()
        .map(|(title, _)| title)
        .collect();
    assert_eq!(
        titles,
        ["Mail list", "Notifications", "Writing", "Reading", "Search"]
    );
}

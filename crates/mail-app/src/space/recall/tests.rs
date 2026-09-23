use super::Recall;
use crate::space::{Space, load};
use std::collections::BTreeMap;

fn plain(name: &str) -> Space {
    Space {
        name: name.to_owned(),
        ..Space::default()
    }
}

#[test]
fn a_damaged_recall_is_dropped_on_its_own() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let path = dir.path().join("spaces.json");
    let two = vec![plain("A"), plain("B")];
    let cases: &[(&str, &str, BTreeMap<usize, Recall>)] = &[
        (
            "not a map",
            r#"{"spaces":[{"name":"A"},{"name":"B"}],"recall":"garbage"}"#,
            BTreeMap::new(),
        ),
        (
            "an entry past the last Space",
            r#"{"spaces":[{"name":"A"},{"name":"B"}],"recall":{"1":{"place":"Sent"},"7":{"place":"Trash"}}}"#,
            BTreeMap::from([(
                1,
                Recall {
                    place: "Sent".to_owned(),
                    ..Recall::default()
                },
            )]),
        ),
    ];
    for (name, bytes, want) in cases {
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        let read = load(dir.path());
        assert_eq!(read.spaces, two, "{name}: the Spaces went with it");
        assert_eq!(&read.recall, want, "{name}");
    }
}

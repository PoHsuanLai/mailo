use super::{of_config, set_aside};
use std::path::{Path, PathBuf};

/// A scratch config directory named `mailo`, as the person's is, so `settings.toml` is in it.
fn scratch() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("mailo");
    std::fs::create_dir_all(&config).unwrap();
    (dir, config)
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn missing_files_and_keys_this_build_does_not_know_are_not_damage() {
    let (_dir, config) = scratch();
    assert_eq!(of_config(&config), []);
    std::fs::write(
        config.join("settings.toml"),
        "[reading]\nbrand_logos = \"on\"\n[later]\nkey = 1\n",
    )
    .unwrap();
    std::fs::write(config.join("offline.json"), r#"{"everything":[]}"#).unwrap();
    assert_eq!(of_config(&config), []);
}

#[test]
fn a_value_that_is_not_valid_is_said_and_takes_its_default_only_when_agreed() {
    let (_dir, config) = scratch();
    let body = "# mine\n[reading]\nbrand_logos = \"sometimes\"\nread_receipts = \"always\"\n";
    std::fs::write(config.join("settings.toml"), body).unwrap();

    let found = of_config(&config);
    assert_eq!(found.len(), 1);
    assert!(
        found[0].why.contains("reading.brand_logos"),
        "{}",
        found[0].why
    );
    assert_eq!(
        std::fs::read_to_string(config.join("settings.toml")).unwrap(),
        body,
        "finding writes nothing"
    );

    set_aside(&found, 1_700_000_000).unwrap();
    let after = std::fs::read_to_string(config.join("settings.toml")).unwrap();
    assert!(!after.contains("sometimes"), "{after}");
    assert!(
        after.contains("# mine") && after.contains("read_receipts = \"always\""),
        "{after}"
    );
    assert_eq!(of_config(&config), []);
}

#[test]
fn a_file_that_cannot_be_read_is_kept_beside_itself_when_agreed() {
    let (_dir, config) = scratch();
    std::fs::write(config.join("settings.toml"), "[reading\n").unwrap();
    std::fs::write(config.join("offline.json"), "{not json").unwrap();

    let found = of_config(&config);
    assert_eq!(found.len(), 2, "{found:?}");

    set_aside(&found, 1_700_000_000).unwrap();
    assert_eq!(of_config(&config), []);
    let names = names_in(&config);
    assert!(
        names.contains(&"offline.json.bad-1700000000".to_owned()),
        "{names:?}"
    );
    assert_eq!(
        std::fs::read(config.join("offline.json.bad-1700000000")).unwrap(),
        b"{not json"
    );
    let kept: Vec<&String> = names
        .iter()
        .filter(|name| name.starts_with("settings.toml.bad-"))
        .collect();
    assert_eq!(kept.len(), 1, "{names:?}");
    assert_eq!(
        std::fs::read_to_string(config.join(kept[0])).unwrap(),
        "[reading\n"
    );
}

#[test]
fn a_second_set_aside_in_the_same_second_takes_the_next_name() {
    let (_dir, config) = scratch();
    for _ in 0..2 {
        std::fs::write(config.join("offline.json"), "{not json").unwrap();
        set_aside(&of_config(&config), 7).unwrap();
    }
    let names = names_in(&config);
    assert_eq!(names, ["offline.json.bad-7", "offline.json.bad-7-1"]);
}

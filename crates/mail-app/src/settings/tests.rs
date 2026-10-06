//! The schema detent reads, and `settings.toml` read, started from the old files and written.

use super::{
    BrandLogos, MailSettings, NewMail, ProviderMarks, ServerSearch, Spelling, change, load, schema,
    store,
};
use ds_settings::ConfigRoot;
use ds_settings::schema::{KeyKind, Page, Schema};
use std::path::Path;

fn scratch() -> (tempfile::TempDir, ConfigRoot) {
    let dir = tempfile::tempdir().unwrap();
    let root = ConfigRoot::Scratch(dir.path().to_path_buf());
    (dir, root)
}

fn mailo_dir(root: &tempfile::TempDir) -> std::path::PathBuf {
    let dir = root.path().join("mailo");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) {
    std::fs::write(dir.join(name), text).unwrap();
}

#[test]
fn the_schema_names_every_key_on_mailo_s_page_in_mailo_s_file() {
    let schema = schema();
    assert_eq!(schema.app.0, "mailo");
    // detent skips a schema whose file is not `<app>/<name>.toml`.
    assert_eq!(schema.file.0, "mailo/settings.toml");
    // (path, the words it is stored as)
    let expect: &[(&str, [&str; 2])] = &[
        ("window.provider_marks", ["icons", "letters"]),
        ("notifications.new_mail", ["on", "off"]),
        ("compose.spelling", ["on", "off"]),
        ("reading.brand_logos", ["on", "off"]),
        ("search.server_automatically", ["on", "off"]),
    ];
    let got: Vec<(String, KeyKind)> = schema
        .key
        .iter()
        .map(|key| (key.path.0.clone(), key.kind.clone()))
        .collect();
    let want: Vec<(String, KeyKind)> = expect
        .iter()
        .map(|(path, [a, b])| {
            (
                (*path).to_owned(),
                KeyKind::Toggle {
                    variants: [(*a).to_owned(), (*b).to_owned()],
                },
            )
        })
        .collect();
    assert_eq!(got, want);
    assert!(
        schema
            .key
            .iter()
            .all(|key| key.page == Page::App("mailo".to_owned()))
    );
    assert!(
        schema
            .key
            .iter()
            .all(|key| !key.label.0.is_empty() && !key.help.0.is_empty())
    );
}

#[test]
fn the_schema_reads_back_as_it_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let path = schema().write_to(dir.path()).unwrap();
    assert_eq!(path.file_name().unwrap(), "mailo.settings.toml");
    let back = Schema::from_toml(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(back, schema());
}

#[test]
fn with_no_file_at_all_it_is_the_defaults_and_writes_nothing() {
    let (dir, root) = scratch();
    assert_eq!(load(&root), MailSettings::default());
    assert!(!dir.path().join("mailo").join("settings.toml").exists());
}

#[test]
fn the_old_files_start_settings_toml_once_and_stay_as_they_were() {
    let (dir, root) = scratch();
    let old = mailo_dir(&dir);
    write(&old, "mailo.toml", "marks = \"letters\"\n");
    write(&old, "notify.json", r#"{"notifications":"off"}"#);
    write(&old, "spelling.json", r#"{"spelling":"off"}"#);
    write(&old, "bimi.json", r#"{"bimi":"on"}"#);
    write(&old, "server-search.json", r#"{"automatic":"on"}"#);

    let loaded = load(&root);

    assert_eq!(loaded.window.provider_marks, ProviderMarks::Letters);
    assert_eq!(loaded.notifications.new_mail, NewMail::Off);
    assert_eq!(loaded.compose.spelling, Spelling::Off);
    assert_eq!(loaded.reading.brand_logos, BrandLogos::On);
    assert_eq!(loaded.search.server_automatically, ServerSearch::On);
    assert!(
        old.join("settings.toml").exists(),
        "settings.toml was not written"
    );
    assert_eq!(
        std::fs::read_to_string(old.join("notify.json")).unwrap(),
        r#"{"notifications":"off"}"#
    );
    // Once settings.toml exists it is the truth: an old file changed later is not read.
    write(&old, "notify.json", r#"{"notifications":"on"}"#);
    assert_eq!(load(&root).notifications.new_mail, NewMail::Off);
}

#[test]
fn appearance_json_s_marks_count_when_there_is_no_mailo_toml() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "appearance.json",
        r#"{"theme":"dark","marks":"letters"}"#,
    );
    assert_eq!(load(&root).window.provider_marks, ProviderMarks::Letters);
}

#[test]
fn a_change_is_written_and_read_back() {
    let (_dir, root) = scratch();
    let written = change(&root, |settings| {
        settings.reading.brand_logos = BrandLogos::On;
    })
    .unwrap();
    assert_eq!(written.reading.brand_logos, BrandLogos::On);
    assert_eq!(load(&root), written);
    assert_eq!(store(root).load::<MailSettings>().value, written);
}

#[test]
fn a_bad_value_costs_only_its_own_key() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[compose]\nspelling = \"sometimes\"\n\n[reading]\nbrand_logos = \"on\"\n",
    );
    let loaded = load(&root);
    assert_eq!(
        loaded.compose.spelling,
        Spelling::On,
        "the default for a bad word"
    );
    assert_eq!(loaded.reading.brand_logos, BrandLogos::On);
}

#[test]
fn a_config_directory_named_mailo_is_in_its_root_and_any_other_is_a_root() {
    let base = Path::new("/home/someone/.config");
    // (config directory, where settings.toml goes)
    let cases = [
        (base.join("mailo"), base.join("mailo").join("settings.toml")),
        (
            Path::new("/tmp/scratch-1").to_path_buf(),
            Path::new("/tmp/scratch-1/mailo/settings.toml").to_path_buf(),
        ),
    ];
    for (dir, file) in cases {
        let store = store(super::root_for(&dir));
        assert_eq!(
            store.path::<MailSettings>(),
            Some(file),
            "{}",
            dir.display()
        );
    }
}

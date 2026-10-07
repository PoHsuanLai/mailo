//! The schema detent reads, and `settings.toml` read, started from the old files and written.

use super::{
    BrandLogos, LoadRemoteImages, MailSettings, NewMail, ProviderMarks, ReadingSettings,
    ServerSearch, Spelling, change, load, schema, store,
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
    let pair = |a: &str, b: &str| KeyKind::Toggle {
        variants: [a.to_owned(), b.to_owned()],
    };
    // (path, its kind: the words it is stored as, or a list of text)
    let want: Vec<(String, KeyKind)> = [
        ("window.provider_marks", pair("icons", "letters")),
        ("notifications.new_mail", pair("on", "off")),
        ("compose.spelling", pair("on", "off")),
        ("reading.brand_logos", pair("on", "off")),
        (
            "reading.remote_images",
            KeyKind::Segmented {
                variants: ["ask", "trusted", "always"].map(str::to_owned).to_vec(),
            },
        ),
        (
            "reading.trusted_image_senders",
            KeyKind::List(Box::new(KeyKind::Text)),
        ),
        ("search.server_automatically", pair("on", "off")),
    ]
    .into_iter()
    .map(|(path, kind)| (path.to_owned(), kind))
    .collect();
    let got: Vec<(String, KeyKind)> = schema
        .key
        .iter()
        .map(|key| (key.path.0.clone(), key.kind.clone()))
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
fn load_remote_images_says_what_trusted_means_and_starts_at_ask() {
    let schema = schema();
    let key = schema
        .key
        .iter()
        .find(|key| key.path.0 == "reading.remote_images")
        .expect("reading.remote_images");
    assert_eq!(key.label.0, "Load remote images");
    assert_eq!(
        key.help.0,
        "Images from the web can tell the sender when you open a message."
    );
    assert_eq!(key.labels.of("trusted"), Some("From senders I trust"));
    assert_eq!(key.default, toml::Value::String("ask".to_owned()));
    let senders = schema
        .key
        .iter()
        .find(|key| key.path.0 == "reading.trusted_image_senders")
        .expect("reading.trusted_image_senders");
    assert_eq!(senders.default, toml::Value::Array(Vec::new()));
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

#[test]
fn remote_images_and_trusted_senders_are_written_and_read_back() {
    let (dir, root) = scratch();
    let written = change(&root, |settings| {
        settings.reading.remote_images = LoadRemoteImages::Trusted;
        settings.reading.trusted_image_senders = vec![
            "news@shop.example".to_owned(),
            "ada@example.test".to_owned(),
        ];
    })
    .unwrap();
    assert_eq!(load(&root), written);
    let text = std::fs::read_to_string(dir.path().join("mailo").join("settings.toml")).unwrap();
    assert!(text.contains("remote_images = \"trusted\""), "{text}");
    assert!(text.contains("news@shop.example"), "{text}");
}

#[test]
fn a_file_from_before_remote_images_asks_and_trusts_nobody() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[reading]\nbrand_logos = \"on\"\n",
    );
    let loaded = load(&root);
    assert_eq!(loaded.reading.brand_logos, BrandLogos::On);
    assert_eq!(loaded.reading.remote_images, LoadRemoteImages::Ask);
    assert!(loaded.reading.trusted_image_senders.is_empty());
}

#[test]
fn trusting_a_sender_keeps_one_lowercase_address_and_leaves_ask_for_trusted() {
    // (mode before, mode after)
    let cases = [
        (LoadRemoteImages::Ask, LoadRemoteImages::Trusted),
        (LoadRemoteImages::Trusted, LoadRemoteImages::Trusted),
        (LoadRemoteImages::Always, LoadRemoteImages::Always),
    ];
    for (before, after) in cases {
        let mut reading = ReadingSettings {
            remote_images: before,
            ..ReadingSettings::default()
        };
        reading.trust_images_from(" News@Shop.Example ");
        reading.trust_images_from("news@shop.example");
        assert_eq!(reading.remote_images, after, "{before:?}");
        assert_eq!(reading.trusted_image_senders, ["news@shop.example"]);
        assert!(reading.trusts_images_from("NEWS@shop.example"));
        assert!(!reading.trusts_images_from("other@shop.example"));
    }
    let mut reading = ReadingSettings::default();
    reading.trust_images_from("  ");
    assert_eq!(
        reading,
        ReadingSettings::default(),
        "a blank address is no one"
    );
}

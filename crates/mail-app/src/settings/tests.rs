//! The schema detent reads, and `settings.toml` read, started from the old files and written.

use super::{
    AttachmentSettings, BrandLogos, LoadRemoteImages, MailSettings, NewMail, ProviderMarks,
    ReadReceipts, ReadingSettings, RemoveUnedited, ServerSearch, Spelling, change, load, schema,
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
    let pair = |a: &str, b: &str| KeyKind::Toggle {
        variants: [a.to_owned(), b.to_owned()],
    };
    // (path, its kind: the words it is stored as, or a list of text)
    let want: Vec<(String, KeyKind)> = [
        ("window.provider_marks", pair("icons", "letters")),
        ("window.snippets", pair("on", "off")),
        ("window.label_chips", pair("on", "off")),
        ("window.group_by_date", pair("on", "off")),
        ("notifications.new_mail", pair("on", "off")),
        ("compose.spelling", pair("on", "off")),
        (
            "compose.copy_myself",
            KeyKind::Segmented {
                variants: ["off", "cc", "bcc"].map(str::to_owned).to_vec(),
            },
        ),
        ("compose.signature_above_quote", pair("on", "off")),
        (
            "compose.undo_send",
            KeyKind::Segmented {
                variants: ["off", "5s", "10s", "20s", "30s"]
                    .map(str::to_owned)
                    .to_vec(),
            },
        ),
        ("compose.format", pair("rich", "plain")),
        ("compose.reply_same_format", pair("on", "off")),
        ("reading.brand_logos", pair("on", "off")),
        (
            "reading.read_receipts",
            KeyKind::Segmented {
                variants: ["never", "ask", "always"].map(str::to_owned).to_vec(),
            },
        ),
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
        ("sync.while_quit", pair("on", "off")),
        ("attachments.downloads_folder", KeyKind::Text),
        (
            "attachments.remove_unedited",
            KeyKind::Segmented {
                variants: ["never", "when_mailo_quits", "after_message_is_deleted"]
                    .map(str::to_owned)
                    .to_vec(),
            },
        ),
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

/// A file written before read receipts had a key loads as Never, and the key is read once
/// written.
#[test]
fn read_receipts_are_never_sent_until_the_file_says_always() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[reading]\nbrand_logos = \"on\"\n",
    );
    assert_eq!(load(&root).reading.read_receipts, ReadReceipts::Never);
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[reading]\nread_receipts = \"always\"\n",
    );
    assert_eq!(load(&root).reading.read_receipts, ReadReceipts::Always);
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

/// A change over a `settings.toml` that cannot be used changes nothing and says why: writing
/// would put the defaults in place of what the file held.
#[test]
fn a_change_over_a_damaged_file_is_refused_and_the_file_stays() {
    let (dir, root) = scratch();
    let body = "[reading]\nbrand_logos = \"sometimes\"\n";
    write(&mailo_dir(&dir), "settings.toml", body);
    let refused = change(&root, |s| s.notifications.new_mail = NewMail::Off);
    assert!(
        refused
            .as_ref()
            .is_err_and(|why| why.contains("reading.brand_logos")),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(mailo_dir(&dir).join("settings.toml")).unwrap(),
        body
    );
}

#[test]
fn undo_send_starts_at_the_five_seconds_the_composer_had_and_each_word_is_its_wait() {
    use super::UndoSend;
    assert_eq!(
        MailSettings::default().compose.undo_send,
        UndoSend::Seconds5
    );
    let cases: [(UndoSend, &str, i64); 5] = [
        (UndoSend::Off, "off", 0),
        (UndoSend::Seconds5, "5s", 5),
        (UndoSend::Seconds10, "10s", 10),
        (UndoSend::Seconds20, "20s", 20),
        (UndoSend::Seconds30, "30s", 30),
    ];
    for (word, stored, seconds) in cases {
        assert_eq!(
            toml::Value::try_from(word).unwrap(),
            toml::Value::String(stored.to_owned()),
            "{word:?}"
        );
        assert_eq!(
            word.grace(),
            chrono::TimeDelta::seconds(seconds),
            "{word:?}"
        );
    }
    let key = schema()
        .key
        .into_iter()
        .find(|key| key.path.0 == "compose.undo_send")
        .expect("compose.undo_send");
    assert_eq!(key.labels.of("10s"), Some("10 seconds"));
}

#[test]
fn a_reply_takes_its_original_s_format_only_while_reply_same_format_is_on() {
    use super::{ComposeSettings, MessageFormat, ReplySameFormat};
    let compose = |format, reply_same_format| ComposeSettings {
        format,
        reply_same_format,
        ..ComposeSettings::default()
    };
    // (settings, the original's format, what is written)
    type Case = (ComposeSettings, Option<MessageFormat>, MessageFormat);
    let cases: [Case; 5] = [
        (ComposeSettings::default(), None, MessageFormat::Rich),
        (
            ComposeSettings::default(),
            Some(MessageFormat::Plain),
            MessageFormat::Plain,
        ),
        (
            compose(MessageFormat::Plain, ReplySameFormat::On),
            Some(MessageFormat::Rich),
            MessageFormat::Rich,
        ),
        (
            compose(MessageFormat::Plain, ReplySameFormat::On),
            None,
            MessageFormat::Plain,
        ),
        (
            compose(MessageFormat::Rich, ReplySameFormat::Off),
            Some(MessageFormat::Plain),
            MessageFormat::Rich,
        ),
    ];
    for (settings, original, want) in cases {
        assert_eq!(
            settings.format_for(original),
            want,
            "{settings:?} answering {original:?}"
        );
    }
}

/// The downloads folder and when unedited opened copies go are written and read back; a file from
/// before either had a key is the system's Downloads folder and Never.
#[test]
fn the_attachment_settings_are_written_and_read_back() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[reading]\nbrand_logos = \"on\"\n",
    );
    let before = load(&root);
    assert_eq!(before.attachments, AttachmentSettings::default());
    assert_eq!(before.attachments.downloads_in(), None);
    assert_eq!(before.attachments.remove_unedited, RemoveUnedited::Never);

    let folder = dir.path().join("Saved attachments");
    let cases = [
        RemoveUnedited::WhenMailoQuits,
        RemoveUnedited::AfterMessageIsDeleted,
        RemoveUnedited::Never,
    ];
    for remove in cases {
        let written = change(&root, |settings| {
            settings.attachments.downloads_folder = folder.display().to_string();
            settings.attachments.remove_unedited = remove;
        })
        .unwrap();
        let back = load(&root);
        assert_eq!(back, written, "{remove:?}");
        assert_eq!(back.attachments.remove_unedited, remove);
        assert_eq!(back.attachments.downloads_in(), Some(folder.clone()));
        assert_eq!(back.reading.brand_logos, BrandLogos::On, "{remove:?}");
    }
    let text = std::fs::read_to_string(dir.path().join("mailo").join("settings.toml")).unwrap();
    assert!(text.contains("[attachments]"), "{text}");
}

/// Only an absolute folder is one Save writes into: a relative one would land wherever the
/// process was started.
#[cfg(unix)]
#[test]
fn a_relative_or_blank_downloads_folder_is_the_system_s() {
    // (as written, the folder Save uses)
    let cases = [
        ("", None),
        ("   ", None),
        ("Downloads/mail", None),
        ("/srv/mail saves", Some("/srv/mail saves")),
        (" /srv/mail ", Some("/srv/mail")),
    ];
    for (written, want) in cases {
        let settings = AttachmentSettings {
            downloads_folder: written.to_owned(),
            ..AttachmentSettings::default()
        };
        assert_eq!(
            settings.downloads_in(),
            want.map(std::path::PathBuf::from),
            "{written:?}"
        );
    }
}

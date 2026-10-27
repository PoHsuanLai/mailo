//! The schema detent reads, and `settings.toml` read, started from the old files and written.

use super::{
    AttachmentSettings, BlockRemote, BrandLogos, ListOrder, LoadRemoteImages, MailSettings,
    MarkRead, MarkReadDelay, NewMail, NotifyFor, PreferPlainText, Protect, ProviderMarks,
    ReadReceipts, ReadingSettings, RelayKind, RemoveUnedited, RouteProblem, ServerSearch, Spelling,
    TextZoom, ZoomStep, change, load, schema, store,
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
        ("notifications.notify_for", pair("all", "vips")),
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
        ("reading.prefer_plain_text", pair("on", "off")),
        (
            "reading.mark_read",
            KeyKind::Segmented {
                variants: ["on_open", "after_delay", "never"]
                    .map(str::to_owned)
                    .to_vec(),
            },
        ),
        (
            "reading.mark_read_delay",
            KeyKind::Segmented {
                variants: ["two", "five", "ten"].map(str::to_owned).to_vec(),
            },
        ),
        (
            "reading.text_zoom",
            KeyKind::Bounded {
                min: 50,
                max: 300,
                unit: Some("%".to_owned()),
            },
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
        ("mailboxes.delete", pair("trash", "now")),
        (
            "mailboxes.erase_trash",
            KeyKind::Segmented {
                variants: ["never", "day", "week", "month"]
                    .map(str::to_owned)
                    .to_vec(),
            },
        ),
        ("privacy.block_remote_content", pair("off", "on")),
        (
            "privacy.protect",
            KeyKind::Segmented {
                variants: ["off", "relay", "proxy"].map(str::to_owned).to_vec(),
            },
        ),
        ("privacy.relay_kind", pair("camo", "token")),
        ("privacy.relay", KeyKind::Text),
        ("privacy.relay_key", KeyKind::Text),
        ("privacy.proxy", KeyKind::Text),
        (
            "mailboxes.erase_spam",
            KeyKind::Segmented {
                variants: ["never", "day", "week", "month"]
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

/// A list's order is kept under its place's key and read back, and is no key of the schema: a
/// Settings page has nothing to draw for it.
#[test]
fn a_list_s_order_is_written_and_read_back_and_is_not_a_key() {
    use mail_domain::{Property, Sort, SortDir};
    let (dir, root) = scratch();
    let by_size = Sort {
        property: Property::Size,
        dir: SortDir::Desc,
    };
    let written = change(&root, |settings| {
        settings
            .window
            .sorts
            .insert("place:Inbox".to_owned(), ListOrder(by_size));
    })
    .unwrap();
    assert_eq!(load(&root), written);
    assert_eq!(
        load(&root).window.sorts.get("place:Inbox"),
        Some(&ListOrder(by_size))
    );
    let text = std::fs::read_to_string(dir.path().join("mailo").join("settings.toml")).unwrap();
    assert!(text.contains("\"place:Inbox\" = \"size desc\""), "{text}");
    // An order this build cannot read costs only itself.
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[window.sorts]\n\"place:Inbox\" = \"size\"\n\"place:Sent\" = \"subject asc\"\n",
    );
    let sorts = load(&root).window.sorts;
    assert_eq!(sorts.get("place:Inbox"), None);
    assert_eq!(
        sorts.get("place:Sent"),
        Some(&ListOrder(Sort {
            property: Property::Subject,
            dir: SortDir::Asc,
        }))
    );
    assert!(
        schema().key.iter().all(|key| !key.path.0.contains("sorts")),
        "a map is not a Settings row"
    );
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

/// The reader's newer keys: a file from before them reads as today's behaviour (HTML, read on
/// open, actual size), and each is written and read back.
#[test]
fn the_reader_s_keys_start_as_before_and_are_written_and_read_back() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[reading]\nbrand_logos = \"on\"\n",
    );
    let before = load(&root).reading;
    assert_eq!(before.prefer_plain_text, PreferPlainText::Off);
    assert_eq!(before.mark_read, MarkRead::OnOpen);
    assert_eq!(before.mark_read_delay, MarkReadDelay::Two);
    assert_eq!(before.text_zoom, TextZoom(100));
    let written = change(&root, |settings| {
        settings.reading.prefer_plain_text = PreferPlainText::On;
        settings.reading.mark_read = MarkRead::AfterDelay;
        settings.reading.mark_read_delay = MarkReadDelay::Five;
        settings.reading.text_zoom = TextZoom(125);
    })
    .unwrap();
    assert_eq!(load(&root), written);
    let text = std::fs::read_to_string(mailo_dir(&dir).join("settings.toml")).unwrap();
    for line in [
        "prefer_plain_text = \"on\"",
        "mark_read = \"after_delay\"",
        "mark_read_delay = \"five\"",
        "text_zoom = 125",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    // The words a person reads for the stored ones.
    let schema = schema();
    let labels = |path: &str, word: &str| {
        schema
            .key
            .iter()
            .find(|key| key.path.0 == path)
            .and_then(|key| key.labels.of(word))
            .map(str::to_owned)
    };
    assert_eq!(
        labels("reading.mark_read", "after_delay").as_deref(),
        Some("After a Delay")
    );
    assert_eq!(
        labels("reading.mark_read_delay", "five").as_deref(),
        Some("5 Seconds")
    );
}

/// A step of the text's size climbs the ladder and holds at its ends; Actual Size is 100; a
/// size between rungs goes to the nearest one past it, and one past the range is held to it.
#[test]
fn a_zoom_step_climbs_the_ladder_and_holds_at_its_ends() {
    // (from, step, to)
    let cases = [
        (100, ZoomStep::Bigger, 110),
        (100, ZoomStep::Smaller, 90),
        (110, ZoomStep::Smaller, 100),
        (130, ZoomStep::Bigger, 150),
        (130, ZoomStep::Smaller, 125),
        (300, ZoomStep::Bigger, 300),
        (50, ZoomStep::Smaller, 50),
        (200, ZoomStep::ActualSize, 100),
        (900, ZoomStep::Smaller, 200),
        (10, ZoomStep::Bigger, 67),
    ];
    for (from, step, to) in cases {
        assert_eq!(
            TextZoom(from).stepped(step),
            TextZoom(to),
            "{from} {step:?}"
        );
    }
    // (stored, the factor drawn)
    for (stored, factor) in [(100, 1.0), (125, 1.25), (10, 0.5), (900, 3.0)] {
        assert!(
            (TextZoom(stored).factor() - factor).abs() < 1e-6,
            "{stored}"
        );
    }
}

fn private(set: impl FnOnce(&mut super::PrivacySettings)) -> super::PrivacySettings {
    let mut privacy = super::PrivacySettings::default();
    set(&mut privacy);
    privacy
}

#[test]
fn privacy_starts_straight_and_unblocked() {
    let privacy = MailSettings::default().privacy;
    assert_eq!(privacy.protect, Protect::Off);
    assert_eq!(privacy.block_remote_content, BlockRemote::Off);
    assert_eq!(privacy.route(), Ok(mail_core::remote_image::Route::Direct));
}

#[test]
fn a_relay_or_proxy_that_cannot_be_used_is_never_taken_as_off() {
    use mail_core::remote_image::Route;
    let cases: Vec<(super::PrivacySettings, Result<(), RouteProblem>)> = vec![
        (
            private(|p| p.protect = Protect::Relay),
            Err(RouteProblem::Relay),
        ),
        (
            private(|p| {
                p.protect = Protect::Relay;
                p.relay = "ftp://relay.example.test".to_owned();
                p.relay_key = "k".to_owned();
            }),
            Err(RouteProblem::Relay),
        ),
        (
            private(|p| {
                p.protect = Protect::Relay;
                p.relay = "http://relay.example.test".to_owned();
                p.relay_key = "k".to_owned();
            }),
            Err(RouteProblem::Relay),
        ),
        (
            private(|p| {
                p.protect = Protect::Relay;
                p.relay = "https://relay.example.test".to_owned();
            }),
            Err(RouteProblem::RelayKey),
        ),
        (
            private(|p| p.protect = Protect::Proxy),
            Err(RouteProblem::Proxy),
        ),
        (
            private(|p| {
                p.protect = Protect::Proxy;
                p.proxy = "127.0.0.1:9050".to_owned();
            }),
            Err(RouteProblem::Proxy),
        ),
        (
            private(|p| {
                p.protect = Protect::Proxy;
                p.proxy = "socks5h://127.0.0.1:9050".to_owned();
            }),
            Ok(()),
        ),
        (
            private(|p| {
                p.protect = Protect::Relay;
                p.relay_kind = RelayKind::Token;
                p.relay = "https://relay.example.test".to_owned();
                p.relay_key = " t ".to_owned();
            }),
            Ok(()),
        ),
    ];
    for (privacy, want) in cases {
        let got = privacy.route();
        assert_eq!(
            got.as_ref().map(|_| ()).map_err(|e| *e),
            want,
            "{privacy:?}"
        );
        if let Ok(Route::Token { token, .. }) = got {
            assert_eq!(token, "t");
        }
    }
}

/// "Notify for" is all new mail until the file says VIPs only, reads back as written, says its
/// two choices in words, and is the watch's whose-mail.
#[test]
fn notify_for_starts_at_all_new_mail_and_vips_only_is_written_and_read_back() {
    let (dir, root) = scratch();
    write(
        &mailo_dir(&dir),
        "settings.toml",
        "[notifications]\nnew_mail = \"on\"\n",
    );
    assert_eq!(load(&root).notifications.notify_for, NotifyFor::All);
    let written = change(&root, |s| s.notifications.notify_for = NotifyFor::Vips).unwrap();
    assert_eq!(load(&root), written);
    assert_eq!(load(&root).notifications.notify_for, NotifyFor::Vips);
    let text = std::fs::read_to_string(dir.path().join("mailo").join("settings.toml")).unwrap();
    assert!(text.contains("notify_for = \"vips\""), "{text}");
    let key = schema()
        .key
        .into_iter()
        .find(|key| key.path.0 == "notifications.notify_for")
        .expect("notifications.notify_for");
    assert_eq!(key.labels.of("all"), Some("All new mail"));
    assert_eq!(key.labels.of("vips"), Some("VIPs only"));
    assert_eq!(key.default, toml::Value::String("all".to_owned()));
    assert_eq!(
        mail_core::notify::Whom::from(NotifyFor::Vips),
        mail_core::notify::Whom::Vips
    );
}

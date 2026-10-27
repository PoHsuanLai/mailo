//! A settings file that is there and cannot be read is asked about before the window opens:
//! nothing of the window mounts and no file is touched until the answer, and Use Defaults keeps
//! each file beside itself and opens. The window is the launched one (`live_root`) over scratch
//! directories and fixed desktop preferences, so nothing here reads the real config.

use crate::drive::Drive;
use ds::prelude::{ShortcutKey as Key, SystemPrefs};
use ds_blitz::NetPolicy;
use ds_harness::{Clock, Harness, HarnessConfig, Viewport};
use ds_settings::{ConfigRoot, SystemPrefsSource};
use mail_core::SqliteStore;
use mail_ui::ui::appearance::WindowDirs;
use mail_ui::ui::native::{DesktopSettings, Found, damaged_files};
use std::sync::Arc;
use std::time::Duration;

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};
const ASKED: &str = ".ds-alert";
const KEYS: &str = "{\"bindings\":[{\"action\":\"archive\",\"key\":\"Enter\"}]}";
const SETTINGS: &str = "[reading]\nbrand_logos = \"sometimes\"\n";

fn until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
    for _ in 0..400 {
        if done(harness) {
            return;
        }
        harness.advance(Duration::from_millis(25));
    }
    panic!("{what}: never");
}

#[test]
fn a_damaged_settings_file_is_asked_about_and_kept_aside_before_the_window_opens() {
    let dir = tempfile::tempdir().unwrap();
    let dirs = WindowDirs {
        config: dir.path().join("mailo"),
        state: dir.path().join("state"),
    };
    std::fs::create_dir_all(&dirs.config).unwrap();
    std::fs::create_dir_all(&dirs.state).unwrap();
    std::fs::create_dir_all(dir.path().join("data/blobs")).unwrap();
    std::fs::write(dirs.config.join("keyboard.json"), KEYS).unwrap();
    std::fs::write(dirs.config.join("settings.toml"), SETTINGS).unwrap();
    let mail = SqliteStore::open(
        dir.path().join("data/mail.db"),
        dir.path().join("data/blobs"),
    )
    .unwrap();

    let found = damaged_files(&dirs);
    assert_eq!(found.len(), 2, "{found:?}");
    let contexts = mail_ui::ui::native::contexts(
        mail_client::Client::open(Arc::new(mail), mail_client::ClientConfig::default()),
        mail_ui::ui::view::Appearance::default(),
        None,
        Some(dirs.clone()),
        mail_ui::ui::Start::Inbox,
    )
    .with(Found(found))
    .with(DesktopSettings {
        root: ConfigRoot::Scratch(dir.path().join("desktop")),
        prefs: SystemPrefsSource::Fixed(SystemPrefs::default()),
    });
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut window = Harness::new(mail_ui::ui::native::live_root, config);

    until(&mut window, "the question", |h| h.count(ASKED) == 1);
    let said = window.text_of(ASKED).unwrap_or_default();
    assert!(
        said.contains("keyboard.json") && said.contains("settings.toml"),
        "{said:?}"
    );
    assert_eq!(
        window.count(".card"),
        0,
        "the window opened before the answer"
    );
    assert_eq!(
        std::fs::read_to_string(dirs.config.join("keyboard.json")).unwrap(),
        KEYS
    );
    assert_eq!(
        std::fs::read_to_string(dirs.config.join("settings.toml")).unwrap(),
        SETTINGS
    );

    // Return is Use Defaults, once the alert takes an answer.
    window.advance(Duration::from_millis(1500));
    window.key(Key::Enter);
    until(&mut window, "the window opens", |h| {
        h.count(ASKED) == 0 && h.count(".card") == 1
    });

    assert!(!dirs.config.join("keyboard.json").exists());
    let kept: Vec<String> = std::fs::read_dir(&dirs.config)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("keyboard.json.bad-"))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(
        std::fs::read_to_string(dirs.config.join(&kept[0])).unwrap(),
        KEYS
    );
    let settings = std::fs::read_to_string(dirs.config.join("settings.toml")).unwrap();
    assert!(!settings.contains("sometimes"), "{settings:?}");
    assert_eq!(damaged_files(&dirs), []);
}

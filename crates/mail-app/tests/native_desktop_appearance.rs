//! mailo follows the one desktop appearance, live: the launched window's root over a scratch
//! `quire/appearance.toml`, changed while the window runs, as detent or the control centre changes
//! it. The window is the real one (`mail_app::ui::native::live_root`, with its watch on the
//! settings directory); the store it watches is a `TempDir` standing in for `$XDG_CONFIG_HOME`
//! and the desktop's preferences are fixed, so nothing here reads or watches the real
//! `~/.config`, and nothing reaches the session bus.

use ds::prelude::*;
use ds_blitz::NetPolicy;
use ds_harness::{Clock, Driver, Harness, HarnessConfig, Viewport};
use ds_settings::{AppName, AppearanceFile, ConfigRoot, Store, SystemPrefsSource};
use mail_app::ui::native::DesktopSettings;
use mail_store::SqliteStore;
use std::sync::Arc;
use std::time::{Duration, Instant};

const VIEW: Viewport = Viewport {
    width: 1200,
    height: 800,
    scale_percent: 100,
};

/// The real time the file watch (a thread, a 30 ms debounce, a task) may take to deliver a change.
const WAIT_BOUND: Duration = Duration::from_secs(30);

fn desktop(root: &std::path::Path) -> Store {
    Store::new(ConfigRoot::Scratch(root.to_path_buf()), AppName::QUIRE)
}

fn saved(store: &Store, theme: Theme, accent: Accent, motion: Motion) {
    let mut file = AppearanceFile::default();
    file.appearance.theme = theme;
    file.appearance.accent = accent;
    file.appearance.motion_level = motion;
    store.save(&file).unwrap_or_else(|e| panic!("{e}"));
}

/// The window's `div.ds` root tag, which stamps the resolved theme, accent and motion.
fn stamps(harness: &Harness) -> (String, String, String) {
    let html = harness.html();
    let attr = |name: &str| {
        let key = format!("{name}=\"");
        html.split(&key)
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_default()
            .to_owned()
    };
    (attr("data-theme"), attr("data-accent"), attr("data-motion"))
}

/// Advance the window until `done`, waiting real time for the watch thread, which the virtual
/// clock cannot reach.
fn settle_until(harness: &mut Harness, done: impl Fn(&Harness) -> bool) {
    let deadline = Instant::now() + WAIT_BOUND;
    while Instant::now() < deadline && !done(harness) {
        harness.advance(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        done(harness),
        "no restyle within {WAIT_BOUND:?}: {:?}",
        stamps(harness)
    );
}

#[test]
fn a_change_to_the_desktops_appearance_restyles_the_running_window() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let config = dir.path().join("config");
    let data = dir.path().join("data");
    std::fs::create_dir_all(data.join("blobs")).unwrap_or_else(|e| panic!("{e}"));
    let mail = SqliteStore::open(data.join("mail.db"), data.join("blobs"))
        .unwrap_or_else(|e| panic!("{e}"));

    let store = desktop(&config);
    saved(&store, Theme::Light, Accent::Blue, Motion::Standard);

    let contexts = mail_app::ui::native::contexts(
        Arc::new(mail),
        mail_app::ui::view::Appearance::default(),
        mail_app::ui::space::Spaces::default(),
        None,
        mail_app::ui::Start::Inbox,
    )
    .with(DesktopSettings {
        root: ConfigRoot::Scratch(config.clone()),
        prefs: SystemPrefsSource::Fixed(SystemPrefs::default()),
    });
    let config = HarnessConfig::new(VIEW)
        .with_net(NetPolicy::Local)
        .with_clock(Clock::Virtual)
        .with_contexts(contexts);
    let mut harness = Harness::new(mail_app::ui::native::live_root, config);

    settle_until(&mut harness, |h| stamps(h).0 == "light");
    assert_eq!(
        stamps(&harness),
        (
            "light".into(),
            Accent::Blue.slug().into(),
            "standard".into()
        ),
        "the file's look, on the first frame it can be read"
    );

    // The dark-mode switch: another program saves the one file.
    saved(&store, Theme::Dark, Accent::Blue, Motion::Standard);
    settle_until(&mut harness, |h| stamps(h).0 == "dark");

    // The accent and motion follow the same way, with the theme kept.
    saved(&store, Theme::Dark, Accent::Green, Motion::Reduced);
    settle_until(&mut harness, |h| stamps(h).1 == Accent::Green.slug());
    assert_eq!(
        stamps(&harness),
        ("dark".into(), Accent::Green.slug().into(), "reduced".into())
    );
}

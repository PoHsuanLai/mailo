//! The root of the Settings window: the sidebar and its pages, and the sheets they open.
//!
//! quire gives this VirtualDom the same root contexts as the first window. What the window keeps
//! for itself is made here again: its `Shell` (which page is shown, which sheet is open), its
//! `revision`, its toast host. What it changes it tells the other windows by moving the shared
//! revision: a setting, a key binding, an account added or removed. It opens where the main
//! window asked for last ([`super::SettingsAsked`]), a page or an account's page, and turns to
//! each place asked for while it is open.

use super::{SettingsAsked, SettingsView};
use crate::ui::app::Frame;
use crate::ui::appearance::WindowDirs;
use crate::ui::space::Spaces;
use crate::ui::style::STYLE;
use crate::ui::view::{SettingsPage, Shell};
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds_settings::use_environment;
use mail_store::SqliteStore;
use std::sync::Arc;

/// The window as quire opens it: the desktop's settings and mailo's own watched as the first
/// window's root watches them, then [`SettingsShell`].
pub(super) fn settings_window() -> Element {
    rsx! { Watched {} }
}

#[component]
fn Watched() -> Element {
    let desktop = crate::ui::launch::DesktopSettings::current();
    let spawner: Arc<dyn Spawner> = Arc::new(ds_blitz::TokioSpawner::current());
    let environment = use_environment(desktop.store(), desktop.prefs.clone(), spawner);
    use_context_provider(|| environment);
    crate::ui::prefs::use_window_settings();
    rsx! { SettingsShell {} }
}

/// The window as a test drives it: [`SettingsShell`] without the watches on the settings
/// directory.
pub fn settings_root() -> Element {
    rsx! { SettingsShell {} }
}

#[component]
fn SettingsShell() -> Element {
    let loaded = try_consume_context::<mail_core::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    crate::ui::provider_chip::use_fetch_missing(icons);
    crate::ui::host::use_window_host();
    let dirs = try_consume_context::<WindowDirs>();
    let asked = try_consume_context::<SettingsAsked>();
    let first = asked.as_ref().map(SettingsAsked::at).unwrap_or_default();
    let shell = use_signal({
        let dirs = dirs.clone();
        move || {
            let store = consume_context::<Arc<SqliteStore>>();
            Shell {
                settings: Some(first.page()),
                accounts_pane: first.pane(),
                appearance: try_consume_context::<crate::ui::view::Appearance>()
                    .unwrap_or_default(),
                keymap: dirs
                    .as_ref()
                    .map(|dirs| crate::ui::keymap::load(&dirs.config))
                    .unwrap_or_default(),
                labels: mail_core::query::known_labels(&store),
                accounts: mail_core::compose::sending_accounts(&store),
                ..Shell::default()
            }
        }
    });
    // Each place the main window asks for while this window is open.
    use_future(move || {
        let asked = asked.clone();
        async move {
            let Some(asked) = asked else { return };
            let mut heard = asked.heard();
            heard.mark_unchanged();
            while heard.changed().await.is_ok() {
                let at = heard.borrow_and_update().clone();
                super::go_to(shell, at);
            }
        }
    });
    let revision = use_signal(|| 0u64);
    crate::ui::revisions::use_shared_revision(revision);
    // The Spaces, read so the frame wears the Space on screen; the first window owns the file.
    let spaces = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(crate::ui::space::load)
                .or_else(try_consume_context::<Spaces>)
                .unwrap_or_else(|| crate::ui::space::first_run(&[]))
        }
    });
    let _ = crate::ui::prefs::use_prefs(dirs.as_ref());
    crate::ui::hover::use_hover();
    crate::ui::motion::use_motion();
    // Key bindings and Spaces another window writes; this one tells them its own through
    // `frame::keep` and the Keyboard page. Settings are watched by the root.
    crate::ui::frame::use_followed_configuration(shell, spaces, None);

    let keys = crate::ui::actions::use_window_keys();
    let on_key = move |event: Event<KeyboardData>| {
        let key = event.key().to_string();
        // The Keyboard page takes every key while an action waits for one: the key pressed to be
        // bound must do nothing else, Escape included, which stops the wait.
        if crate::ui::keyboard::capturing(&shell.read()) {
            let held = event.modifiers();
            let key = if held.shift() {
                crate::ui::view::shifted(&key).to_owned()
            } else {
                key
            };
            let chord = crate::ui::actions::is_chord(keys, &event.key(), held);
            crate::ui::keyboard::pressed(shell, &key, chord);
            return;
        }
        // Escape with the keyboard outside the Accounts pane (on the window itself, or the
        // sidebar) still goes back from an account's page; inside it the pane's stack took it.
        let pushed = !shell.read().accounts_pane.path.is_root();
        if key == "Escape" && pushed && shell.read().settings == Some(SettingsPage::Accounts) {
            super::account::back(shell);
        }
    };

    rsx! {
        Frame { spaces,
            style { {STYLE} }
            div { class: "app",
                tabindex: "0",
                onmounted: crate::ui::host::Host::app_mounted,
                onkeydown: on_key,
                SettingsView { shell, revision }
                crate::ui::motion::Toast { shell, revision }
            }
        }
    }
}

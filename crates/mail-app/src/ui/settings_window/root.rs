//! The root of the Settings window: the sidebar and its pages, and the sheets they open.
//!
//! quire gives this VirtualDom the same root contexts as the first window. What the window keeps
//! for itself is made here again: its `Shell` (which page is shown, which sheet is open), its
//! `revision`, its toast host. What it changes it tells the other windows by moving the shared
//! revision: a setting, a key binding, an account added or removed.

use super::SettingsView;
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
    crate::ui::host::use_window_host();
    let dirs = try_consume_context::<WindowDirs>();
    let shell = use_signal({
        let dirs = dirs.clone();
        move || {
            let store = consume_context::<Arc<SqliteStore>>();
            Shell {
                settings: Some(SettingsPage::General),
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
    let revision = use_signal(|| 0u64);
    crate::ui::revisions::use_shared_revision(revision);
    // The Spaces, read so the frame wears the Space on screen; the first window owns the file.
    let spaces = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(|dirs| crate::ui::space::load(&dirs.config))
                .or_else(try_consume_context::<Spaces>)
                .unwrap_or_default()
        }
    });
    let _ = crate::ui::prefs::use_prefs(dirs.as_ref());
    crate::ui::hover::use_hover();
    crate::ui::motion::use_motion();
    // Key bindings and Spaces another window writes; this one tells them its own through
    // `frame::keep` and the keyboard sheet. Settings are watched by the root.
    crate::ui::frame::use_followed_configuration(shell, spaces, None);

    let on_key = move |event: Event<KeyboardData>| {
        let key = event.key().to_string();
        // The keyboard sheet takes every key: the one pressed to be bound must do nothing else.
        if shell.read().keyboard.is_some() {
            let held = event.modifiers();
            let key = if held.shift() {
                crate::ui::view::shifted(&key).to_owned()
            } else {
                key
            };
            let chord = held.ctrl() || held.alt() || held.meta();
            crate::ui::keyboard::pressed(shell, &key, chord);
            return;
        }
        if key != "Escape" {
            return;
        }
        let open = shell.read().clone();
        if open.account_sheet.is_some() {
            crate::ui::account_settings::escape(shell);
        } else if open.adding.is_some() {
            crate::ui::add_account::close(shell);
        } else if open.contacts.is_some() {
            crate::ui::contacts::close(shell);
        } else if open.rules.is_some() {
            crate::ui::rules::close(shell);
        } else if open.keys.is_some() {
            crate::ui::pgp::keys::close(shell);
        }
    };

    rsx! {
        Frame { spaces,
            style { {STYLE} }
            div { class: "app settings-window",
                tabindex: "0",
                onmounted: crate::ui::host::Host::app_mounted,
                onkeydown: on_key,
                SettingsView { shell, revision }
                if shell.read().contacts.is_some() {
                    crate::ui::contacts::ContactsSheet { shell }
                }
                if shell.read().adding.is_some() {
                    crate::ui::add_account::AddAccountSheet { shell, revision, spaces }
                }
                if shell.read().rules.is_some() {
                    crate::ui::rules::RulesSheet { shell, revision }
                }
                if shell.read().keys.is_some() {
                    crate::ui::pgp::keys::KeysSheet { shell }
                }
                if shell.read().account_sheet.is_some() {
                    crate::ui::account_settings::AccountSettingsSheet { shell, revision }
                }
                if shell.read().keyboard.is_some() {
                    crate::ui::keyboard::KeyboardSheet { shell }
                }
                crate::ui::motion::Toast { shell, revision }
            }
        }
    }
}

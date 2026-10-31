//! The root of the Settings window: the sidebar and its pages, and the sheets they open.
//!
//! quire gives this VirtualDom the same root contexts as the first window. What the window keeps
//! for itself is made here again: its `Shell` (which page is shown, which sheet is open) and its
//! toast host. An account added or removed the other windows hear from the client; a setting or a
//! key binding it tells them (`ui::configured`). It opens where the main
//! window asked for last ([`super::SettingsAsked`]), a page or an account's page, and turns to
//! each place asked for while it is open.

use super::{SettingsAsked, SettingsView};
use crate::ui::app::Frame;
use crate::ui::style::STYLE;
use dioxus::prelude::*;
use ds::base::spawner::Spawner;
use ds::components::app::spaces::{Switched, use_spaces};
use ds_settings::use_environment;
use mail_ui_kit::appearance::WindowDirs;
use mail_ui_kit::space::{Recall, Spaces};
use mail_ui_kit::view::{PageId, Shell};
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
    mail_ui_kit::prefs::use_window_settings();
    rsx! { SettingsShell {} }
}

/// The window as a test drives it: [`SettingsShell`] without the watches on the settings
/// directory.
pub fn settings_root() -> Element {
    rsx! { SettingsShell {} }
}

#[component]
fn SettingsShell() -> Element {
    let loaded = try_consume_context::<mail_client::provider::icon::Loaded>().unwrap_or_default();
    let icons = use_signal(|| loaded);
    use_context_provider(|| icons);
    crate::ui::provider_chip::use_fetch_missing(icons);
    mail_ui_kit::host::use_window_host();
    let dirs = try_consume_context::<WindowDirs>();
    let asked = try_consume_context::<SettingsAsked>();
    let first = asked.as_ref().map(SettingsAsked::at).unwrap_or_default();
    let shell = use_signal({
        let dirs = dirs.clone();
        move || {
            let client = mail_ui_kit::live::client();
            Shell {
                settings: Some(first.page()),
                accounts_pane: first.pane(),
                spaces_pane: first.spaces_pane(),
                contacts_pane: first.contacts_pane(),
                appearance: try_consume_context::<mail_ui_kit::view::Appearance>()
                    .unwrap_or_default(),
                keymap: dirs
                    .as_ref()
                    .map(|dirs| mail_ui_kit::keymap::load(&dirs.config))
                    .unwrap_or_default(),
                labels: client.query().known_labels(),
                accounts: client.compose().sending_accounts(),
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
    // The Spaces, so the frame wears the Space on screen and the Spaces page edits them: a
    // handle of the window's own, which writes each edit (`frame::keep`) for the other windows to
    // read again. It never switches Space but by adding one.
    let handle = use_spaces(
        {
            let dirs = dirs.clone();
            move || {
                dirs.as_ref()
                    .map(mail_ui_kit::space::load)
                    .or_else(try_consume_context::<Spaces>)
                    .unwrap_or_else(|| mail_ui_kit::space::first_run(&[]))
            }
        },
        |spaces: &Spaces| crate::ui::frame::keep(spaces),
        Recall::default,
        |_: Switched<Recall>| {},
    );
    use_context_provider(|| handle);
    let spaces = handle.spaces();
    let _ = mail_ui_kit::prefs::use_prefs(dirs.as_ref());
    mail_ui_kit::hover::use_hover();
    mail_ui_kit::motion::use_motion();
    let keys = mail_ui_kit::actions::use_window_keys();
    // Key bindings and Spaces another window writes; this one tells them its own through
    // `frame::keep` and the Keyboard page. Settings are watched by the root.
    crate::ui::frame::use_followed_configuration(shell, spaces, Some(handle), keys);
    let on_key = move |event: Event<KeyboardData>| {
        let key = event.key().to_string();
        // Escape with the keyboard outside the Accounts pane (on the window itself, or the
        // sidebar) still goes back from an account's page; inside it the pane's stack took it.
        let pushed = !shell.read().accounts_pane.path.is_root();
        if key == "Escape" && pushed && shell.read().settings == Some(PageId::ACCOUNTS) {
            super::account::back(shell);
        }
        let space_pushed = !shell.read().spaces_pane.is_root();
        if key == "Escape" && space_pushed && shell.read().settings == Some(PageId::SPACES) {
            super::spaces::back(shell, handle);
        }
        let card_pushed = !shell.read().contacts_pane.is_root();
        if key == "Escape" && card_pushed && shell.read().settings == Some(PageId::CONTACTS) {
            crate::ui::contacts::page::back(shell);
        }
        let reply_pushed = !shell.read().vacation_pane.is_root();
        if key == "Escape" && reply_pushed && shell.read().settings == Some(PageId::VACATION) {
            crate::ui::rules::vacation_back(shell);
        }
    };

    rsx! {
        Frame { spaces,
            style { {STYLE} }
            div { class: "app",
                tabindex: "0",
                onmounted: mail_ui_kit::host::Host::app_mounted,
                onkeydown: on_key,
                SettingsView { shell }
                mail_ui_kit::motion::Toast { shell }
            }
        }
    }
}

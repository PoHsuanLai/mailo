//! Before the window opens over a settings file that is there and cannot be read: a dialog that
//! names each and asks. Use Defaults sets them aside (`mail_ui_model::damage::set_aside`) and opens;
//! Quit ends mailo with every file as it was. Nothing in the window mounts, and so nothing reads
//! or writes those files, before the answer.

use super::Shell;
use crate::ui::app::Frame;
use dioxus::prelude::*;
use ds::components::overlays::alert_model::{AlertButton, AlertRole, AlertStyle};
use ds::prelude::*;
use mail_ui_kit::appearance::WindowDirs;
use mail_ui_kit::common::Problem;
use mail_ui_kit::{keymap, space, today};
use mail_ui_model::copy;
use mail_ui_model::damage::{Damaged, moved, of_config, set_aside};

/// The damaged files the window was opened over: a root context, found by [`found`] before the
/// window opened. None, or no context, opens the window at once.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Found(pub Vec<Damaged>);

/// Every file the window reads that is there and cannot be used: the settings and offline
/// choices, the keys and the Spaces in the config directory, and Today in the state directory.
pub fn found(dirs: &WindowDirs) -> Vec<Damaged> {
    let config = &dirs.config;
    let mut found = of_config(config);
    found.extend(moved(config, keymap::FILE_NAME, keymap::damage(config)));
    found.extend(moved(config, space::FILE_NAME, space::damage(dirs)));
    found.extend(moved(
        &dirs.state,
        today::FILE_NAME,
        today::damage(&dirs.state),
    ));
    found
}

/// The window, or first the question when it was opened over damaged files.
#[component]
pub(super) fn Gate() -> Element {
    let mut found_now = use_signal(|| try_consume_context::<Found>().unwrap_or_default().0);
    let mut failed = use_signal(|| None::<String>);
    // The dialog wears the first-run look: the Spaces may be among what cannot be read.
    let spaces = use_signal(|| space::first_run(&[]));
    if found_now.read().is_empty() {
        return rsx! { Shell {} };
    }
    let files: Vec<(String, String)> = found_now
        .read()
        .iter()
        .map(|damaged| (file_name(damaged), damaged.why.clone()))
        .collect();
    let use_defaults = move |()| {
        let now = chrono::Utc::now().timestamp();
        if let Err(why) = set_aside(&found_now.peek(), now) {
            failed.set(Some(why));
        }
        // What is left: everything once the files are set aside, else what a failure left.
        let left = try_consume_context::<WindowDirs>()
            .map(|dirs| found(&dirs))
            .unwrap_or_default();
        found_now.set(left);
    };
    let quit = move |()| {
        if let Some(app) = try_consume_context::<ds_blitz::AppHandle>() {
            let _ = app.quit();
        }
    };
    rsx! {
        Frame { spaces,
            match failed() {
                Some(why) => rsx! {
                    Problem {
                        title: copy::titles::NOT_SET_ASIDE,
                        why,
                        onclose: move |()| failed.set(None),
                    }
                },
                None => rsx! {
                    Alert {
                        title: copy::titles::SETTINGS_UNREADABLE.to_owned(),
                        message: Some(TextLine::from(copy::confirm::settings_unreadable(&files))),
                        style: AlertStyle::Warning,
                        buttons: vec![
                            AlertButton::new(copy::controls::USE_DEFAULTS, AlertRole::Normal, EventHandler::new(use_defaults)),
                            AlertButton::new(copy::controls::QUIT, AlertRole::Cancel, EventHandler::new(quit)),
                        ],
                    }
                },
            }
        }
    }
}

/// The file's own name, which says which setting it is; the whole path when it has none.
fn file_name(damaged: &Damaged) -> String {
    damaged.path.file_name().map_or_else(
        || damaged.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

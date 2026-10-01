//! Whether new mail is said on the desktop: the window's half of `mailo notify on|off`.
//!
//! The window's, not the Space's, like provider marks, so a choice is kept at once — in
//! `notify.json`, beside `appearance.json`, which is where `mailo watch` reads it. The watch
//! raises the notifications; this only says whether it should.

use crate::appearance::WindowDirs;
use crate::notify::{self, Setting};
use dioxus::prelude::*;
use ds::components::content::label::LabelRole;
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::FieldRow;
use ds::prelude::{Choice, Label, SegmentedControl};

const CHOICES: [(Setting, &str); 2] = [(Setting::On, "On"), (Setting::Off, "Off")];

#[component]
pub(super) fn Notifications() -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let mut now = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(|dirs| notify::load(&dirs.config))
                .unwrap_or_default()
        }
    });
    let mut failed = use_signal(|| None::<String>);
    let current = now();
    rsx! {
        FieldRow {
            label: "New mail on the desktop",
            help: match current {
                Setting::On => "mailo watch shows new unread inbox mail as it arrives.",
                Setting::Off => "mailo watch keeps new mail to itself.",
            },
            SegmentedControl::<Setting> {
                label: "Notifications",
                choices: CHOICES
                    .iter()
                    .map(|(setting, name)| Choice::new(*setting, *name))
                    .collect::<Vec<_>>(),
                tracking: Tracking::SelectOne(current),
                onchange: move |setting: Setting| {
                    let kept = match &dirs {
                        Some(dirs) => notify::save(&dirs.config, setting),
                        None => Err("There is no config directory to keep this in.".to_owned()),
                    };
                    match kept {
                        Ok(()) => {
                            now.set(setting);
                            failed.set(None);
                        }
                        Err(why) => failed.set(Some(why)),
                    }
                },
            }
            if let Some(why) = failed() {
                Label { text: why, role: LabelRole::Secondary }
            }
        }
    }
}

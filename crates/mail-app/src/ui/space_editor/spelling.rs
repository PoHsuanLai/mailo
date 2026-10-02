//! Whether the composer marks misspelt words, and why it cannot when there is no dictionary.
//!
//! The window's, not the Space's, like notifications: kept at once in `spelling.json`, and put
//! in the desk, which every open composer reads, so marks come and go as it changes. A window
//! with no config directory keeps the choice in memory until it closes.

use super::parts::Seg;
use crate::ui::appearance::WindowDirs;
use crate::ui::compose::{Desk, dictionaries};
use crate::ui::spelling::{self, Setting};
use dioxus::prelude::*;

const CHOICES: [(Setting, &str); 2] = [(Setting::On, "On"), (Setting::Off, "Off")];

#[component]
pub(super) fn Spelling() -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let desk = try_use_context::<Desk>();
    // Read once as the sheet opens: it lists the dictionary directories.
    let found = use_hook(dictionaries);
    let mut failed = use_signal(|| None::<String>);
    let current = desk.map(|desk| (desk.spelling)()).unwrap_or_default();
    let missing = match current {
        Setting::On => found.missing(),
        Setting::Off => None,
    };
    rsx! {
        div {
            div { class: "ed-label", "Spelling" }
            Seg {
                label: "Check spelling".to_owned(),
                options: CHOICES
                    .iter()
                    .map(|(setting, name)| ((*name).to_owned(), *setting == current))
                    .collect::<Vec<_>>(),
                on_pick: move |index: usize| {
                    let setting = CHOICES[index % CHOICES.len()].0;
                    let kept = match &dirs {
                        Some(dirs) => spelling::save(&dirs.config, setting),
                        None => Ok(()),
                    };
                    match kept {
                        Ok(()) => {
                            if let Some(mut desk) = desk {
                                desk.spelling.set(setting);
                            }
                            failed.set(None);
                        }
                        Err(why) => failed.set(Some(why)),
                    }
                },
            }
            p { class: "capnote",
                match current {
                    Setting::On => "Misspelt words in a message are underlined; right-click one for suggestions.",
                    Setting::Off => "Messages are not checked for spelling.",
                }
            }
            if let Some(missing) = missing {
                p { class: "capnote", "{missing}" }
            }
            if let Some(why) = failed() {
                p { class: "capnote", "{why}" }
            }
        }
    }
}

//! Show brand logos (BIMI): the window's switch, off unless turned on.
//!
//! The window's, not the Space's, like notifications: kept at once in `bimi.json`, which the
//! reader and the sender card read each time they look for a logo, so a change applies to the
//! next message opened. Says so when no mark verifying authority's root is installed, since then
//! no logo can be verified and none will show.

use super::parts::Seg;
use crate::appearance::WindowDirs;
use crate::bimi::{self, Setting};
use dioxus::prelude::*;

const CHOICES: [(Setting, &str); 2] = [(Setting::On, "On"), (Setting::Off, "Off")];

#[component]
pub(super) fn BrandLogos() -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let mut current = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(|dirs| bimi::load(&dirs.config))
                .unwrap_or_default()
        }
    });
    // Read once as the sheet opens: the shipped roots and the user's file.
    let roots = use_hook({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map_or(0, |dirs| bimi::anchors(&dirs.config).len())
        }
    });
    let mut failed = use_signal(|| None::<String>);
    let now = current();
    rsx! {
        div {
            div { class: "ed-label", "Brand logos" }
            Seg {
                label: "Show brand logos (BIMI)".to_owned(),
                options: CHOICES
                    .iter()
                    .map(|(setting, name)| ((*name).to_owned(), *setting == now))
                    .collect::<Vec<_>>(),
                on_pick: move |index: usize| {
                    let setting = CHOICES[index % CHOICES.len()].0;
                    let kept = match &dirs {
                        Some(dirs) => bimi::save(&dirs.config, setting),
                        None => Err("There is no config directory to keep this in.".to_owned()),
                    };
                    match kept {
                        Ok(()) => {
                            current.set(setting);
                            failed.set(None);
                        }
                        Err(why) => failed.set(Some(why)),
                    }
                },
            }
            p { class: "capnote",
                match now {
                    Setting::On => "A sender's logo replaces their initial when the receiving server says DMARC passed and a mark certificate vouches for the logo. Looking one up asks DNS and the sender's web server.",
                    Setting::Off => "Show brand logos (BIMI). Off: no logo is looked up or fetched.",
                }
            }
            if now == Setting::On && roots == 0 {
                p { class: "capnote",
                    "No mark verifying authority's root is installed, so no logo can be verified yet. Roots can be added to {bimi::USER_ROOTS} in the config directory."
                }
            }
            if let Some(why) = failed() {
                p { class: "capnote", "{why}" }
            }
        }
    }
}

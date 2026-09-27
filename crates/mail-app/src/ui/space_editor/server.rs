//! Search the server automatically: the window's switch, off unless turned on.
//!
//! The window's, not the Space's, like brand logos: kept at once in `server-search.json`, which
//! the list reads when a search is shown. Off, a search's list ends with a button per account
//! that asks its server; on, the servers are asked as the search is shown.

use super::parts::Seg;
use crate::appearance::WindowDirs;
use crate::server_search::{self, Automatic};
use dioxus::prelude::*;

const CHOICES: [(Automatic, &str); 2] = [(Automatic::On, "On"), (Automatic::Off, "Off")];

#[component]
pub(super) fn ServerSearch() -> Element {
    let dirs = try_consume_context::<WindowDirs>();
    let mut current = use_signal({
        let dirs = dirs.clone();
        move || {
            dirs.as_ref()
                .map(|dirs| server_search::load(&dirs.config))
                .unwrap_or_default()
        }
    });
    let mut failed = use_signal(|| None::<String>);
    let now = current();
    rsx! {
        div {
            div { class: "ed-label", "Search the server" }
            Seg {
                label: "Search the server automatically".to_owned(),
                options: CHOICES
                    .iter()
                    .map(|(setting, name)| ((*name).to_owned(), *setting == now))
                    .collect::<Vec<_>>(),
                on_pick: move |index: usize| {
                    let setting = CHOICES[index % CHOICES.len()].0;
                    let kept = match &dirs {
                        Some(dirs) => server_search::save(&dirs.config, setting),
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
                    Automatic::On => "A search shown in the list is also asked of each account's server, and what it finds is fetched as headers.",
                    Automatic::Off => "Off: a search's list ends with a button that asks each account's server.",
                }
            }
            if let Some(why) = failed() {
                p { class: "capnote", "{why}" }
            }
        }
    }
}

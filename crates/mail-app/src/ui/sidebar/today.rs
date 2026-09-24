//! Today: threads opened in this Space, as sidebar shortcuts.

use super::super::hover::{Hook, corner, hover};
use super::super::icon::{Glyph, Icon};
use super::super::text::sender;
use crate::appearance::WindowDirs;
use crate::space;
use crate::today::Today;
use crate::view::Shell;
use dioxus::prelude::*;
use mail_domain::ThreadId;
use mail_store::{SqliteStore, Store};
use std::sync::Arc;

#[component]
pub(super) fn TodayList(
    shell: Signal<Shell>,
    today: Signal<Today>,
    space_index: usize,
    dirs: Option<WindowDirs>,
    just_added: Signal<Option<ThreadId>>,
) -> Element {
    let mut leaving = use_signal(|| None::<ThreadId>);
    let live = today.read().live(space_index, chrono::Utc::now());
    let mut shown = live.clone();
    if let Some(id) = leaving()
        && !shown.contains(&id)
    {
        shown.push(id);
    }
    let store = consume_context::<Arc<SqliteStore>>();
    let rows: Vec<_> = shown
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(index, id)| {
            let loaded = store.thread(id).ok()?;
            let title = loaded.summary.subject.clone();
            let mut letter = String::new();
            if let Some(ch) = sender(&loaded.summary).chars().next() {
                letter.extend(ch.to_uppercase());
            }
            if letter.is_empty() {
                letter.push('?');
            }
            Some((index, id, title, letter))
        })
        .collect();
    let dirs_clear = dirs.clone();
    rsx! {
        div { class: "s-h",
            "Today"
            if !live.is_empty() {
                button {
                    id: "clear-today",
                    onclick: move |_| {
                        today.write().clear(space_index);
                        save(&dirs_clear, &today.read());
                        leaving.set(None);
                    },
                    "Clear"
                }
            }
        }
        super::super::compose::ParkedDrafts { shell, space_index }
        super::super::compose::ScheduledDrafts { shell }
        if shown.is_empty() && today.read().parked(space_index).is_empty() {
            p { class: "today-hint",
                "Threads you open land here, like tabs. They drop off after 12 idle hours; the mail stays where it is."
            }
        }
        for (index, id, title, letter) in rows {
            {
                let dirs_row = dirs.clone();
                let color = space::AVATAR[index % space::AVATAR.len()];
                let class = if leaving() == Some(id) {
                    "item today-item leaving"
                } else if just_added() == Some(id) {
                    "item today-item entering"
                } else {
                    "item today-item"
                };
                rsx! {
                    div {
                        key: "{id}",
                        class: "{class}",
                        role: "button",
                        "data-hc": "today:{id}",
                        tabindex: "0",
                        onclick: move |_| shell.write().open(id),
                        onpointerenter: move |event| {
                            if let Some(hover) = hover() {
                                hover.enter(Hook::Today(id), corner(&event));
                            }
                        },
                        onpointerleave: move |_| {
                            if let Some(hover) = hover() {
                                hover.leave();
                            }
                        },
                        onanimationend: move |_| {
                            if leaving() == Some(id) {
                                leaving.set(None);
                            }
                            if just_added() == Some(id) {
                                just_added.set(None);
                            }
                        },
                        span { class: "fav", style: "background:{color}", "{letter}" }
                        span { class: "t", "{title}" }
                        button {
                            class: "x",
                            aria_label: "Close {id}",
                            onclick: move |event| {
                                event.stop_propagation();
                                today.write().close(space_index, id);
                                save(&dirs_row, &today.read());
                                leaving.set(Some(id));
                            },
                            Glyph { icon: Icon::X, class: None }
                        }
                    }
                }
            }
        }
    }
}

fn save(dirs: &Option<WindowDirs>, today: &Today) {
    if let Some(dirs) = dirs {
        let _ = crate::today::save(&dirs.state, today);
    }
}

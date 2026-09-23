//! The Ctrl T menu: a field, the operator chips, and the grouped results.
//!
//! The rows are built in [`items`]; this file is the overlay and what a pick does.

mod items;

use super::field::{Field, FieldKind};
use super::icon::Icon;
use super::menu::{Menu, MenuItem, MenuKey, MenuState};
use super::ops::start_new;
use crate::view::{Appearance, PageMenu, Shell, Theme};
use chrono::Utc;
use dioxus::prelude::*;
use items::{Pick, interpret, rows_of, search_now, tokens};
use mail_store::SqliteStore;
use std::sync::Arc;

/// The centred overlay. Open while `shell.command` is `Some`.
#[component]
pub(super) fn CommandMenu(
    shell: Signal<Shell>,
    pages: Signal<u32>,
    revision: Signal<u64>,
    side_hidden: Signal<bool>,
    sync_state: Signal<crate::view::SyncState>,
    spaces: Signal<crate::space::Spaces>,
    in_a_field: Signal<bool>,
) -> Element {
    let query = shell.read().command.clone().unwrap_or_default();
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    let now = Utc::now();
    let (results, names) = search_now(&store, &query, now);
    let items = rows_of(&results, &names, &query);
    let chips = tokens(&query);
    let mut keys = use_signal(|| MenuState::new(false));
    let mut seen = use_signal(String::new);
    if seen() != query {
        seen.set(query.clone());
        keys.write().restart();
    }
    let active = keys.read().active().min(items.len().saturating_sub(1));
    let placeholder = "Search mail, people, actions · try from:dana or has:attachment".to_owned();
    rsx! {
        div {
            class: "cmdk-wrap",
            onclick: move |_| close(shell),
            div {
                class: "cmdk",
                role: "dialog",
                aria_label: "Search and commands",
                onclick: move |event| event.stop_propagation(),
                onkeydown: move |event| {
                    let Some(key) = super::menu::menu_key(&event.key().to_string()) else {
                        return;
                    };
                    if matches!(key, MenuKey::Character(_) | MenuKey::Backspace) {
                        return;
                    }
                    event.stop_propagation();
                    let current = rows_of_now(shell);
                    match keys.write().on_key(key, &current) {
                        super::menu::MenuEvent::Pick(key) => act(
                            shell, pages, revision, side_hidden, sync_state, spaces, &key,
                        ),
                        super::menu::MenuEvent::Close => close(shell),
                        _ => {}
                    }
                },
                div { class: "cmdk-in",
                    super::icon::Glyph { icon: Icon::Search, class: None }
                    Field {
                        kind: FieldKind::Inline,
                        value: query.clone(),
                        placeholder,
                        extra: None,
                        on_input: move |value| {
                            shell.write().command = Some(value);
                        },
                        on_focus: move |_| in_a_field.set(true),
                        on_blur: move |_| in_a_field.set(false),
                    }
                }
                if !chips.is_empty() {
                    div { class: "tokens",
                        for chip in chips {
                            span { key: "{chip}", class: "tok", "{chip}" }
                        }
                    }
                }
                Menu {
                    title: String::new(),
                    items,
                    filterable: false,
                    on_pick: move |key: String| {
                        act(shell, pages, revision, side_hidden, sync_state, spaces, &key);
                    },
                    on_close: move |_| close(shell),
                    on_query: move |_| {},
                    slim: false,
                    active: Some(active),
                }
            }
        }
    }
}

fn rows_of_now(shell: Signal<Shell>) -> Vec<MenuItem> {
    let query = shell.read().command.clone().unwrap_or_default();
    let store = consume_context::<Arc<SqliteStore>>();
    let (results, names) = search_now(&store, &query, Utc::now());
    rows_of(&results, &names, &query)
}

fn close(mut shell: Signal<Shell>) {
    shell.write().command = None;
    shell.write().page_menu = PageMenu::Closed;
    dioxus::document::eval("document.querySelector('.app')?.focus()");
}

fn act(
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    mut revision: Signal<u64>,
    mut side_hidden: Signal<bool>,
    mut sync_state: Signal<crate::view::SyncState>,
    spaces: Signal<crate::space::Spaces>,
    key: &str,
) {
    let store = consume_context::<Arc<SqliteStore>>();
    let query = shell.read().command.clone().unwrap_or_default();
    let (results, _) = search_now(&store, &query, Utc::now());
    let Some(pick) = interpret(&results, key) else {
        return;
    };
    match pick {
        Pick::Open(id) => {
            shell.write().open(id);
            close(shell);
        }
        Pick::From(email) => {
            shell.write().search = format!("from:{email}");
            pages.set(1);
            close(shell);
        }
        Pick::Action(label) => run_action(
            shell,
            pages,
            &mut revision,
            &mut side_hidden,
            &mut sync_state,
            spaces,
            &label,
        ),
    }
}

fn run_action(
    mut shell: Signal<Shell>,
    mut pages: Signal<u32>,
    revision: &mut Signal<u64>,
    side_hidden: &mut Signal<bool>,
    sync_state: &mut Signal<crate::view::SyncState>,
    spaces: Signal<crate::space::Spaces>,
    label: &str,
) {
    if let Some(place) = label.strip_prefix("Go to ") {
        let index = shell.read().places.iter().position(|one| one.name == place);
        if let Some(index) = index {
            shell.write().select(index);
            pages.set(1);
        }
        close(shell);
        return;
    }
    match label {
        "Compose" => {
            let store = consume_context::<Arc<SqliteStore>>();
            let known = shell.peek().accounts.clone();
            match start_new(&store, &known) {
                Ok(draft) => {
                    shell.write().compose(&draft);
                    *revision += 1;
                }
                Err(why) => eprintln!("compose: {why}"),
            }
            close(shell);
        }
        "Sync now" => {
            if sync_state.read().may_start() {
                sync_state.set(crate::view::SyncState::Running);
                let store = consume_context::<Arc<SqliteStore>>();
                let mut sync_state = *sync_state;
                let mut revision = *revision;
                spawn(async move {
                    let done = tokio::task::spawn_blocking(move || {
                        crate::sync::run(store, chrono::Utc::now())
                    })
                    .await;
                    sync_state.set(match done {
                        Ok(result) => crate::view::synced(result.map(|ran| ran.text)),
                        Err(e) => crate::view::synced(Err(format!("the sync pass stopped: {e}"))),
                    });
                    revision += 1;
                });
            }
            close(shell);
        }
        "Hide sidebar" => {
            side_hidden.set(!side_hidden());
            close(shell);
        }
        "Theme light" | "Theme dark" | "Theme system" => {
            let theme = match label {
                "Theme light" => Theme::Light,
                "Theme dark" => Theme::Dark,
                _ => Theme::System,
            };
            let look = Appearance {
                theme,
                ..shell.read().appearance
            };
            let space = spaces.read().current_space();
            shell.write().appearance = look;
            dioxus::document::eval(&super::launch::appearance_script(look, &space));
            if let Some(dir) = crate::appearance::config_dir() {
                let _ = crate::appearance::save(&dir, look);
            }
            close(shell);
        }
        _ => close(shell),
    }
}

#[cfg(test)]
pub(in crate::ui) mod tests;

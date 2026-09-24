//! Ctrl T's "New from template": the same overlay, listing every template through the one
//! [`Menu`]. Picking one starts a draft from it and opens that draft as a composer page; the ×
//! deletes one.

use std::sync::Arc;

use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::Draft;
use mail_store::SqliteStore;

use super::super::compose::{every_template, forget_template, template_rows};
use super::super::field::{Field, FieldKind};
use super::super::icon::{Glyph, Icon};
use super::super::menu::{Menu, MenuEvent, MenuKey, MenuState, menu_key};
use super::super::motion::{Follow, tell};
use crate::view::Shell;

#[cfg(test)]
#[path = "templates_tests.rs"]
mod tests;

/// The action's label, as the Actions group lists it.
pub(super) const ACTION: &str = "New from template";

/// Start a draft from the template `key` names, addressed as the template is.
pub(in crate::ui) fn start(store: &SqliteStore, key: &str) -> Result<Draft, String> {
    let all = every_template(store);
    let id = all
        .iter()
        .map(|(_, template)| template.id)
        .find(|id| id.to_string() == key)
        .ok_or_else(|| "that template is gone".to_owned())?;
    crate::template::start(store, id, &[], Utc::now())
}

/// The overlay, while Ctrl T is listing templates. The field narrows the list.
#[component]
pub(super) fn TemplateMenu(
    shell: Signal<Shell>,
    revision: Signal<u64>,
    in_a_field: Signal<bool>,
) -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    // Bumped by a delete, so the list is read again.
    let mut deleted = use_signal(|| 0u64);
    let _ = deleted();
    let all = every_template(&store);
    let typed = shell.read().command.clone().unwrap_or_default();
    let items = template_rows(&all, &typed);
    let mut keys = use_signal(|| MenuState::new(false));
    let active = keys.read().active().min(items.len().saturating_sub(1));
    let empty = all.is_empty();
    let mut pick = move |key: String| {
        let store = consume_context::<Arc<SqliteStore>>();
        match start(&store, &key) {
            Ok(draft) => {
                shell.write().compose(&draft);
                let mut revision = revision;
                revision += 1;
                super::close(shell);
            }
            Err(why) => tell(why, Follow::Nothing),
        }
    };
    let remove = move |key: String| {
        let store = consume_context::<Arc<SqliteStore>>();
        match forget_template(&store, &key) {
            Ok(said) => tell(said, Follow::Nothing),
            Err(why) => tell(why, Follow::Nothing),
        }
        deleted += 1;
    };
    rsx! {
        div {
            class: "cmdk-wrap",
            onclick: move |_| super::close(shell),
            div {
                class: "cmdk",
                role: "dialog",
                aria_label: "New from template",
                onclick: move |event| event.stop_propagation(),
                onkeydown: move |event| {
                    let Some(key) = menu_key(&event.key().to_string()) else {
                        return;
                    };
                    if matches!(key, MenuKey::Character(_) | MenuKey::Backspace) {
                        return;
                    }
                    event.stop_propagation();
                    let store = consume_context::<Arc<SqliteStore>>();
                    let typed = shell.peek().command.clone().unwrap_or_default();
                    let current = template_rows(&every_template(&store), &typed);
                    match keys.write().on_key(key, &current) {
                        MenuEvent::Pick(key) => pick(key),
                        MenuEvent::Close => super::close(shell),
                        _ => {}
                    }
                },
                div { class: "cmdk-in",
                    Glyph { icon: Icon::FilePen, class: None }
                    Field {
                        kind: FieldKind::Inline,
                        value: typed,
                        placeholder: "New from template · type to narrow".to_owned(),
                        extra: None,
                        on_input: move |value| {
                            keys.write().restart();
                            shell.write().command = Some(value);
                        },
                        on_focus: move |_| in_a_field.set(true),
                        on_blur: move |_| in_a_field.set(false),
                    }
                }
                if empty {
                    p { class: "cmdk-none",
                        "No templates yet. In a message, type / and choose Save as template…"
                    }
                } else {
                    Menu {
                        title: "Templates".to_owned(),
                        items,
                        filterable: false,
                        on_pick: pick,
                        on_close: move |_| super::close(shell),
                        on_query: move |_| {},
                        slim: false,
                        active: Some(active),
                        on_remove: remove,
                    }
                }
            }
        }
    }
}

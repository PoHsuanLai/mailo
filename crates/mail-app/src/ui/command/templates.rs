//! Ctrl T's "New from template": the same overlay, listing every template through the one
//! [`Menu`]. Picking one starts a draft from it and opens that draft as a composer page; the ×
//! deletes one.

use std::sync::Arc;

use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::Draft;
use mail_store::SqliteStore;

use super::super::compose::{every_template, forget_template, template_rows};
use super::super::menu::quire_rows;
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

/// The overlay, while Ctrl T is listing templates: quire's palette again. The field narrows
/// the list; each row's × deletes its template.
#[component]
pub(super) fn TemplateMenu(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let store = use_hook(consume_context::<Arc<SqliteStore>>);
    // Bumped by a delete, so the list is read again.
    let mut deleted = use_signal(|| 0u64);
    let _ = deleted();
    let all = every_template(&store);
    let typed = shell.read().command.clone().unwrap_or_default();
    let items = template_rows(&all, &typed);
    let empty = if all.is_empty() {
        "No templates yet. In a message, type / and choose Save as template…"
    } else {
        "Nothing matches."
    };
    let pick = move |key: String| {
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
    let remove = EventHandler::new(move |key: String| {
        let store = consume_context::<Arc<SqliteStore>>();
        match forget_template(&store, &key) {
            Ok(said) => tell(said, Follow::Nothing),
            Err(why) => tell(why, Follow::Nothing),
        }
        deleted += 1;
    });
    let rows = quire_rows(&items, ds::AvatarSize::Size34, Some(remove));
    rsx! {
        ds::CommandPalette::<String> {
            label: "New from template".to_owned(),
            placeholder: "New from template · type to narrow".to_owned(),
            query: typed,
            tokens: Vec::new(),
            groups: vec![("Templates".to_owned(), rows)],
            empty: empty.to_owned(),
            entrance: ds::PaletteEntrance::Opaque,
            oninput: move |value| {
                shell.write().command = Some(value);
            },
            onpick: pick,
            onclose: move |()| super::close(shell),
            onkey: move |event: KeyboardEvent| super::toggle_key(&event, shell),
        }
    }
}

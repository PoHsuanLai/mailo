//! The search bar's "New from template": the same panel, listing every template. Picking one
//! starts a draft from it and opens that draft as a composer page. A template is deleted from
//! the composer's own list of them, whose rows carry the ×; a menu row has none.

use std::sync::Arc;

use chrono::Utc;
use dioxus::prelude::*;
use mail_domain::Draft;
use mail_store::SqliteStore;

use super::super::compose::{every_template, template_rows};
use super::super::menu::MenuItem;
use super::super::motion::{Follow, tell};
use crate::ui::view::Shell;

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
    mail_core::template::start(store, id, &[], Utc::now()).map_err(String::from)
}

/// The panel's rows while it lists the templates, narrowed by `typed`.
pub(super) fn rows(store: &SqliteStore, typed: &str) -> Vec<MenuItem> {
    template_rows(&every_template(store), typed)
}

/// Start a draft from the template `key` names and open it as a composer page.
pub(super) fn run(shell: Signal<Shell>, mut revision: Signal<u64>, key: &str) {
    let store = consume_context::<Arc<SqliteStore>>();
    match start(&store, key) {
        Ok(draft) => {
            let mut shell = shell;
            shell.write().compose(&draft);
            revision += 1;
            super::close(shell);
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}

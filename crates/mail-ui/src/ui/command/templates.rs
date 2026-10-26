//! The search bar's "New from template": the same panel, listing every template. Picking one
//! starts a draft from it and opens that draft as a composer page. A template is deleted from
//! the composer's own list of them, whose rows carry the ×; a menu row has none.

use chrono::Utc;
use dioxus::prelude::*;
use mail_client::Client;
use mail_domain::Draft;

use super::super::compose::{every_template, template_rows};
use super::super::menu::MenuItem;
use super::super::motion::{Follow, tell};
use crate::ui::view::Shell;

/// Start a draft from the template `key` names, addressed as the template is.
pub(in crate::ui) fn start(client: &Client, key: &str) -> Result<Draft, String> {
    let all = every_template(client);
    let id = all
        .iter()
        .map(|(_, template)| template.id)
        .find(|id| id.to_string() == key)
        .ok_or_else(|| "that template is gone".to_owned())?;
    client
        .template()
        .start(id, &[], Utc::now())
        .map_err(crate::ui::remedy::told)
}

/// The panel's rows while it lists the templates, narrowed by `typed`.
pub(super) fn rows(client: &Client, typed: &str) -> Vec<MenuItem> {
    template_rows(&every_template(client), typed)
}

/// Start a draft from the template `key` names and open it as a composer page.
pub(super) fn run(shell: Signal<Shell>, key: &str) {
    let client = crate::ui::live::client();
    match start(&client, key) {
        Ok(draft) => {
            let mut shell = shell;
            shell.write().compose(&draft);
            super::close(shell);
        }
        Err(why) => tell(why, Follow::Nothing),
    }
}

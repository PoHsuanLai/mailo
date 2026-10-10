//! Templates from the command line: keep a draft as one, start a draft from one, and tidy up.
//!
//! Local only, and deliberately so: a template never goes to the server's Drafts folder, where
//! another client would list it as a message waiting to be sent. See `mail_domain::template`.

use crate::error::CoreError;
use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};

/// Keep `draft` as a template called `name`, returning it. The draft is left where it was.
pub fn save(
    store: &SqliteStore,
    draft: DraftId,
    name: &str,
    now: DateTime<Utc>,
) -> Result<Template, CoreError> {
    let draft = store.draft(draft)?;
    let template = Template::from_draft(&draft, name, now);
    store.put_template(&template)?;
    Ok(template)
}

/// Start a new draft from a template, returning it. The template is left as it was.
///
/// `to`, when not empty, replaces the template's `To`: the usual reason to start from a
/// template is the same words to someone else. `Cc` and `Bcc` come from the template as kept.
pub fn start(
    store: &SqliteStore,
    template: TemplateId,
    to: &[Address],
    now: DateTime<Utc>,
) -> Result<Draft, CoreError> {
    let template = store.template(template)?;
    let mut draft = template.draft(now);
    if !to.is_empty() {
        draft.to = to.to_vec();
    }
    crate::compose::save(store, &draft)?;
    Ok(draft)
}

/// Delete a template, returning the name it had.
pub fn delete(store: &SqliteStore, template: TemplateId) -> Result<String, CoreError> {
    let kept = store.template(template)?;
    store.delete_template(template)?;
    Ok(kept.name)
}

/// Every template on every account, as `(account address, template)`, accounts oldest first.
pub fn all(store: &SqliteStore) -> Result<Vec<(String, Template)>, CoreError> {
    let mut out = Vec::new();
    for (address, account) in crate::compose::sending_accounts(store) {
        for template in store.templates(account)? {
            out.push((address.clone(), template));
        }
    }
    Ok(out)
}

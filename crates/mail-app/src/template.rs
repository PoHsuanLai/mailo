//! Templates from the command line: keep a draft as one, start a draft from one, and tidy up.
//!
//! Local only, and deliberately so: a template never goes to the server's Drafts folder, where
//! another client would list it as a message waiting to be sent. See `mail_domain::template`.

use chrono::{DateTime, Utc};
use mail_domain::*;
use mail_store::{SqliteStore, Store};
use std::fmt::Write as _;

/// Keep `draft` as a template called `name`, returning it. The draft is left where it was.
pub fn save(
    store: &SqliteStore,
    draft: DraftId,
    name: &str,
    now: DateTime<Utc>,
) -> Result<Template, String> {
    let draft = store.draft(draft).map_err(|e| e.to_string())?;
    let template = Template::from_draft(&draft, name, now);
    store.put_template(&template).map_err(|e| e.to_string())?;
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
) -> Result<Draft, String> {
    let template = store.template(template).map_err(|e| e.to_string())?;
    let mut draft = template.draft(now);
    if !to.is_empty() {
        draft.to = to.to_vec();
    }
    crate::compose::save(store, &draft)?;
    Ok(draft)
}

/// Delete a template, returning the name it had.
pub fn delete(store: &SqliteStore, template: TemplateId) -> Result<String, String> {
    let kept = store.template(template).map_err(|e| e.to_string())?;
    store.delete_template(template).map_err(|e| e.to_string())?;
    Ok(kept.name)
}

/// Every template on every account, as `(account address, template)`, accounts oldest first.
pub fn all(store: &SqliteStore) -> Result<Vec<(String, Template)>, String> {
    let mut out = Vec::new();
    for (address, account) in crate::compose::sending_accounts(store) {
        for template in store.templates(account).map_err(|e| e.to_string())? {
            out.push((address.clone(), template));
        }
    }
    Ok(out)
}

/// `mailo template save`, as the CLI reports it.
pub fn save_report(
    store: &SqliteStore,
    draft: DraftId,
    name: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let kept = save(store, draft, name, now)?;
    Ok(format!(
        "template {}\n  name    {}\n\nstart a message from it with: mailo template use {}\n",
        kept.id, kept.name, kept.id
    ))
}

/// `mailo template use`, as the CLI reports it: the new draft's id first, on a line of its own.
pub fn start_report(
    store: &SqliteStore,
    template: TemplateId,
    to: &[Address],
    now: DateTime<Utc>,
) -> Result<String, String> {
    let draft = start(store, template, to, now)?;
    let mut out = format!("draft {}\n", draft.id);
    let _ = writeln!(out, "  to      {}", addresses(&draft.to));
    let _ = writeln!(out, "  subject {}", or_none(&draft.subject));
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        let _ = writeln!(
            out,
            "\nthe template names nobody to send to: start it again with --to someone@example.com"
        );
    } else {
        let _ = writeln!(out, "\nsend it with: mailo send {}", draft.id);
    }
    Ok(out)
}

/// `mailo template list`.
pub fn list(store: &SqliteStore) -> Result<String, String> {
    let all = all(store)?;
    if all.is_empty() {
        return Ok(
            "no templates. Keep a draft as one with: mailo template save <draft-id>\n".to_owned(),
        );
    }
    let mut out = String::new();
    for (address, template) in all {
        let _ = writeln!(
            out,
            "{}  {}  ({}) {}",
            template.id,
            template.name,
            address,
            or_none(&template.subject)
        );
    }
    Ok(out)
}

fn or_none(subject: &str) -> &str {
    if subject.is_empty() {
        "(no subject)"
    } else {
        subject
    }
}

fn addresses(list: &[Address]) -> String {
    list.iter()
        .map(|a| a.email.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

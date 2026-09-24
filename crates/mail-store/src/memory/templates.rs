//! Templates in the in-memory store. Kept in step with `sqlite/template.rs`, which the parity
//! tests in `tests/templates.rs` hold it to.

use super::Inner;
use crate::StoreError;
use mail_domain::{AccountId, Template, TemplateId};

impl Inner {
    pub(super) fn template(&self, id: TemplateId) -> Result<Template, StoreError> {
        self.templates
            .get(&id)
            .cloned()
            .ok_or(StoreError::NoTemplate(id))
    }

    pub(super) fn templates_of(&self, account: AccountId) -> Vec<Template> {
        let mut out: Vec<Template> = self
            .templates
            .values()
            .filter(|t| t.account == account)
            .cloned()
            .collect();
        // SQLite's `ORDER BY name COLLATE NOCASE, id`: `NOCASE` folds ASCII letters only, and a
        // hyphenated lowercase UUID sorts as text the way its bytes sort.
        out.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    pub(super) fn put_template(&mut self, template: &Template) {
        self.accounts.insert(template.account);
        self.templates.insert(template.id, template.clone());
    }

    pub(super) fn delete_template(&mut self, id: TemplateId) -> Result<(), StoreError> {
        self.templates
            .remove(&id)
            .map(|_| ())
            .ok_or(StoreError::NoTemplate(id))
    }
}

//! Rules and vacation replies in the in-memory store. Kept in step with `sqlite/rules.rs`, which
//! the parity tests in `tests/rules.rs` hold it to.

use super::Inner;
use crate::StoreError;
use mail_domain::{AccountId, Label, Rule, RuleId, Vacation};

impl Inner {
    pub(super) fn rules_of(&self, account: AccountId) -> Vec<Rule> {
        let mut out: Vec<Rule> = self
            .rules
            .values()
            .filter(|r| r.account == account)
            .cloned()
            .collect();
        // SQLite's `ORDER BY position, name`, binary collation.
        out.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then_with(|| a.name.cmp(&b.name))
        });
        out
    }

    pub(super) fn put_rule(&mut self, rule: &Rule) -> Result<(), StoreError> {
        let taken = self
            .rules
            .values()
            .any(|r| r.account == rule.account && r.name == rule.name && r.id != rule.id);
        if taken {
            return Err(StoreError::RuleNameTaken(rule.name.clone()));
        }
        self.accounts.insert(rule.account);
        self.rules.insert(rule.id, rule.clone());
        Ok(())
    }

    pub(super) fn delete_rule(&mut self, id: RuleId) -> Result<(), StoreError> {
        self.rules
            .remove(&id)
            .map(|_| ())
            .ok_or(StoreError::NoRule(id))
    }

    pub(super) fn put_vacation(&mut self, account: AccountId, vacation: Option<&Vacation>) {
        match vacation {
            None => {
                self.vacations.remove(&account);
            }
            Some(v) => {
                self.accounts.insert(account);
                self.vacations.insert(account, v.clone());
            }
        }
    }

    /// Every label on the account, by name in binary order, as SQLite's `ORDER BY name`.
    pub(super) fn labels_of_account(&self, account: AccountId) -> Vec<Label> {
        let mut out: Vec<Label> = self
            .labels
            .values()
            .filter(|l| l.account == account)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        out
    }
}

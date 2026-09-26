//! Contact groups in memory. Kept in step with `sqlite/groups.rs`, which the parity tests in
//! `tests/groups.rs` hold it to.

use super::Inner;
use crate::contact::{Group, GroupId};

impl Inner {
    pub(super) fn every_group(&self) -> Vec<Group> {
        let mut out: Vec<Group> = self.groups.values().cloned().collect();
        // SQLite's `ORDER BY name COLLATE NOCASE, id`: `NOCASE` folds ASCII letters only.
        out.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        out
    }

    pub(super) fn one_group(&self, id: &GroupId) -> Option<Group> {
        self.groups.get(id).cloned()
    }

    pub(super) fn write_group(&mut self, group: &Group) {
        self.groups.insert(group.id.clone(), group.clone());
    }

    pub(super) fn drop_group(&mut self, id: &GroupId) -> bool {
        self.groups.remove(id).is_some()
    }
}

//! Saved views in the in-memory store. Kept in step with `sqlite/views.rs`, which the parity
//! tests in `tests/store/views.rs` hold it to.

use super::Inner;
use crate::StoreError;
use mail_domain::{View, ViewId};

impl Inner {
    /// SQLite's `ORDER BY position, id`: a hyphenated lowercase UUID sorts as text the way its
    /// bytes sort, which is how `ViewId` orders.
    pub(super) fn views_in_order(&self) -> Vec<View> {
        let mut out: Vec<&(i64, View)> = self.views.values().collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
        out.into_iter().map(|(_, view)| view.clone()).collect()
    }

    pub(super) fn put_view(&mut self, view: &View) {
        let last = self.views.values().map(|(position, _)| *position).max();
        let position = match self.views.get(&view.id) {
            Some((kept, _)) => *kept,
            None => last.unwrap_or(0) + 1,
        };
        self.views.insert(view.id, (position, view.clone()));
    }

    pub(super) fn delete_view(&mut self, id: ViewId) -> Result<(), StoreError> {
        self.views
            .remove(&id)
            .map(|_| ())
            .ok_or(StoreError::NoView(id))
    }
}

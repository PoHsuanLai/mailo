//! Tests of `mail-ui-kit` that seed a store or mount the whole window: they run over the
//! window's fixtures (`ui::fixtures`), which the kit cannot have, as it cannot name `mail-app`'s
//! shell or a store.

mod data;
mod history;
mod host;
mod list_search;
mod live;
mod motion;
mod ops;
mod store_reads;

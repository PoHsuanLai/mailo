//! The components, actually executed.
//!
//! Everything else about the shell is tested through `view.rs` and `reader.rs`, which are
//! free of Dioxus on purpose. That leaves the components themselves — and a component can
//! fail in ways those tests cannot see: a panic in `rsx!`, a context that is not there, or
//! a hook called somewhere the rules of hooks forbid. A `VirtualDom` runs them with no
//! window, which is the only part of this that ever needed one.

mod dom;
mod events;
mod reference;
mod row_menu;
mod settle;
mod store;

/// The throwaway certificate authority S/MIME tests make their certificates with: the one
/// `mail-mime`'s tests and `tests/app/smime.rs` use, so all three build fixtures the same way
/// (`mail_crypto::testing::smime`).
pub(in crate::ui) use mail_crypto::testing::smime as smime_support;

pub(in crate::ui) use dom::{
    FakeKey, INSIDE_THE_SHELL, Seen, click, deliver, dispatching, drain, drain_seen, key, later,
    markup, press, reader_markup, rebuild_into, right_click, root_attr, thread_like,
};
pub(in crate::ui) use events::{FakePointer, PRIMARY, Scripts, chord, pointer, type_into};
pub(in crate::ui) use reference::{Work, work};
pub(in crate::ui) use row_menu::{
    listed_subjects, menu_names, open_row_menu, pick_named, pick_until, row_action, row_named,
    settle,
};
pub(in crate::ui) use settle::{
    settle_for, settle_quiet, settle_quiet_seen, settle_until, step_clock, step_clock_until,
};
pub(in crate::ui) use store::{
    acct_account, client, empty, gmail_caps, held_and_remote, identify, inbox_query, realistic,
    seeded,
};

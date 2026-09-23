//! The components, actually executed.
//!
//! Everything else about the shell is tested through `view.rs` and `reader.rs`, which are
//! free of Dioxus on purpose. That leaves the components themselves — and a component can
//! fail in ways those tests cannot see: a panic in `rsx!`, a context that is not there, or
//! a hook called somewhere the rules of hooks forbid. A `VirtualDom` runs them with no
//! window, which is the only part of this that ever needed one.

mod dom;
mod store;

pub(in crate::ui) use dom::{
    FakeKey, INSIDE_THE_SHELL, Seen, Typed, click, dispatching, harness, key, markup, press,
    reader_markup, rebuild_into, thread_like,
};
pub(in crate::ui) use store::{
    ACCOUNT, empty, gmail_caps, held_and_remote, inbox_query, realistic, seeded,
};

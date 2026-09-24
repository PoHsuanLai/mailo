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
mod store;

/// The throwaway certificate authority S/MIME tests make their certificates with: the one
/// `mail-mime`'s tests and `tests/smime.rs` use, so all three build fixtures the same way.
#[path = "../../../../mail-mime/tests/smime_support/mod.rs"]
pub(in crate::ui) mod smime_support;

pub(in crate::ui) use dom::{
    FakeKey, INSIDE_THE_SHELL, Seen, Typed, click, dispatching, drain, drain_seen, dump, framed,
    harness, in_scheme, key, markup, page, press, reader_markup, rebuild_into, root_attr,
    thread_like, write_page,
};
pub(in crate::ui) use events::{FakePointer, Scripts, chord, pointer, type_into};
pub(in crate::ui) use reference::{Work, work};
pub(in crate::ui) use store::{
    ACCOUNT, empty, gmail_caps, held_and_remote, inbox_query, realistic, seeded,
};

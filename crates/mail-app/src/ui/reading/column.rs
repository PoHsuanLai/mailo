//! The reader and what sits with it, as either window draws them.
//!
//! A reply sits under its thread; a new message or a forward is a page that stands in for the
//! reader. The first window's reader column and a conversation's own window differ only in what
//! `place` says and in whether a conversation can be missing.
//!
//! A function rather than a component: a scope of its own between the window and the reader
//! leaves the reader's `Move to…` menu open after Esc in the window's tests, one render pass
//! short, so the column is drawn in the caller's scope.

use super::{Reader, ReaderIn};
use crate::ui::compose::{self, ComposerPage, PageKind};
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::overlays::empty_state::{EmptyForm, EmptyState};
use mail_domain::ThreadId;

/// Draw the open conversation with the reply under it, or the composer's page, or, with nothing
/// open, the empty state.
pub(in crate::ui) fn reading_column(
    open: Option<ThreadId>,
    shell: Signal<Shell>,
    revision: Signal<u64>,
    place: ReaderIn,
) -> Element {
    let composing = compose::composing(&shell.read());
    match (composing, open) {
        (Some((draft, PageKind::Reply)), Some(thread)) => rsx! {
            Reader { thread, shell, revision, place,
                ComposerPage { key: "{draft}", draft, shell, revision }
            }
        },
        (Some((draft, _)), _) => rsx! { ComposerPage { key: "{draft}", draft, shell, revision } },
        (None, Some(thread)) => rsx! { Reader { thread, shell, revision, place } },
        (None, None) => rsx! {
            EmptyState { form: EmptyForm::Empty, title: crate::ui::copy::empty::NO_MESSAGE_SELECTED }
        },
    }
}

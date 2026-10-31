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
use dioxus::prelude::*;
use ds::components::overlays::empty_state::{EmptyForm, EmptyState};
use mail_domain::ThreadId;
use mail_ui_kit::view::Shell;

/// Draw the open conversation with the reply under it, or the composer's page, or, with nothing
/// open, the empty state.
pub(in crate::ui) fn reading_column(
    open: Option<ThreadId>,
    shell: Signal<Shell>,
    place: ReaderIn,
) -> Element {
    let composing = compose::composing(&shell.read());
    // The one message opened from a list of single messages, in the window's reader only: a
    // conversation's own window shows it whole.
    let only = match place {
        ReaderIn::Pane => shell.read().focus,
        ReaderIn::Window => None,
    };
    match (composing, open) {
        (Some((draft, PageKind::Reply)), Some(thread)) => rsx! {
            Reader { thread, shell, tools: true, place, only,
                ComposerPage { key: "{draft}", draft, shell }
            }
        },
        (Some((draft, _)), _) => rsx! { ComposerPage { key: "{draft}", draft, shell } },
        (None, Some(thread)) => rsx! { Reader { thread, shell, tools: true, place, only } },
        (None, None) => rsx! {
            EmptyState { form: EmptyForm::Empty, title: mail_ui_model::copy::empty::NO_MESSAGE_SELECTED }
        },
    }
}

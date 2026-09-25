//! Ctrl F: a small field in the reader head that finds in the open thread.
//!
//! What it finds is [`crate::search::find_highlight`]: plain words, or one `re:/…/`. Where it
//! finds it is `found.rs`, over the parsed blocks only. Enter and Shift+Enter move through the
//! matches and scroll the current one into view; Esc closes the field and its marks go with it.

use super::super::field::{Field, FieldKind};
use super::super::host::{Drawn, Host};
use crate::search::{Find, Step};
use crate::view::Shell;
use dioxus::prelude::*;
use ds::{Glyph, Icon, IconButton, IconButtonVariant};

/// The current match, which Enter and typing bring into the middle of the reader.
const CURRENT: &str = "mark.hit.now";

/// Ctrl F. With a thread open it opens the find field, or selects its text when it is already
/// open. With nothing open there is no thread to find in, so it goes to the list's search box.
pub(in crate::ui) fn open_find(mut shell: Signal<Shell>) {
    if shell.peek().open.is_none() {
        Host::focus(".search input");
        return;
    }
    if shell.peek().find.is_none() {
        shell.write().find = Some(Find::default());
    }
    Host::focus_and_select(Drawn::FindField);
}

/// The field, the count, and a close button. `total` is how many matches the thread has.
#[component]
pub(super) fn FindBar(shell: Signal<Shell>, total: usize, invalid: bool) -> Element {
    let Some(find) = shell.read().find.clone() else {
        return rsx! {};
    };
    let count = if invalid { None } else { find.count(total) };
    rsx! {
        div {
            class: "find",
            role: "search",
            onkeydown: move |event: Event<KeyboardData>| {
                let key = event.key().to_string();
                let modifiers = event.modifiers();
                let find_chord = key == "f" || key == "F";
                // Other chords are the window's: Ctrl T still opens the command menu from here.
                if modifiers.ctrl() && !find_chord {
                    return;
                }
                // Everything else stays in the field. A letter typed here is not a shortcut.
                event.stop_propagation();
                match key.as_str() {
                    "Enter" => {
                        let step = if modifiers.shift() { Step::Previous } else { Step::Next };
                        if let Some(find) = shell.write().find.as_mut() {
                            find.step(step, total);
                        }
                        Host::scroll_into_view(CURRENT);
                    }
                    "Escape" => {
                        shell.write().find = None;
                        Host::focus_app();
                    }
                    _ if find_chord => {
                        Host::focus_and_select(Drawn::FindField);
                    }
                    _ => {}
                }
            },
            Glyph { icon: Icon::Search, size: ds::IconSize::Compact }
            Field {
                kind: FieldKind::Inline,
                value: find.query.clone(),
                placeholder: "Find in this thread".to_owned(),
                extra: None,
                on_input: move |value: String| {
                    if let Some(find) = shell.write().find.as_mut() {
                        find.query = value;
                        find.current = 0;
                    }
                    Host::scroll_into_view(CURRENT);
                },
                on_focus: move |_| {},
                on_blur: move |_| {},
            }
            if let Some(count) = count {
                span { class: "find-count mono", aria_live: "polite", "{count}" }
            }
            IconButton {
                variant: IconButtonVariant::Tool,
                icon: Icon::X,
                label: close_label().to_owned(),
                onclick: move |_| {
                    shell.write().find = None;
                    Host::focus_app();
                },
            }
        }
    }
}

/// What the reader marks: an open find's words, or the list search's.
///
/// An invalid find pattern marks nothing and says why. The list's own invalid pattern is said
/// in the list bar, so here it only marks nothing.
pub(super) fn marking(shell: &Shell) -> (crate::search::Highlight, Option<String>) {
    match &shell.find {
        Some(find) => match crate::search::find_highlight(&find.query) {
            Ok(highlight) => (highlight, None),
            Err(why) => (crate::search::Highlight::default(), Some(why)),
        },
        None => {
            let label = crate::query::named(&shell.labels);
            let highlight = crate::search::list_highlight(&shell.search, &chrono::Local, &label);
            (highlight.unwrap_or_default(), None)
        }
    }
}

/// Computed rather than literal, so a test can find the button among the render's attributes.
fn close_label() -> &'static str {
    "Close find"
}

//! The undo toast: it springs up after an op, names it, and has a tab you can pull.
//!
//! Pulling the tab past [`ARMED`] and letting go undoes; so does a plain click. The pointer is
//! followed by the window's own `pointermove` (see [`super::drag::moved`]), because the tab
//! moves out from under it.

use super::{Follow, motion, undo_last};
use crate::view::Shell;
use dioxus::prelude::*;
use ds::{Glyph, Icon};
use mail_store::SqliteStore;
use std::sync::Arc;

/// How far the tab has to travel before letting go undoes, in pixels.
pub(super) const ARMED: f64 = 46.0;

/// A pull on the tab.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(in crate::ui) struct Pull {
    /// Where the pointer went down.
    pub from: f64,
    /// How far it has moved since, clamped to the tab's track.
    pub dx: f64,
    pub hand: Hand,
}

/// Whether the pointer is still holding the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Hand {
    Holding,
    /// Let go. Kept until the click that follows, so a pull that already undid is not also a
    /// click that undoes again.
    LetGo,
}

impl Pull {
    pub(super) fn armed(&self) -> bool {
        self.dx > ARMED
    }

    /// Whether the pointer travelled far enough that this was a pull and not a click.
    fn travelled(&self) -> bool {
        self.dx.abs() >= 3.0
    }
}

/// The tab's track: a little way back, a little further forward than it arms.
pub(super) fn clamp_pull(dx: f64) -> f64 {
    dx.clamp(-6.0, 78.0)
}

#[component]
pub(in crate::ui) fn Toast(shell: Signal<Shell>, revision: Signal<u64>) -> Element {
    let Some(state) = motion() else {
        return rsx! {};
    };
    let Some(said) = state.toast.read().clone() else {
        return rsx! {};
    };
    let pull = state.pull.read().filter(|pull| pull.hand == Hand::Holding);
    let tab = if pull.is_some_and(|pull| pull.armed()) {
        "tab armed"
    } else {
        "tab"
    };
    let moved = match pull {
        Some(pull) => format!("transform:translateX({:.0}px);transition:none", pull.dx),
        None => String::new(),
    };
    match said.follow {
        Follow::Undo => {}
        Follow::Nothing => {
            return rsx! {
                div { class: "toast plain", role: "status", span { "{said.text}" } }
            };
        }
        Follow::ArchiveFrom { sender, list } => {
            let label = "Archive all from this list";
            return rsx! {
                div { class: "toast", role: "status",
                    span { "{said.text}" }
                    button {
                        class: "tab go",
                        r#type: "button",
                        aria_label: "{label}",
                        onclick: move |event| {
                            event.stop_propagation();
                            let store = consume_context::<Arc<SqliteStore>>();
                            crate::ui::unsubscribe::archive_list(
                                &store, shell, revision, &sender, &list,
                            );
                        },
                        Glyph { icon: Icon::Archive }
                        "{label}"
                    }
                }
            };
        }
    }
    rsx! {
        div { class: "toast", role: "status",
            span { "{said.text}" }
            span { class: "hint", "pull →" }
            button {
                class: "{tab}",
                r#type: "button",
                aria_label: "Undo {said.text}",
                style: "{moved}",
                onpointerdown: move |event: Event<PointerData>| {
                    event.stop_propagation();
                    let mut pull = state.pull;
                    pull.set(Some(Pull {
                        from: event.client_coordinates().x,
                        dx: 0.0,
                        hand: Hand::Holding,
                    }));
                },
                onclick: move |event| {
                    event.stop_propagation();
                    let mut pull = state.pull;
                    let pulled = pull.peek().is_some_and(|pull| pull.travelled());
                    pull.set(None);
                    if !pulled {
                        let store = consume_context::<Arc<SqliteStore>>();
                        undo_last(&store, shell, revision);
                    }
                },
                Glyph { icon: Icon::Undo }
                "Undo"
            }
        }
    }
}

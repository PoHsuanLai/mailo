//! The Space editor: a quire `Sheet` hung from the window's top, opened from the Space's name,
//! "+" or the gear.
//!
//! Every change goes into the window's Spaces at once, so the frame's `Ds` root repaints with
//! it (cross-fading, as a switch does), Save writes `spaces.json`, and Esc puts the Space back
//! exactly as the sheet found it. The Space's own look is quire's `SpaceEditor`: its name, the
//! colour field and its stops, theme, the card's accent, the presets and the measured contrast.
//! What quire's editor does not draw stays mailo's, under it: which accounts the Space shows
//! (`members`), and Cancel and Save. Everything that applies to every Space (notifications,
//! spelling, the accounts themselves, contacts, rules, keys, the keyboard) is in Settings
//! (`settings_window`).
//!
//! The drag preview, decided (quire's migration brief §5.1, which left it open): a drag in the
//! colour field repaints the frame through `Ds`'s own cross-fade, each step like any other
//! change. mailo keeps no `Fade` of its own (the hand-rolled `Fade::None` went with
//! `paint_script`) and holds no `Surface` override during the drag, so `Ds` has one fade
//! behaviour, and the frame a drag shows is the frame Save keeps. Nothing here depends on the
//! renderer, so the decision stands on Blitz as it does on the webview.

mod members;
mod parts;
mod remove;

pub(in crate::ui) use parts::Seg;

use self::members::Members;
use super::common::{classed, in_card};
use super::frame::keep;
use super::press::{available, on_primary};
use crate::ui::space::Spaces;
use crate::ui::space::edit::Draft;
use crate::ui::today::Today;
use crate::ui::view::Shell;
use dioxus::prelude::*;
use ds::components::app::space_editor::{DotIndex, SpaceEditor as LookEditor, rows::MeasuredIn};
use ds::components::controls::button_model::Answers;
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;

/// Apply `edit` to the draft and put the result in the window's Spaces, which the frame's
/// root reads.
///
/// No write to disk: that is Save's. A change to a sheet that has closed does nothing. Every
/// change cross-fades the frame, a drag's steps included: quire's root has one fade, and
/// design/21-SPACES.md section 6 has the tint update live with it.
pub(super) fn change(
    mut editing: Signal<Option<Draft>>,
    mut spaces: Signal<Spaces>,
    edit: impl FnOnce(&mut Draft),
) {
    let (index, space) = {
        let mut guard = editing.write();
        let Some(draft) = guard.as_mut() else {
            return;
        };
        edit(draft);
        (draft.index, draft.space.clone())
    };
    if let Some(slot) = spaces.write().spaces.get_mut(index) {
        *slot = space;
    }
}

/// Esc: put the Space back as the sheet found it, and close.
pub(super) fn cancel(mut editing: Signal<Option<Draft>>, mut spaces: Signal<Spaces>) {
    let Some(draft) = editing.write().take() else {
        return;
    };
    let saved = draft.reverted();
    if let Some(slot) = spaces.write().spaces.get_mut(draft.index) {
        *slot = saved;
    }
    crate::ui::host::Host::focus_app();
}

/// Save: the draft is already on screen and in the Spaces; write them and close.
fn save(mut editing: Signal<Option<Draft>>, spaces: Signal<Spaces>) {
    editing.set(None);
    keep(&spaces.read());
    crate::ui::host::Host::focus_app();
}

/// What the Delete Space button says it does.
const DELETE_TITLE: &str = "Delete this Space, not your mail";

/// What the Save button says it does.
const SAVE_TITLE: &str = "Save this Space and close";

/// The sheet. Renders nothing while `editing` is `None`.
#[component]
pub(super) fn SpaceEditor(
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
    pages: Signal<u32>,
    today: Signal<Today>,
) -> Element {
    let scheme = use_scope().scheme;
    let Some(draft) = editing.read().clone() else {
        return rsx! {};
    };
    let space = draft.space.clone();
    rsx! {
        Sheet {
            label: "Edit this Space",
            attach: Attach::Window,
            common: in_card(),
            // Escape from inside the editor lands here and goes no further. While the Delete
            // Space sheet is over the editor, it closes that sheet alone: the editor keeps its
            // Space as it was and stays open.
            onclose: move |()| {
                if shell.read().removing_space.is_some() {
                    remove::close(shell);
                } else {
                    cancel(editing, spaces);
                }
            },
            // The look scrolls; the foot under it stays put.
            div { class: "ed-scroll",
                LookEditor {
                    common: classed("ed-look"),
                    look: space.look.clone(),
                    scheme,
                    active_dot: DotIndex(u8::try_from(draft.active).unwrap_or(0)),
                    name: Some(space.name.clone()),
                    onchange: move |look: SpaceLook| change(editing, spaces, |draft| draft.space.look = look),
                    on_active_dot: move |dot: DotIndex| change(editing, spaces, |draft| draft.active = usize::from(dot.0)),
                    on_rename: move |name: String| change(editing, spaces, |draft| draft.space.name = name),
                    measured: MeasuredIn::EachScheme,
                }
                Members { editing, spaces }
                p { class: "capnote",
                    "Notifications, spelling, accounts and the rest apply to every Space: they are in Settings (\u{2318},)."
                }
            }
            // The sheet's own foot, outside the scroller, so Save stays in reach wherever the
            // look is scrolled. Cancel answers Escape, Save Return.
            div { class: "ed-foot",
                Button {
                    label: "Delete Space\u{2026}",
                    title: DELETE_TITLE.to_owned(),
                    common: Common {
                        aria_label: Some("Delete Space\u{2026}".to_owned()),
                        ..Common::default()
                    },
                    availability: available(remove::offered(spaces.read().spaces.len())),
                    onclick: on_primary(move || remove::ask(shell, spaces, editing)),
                }
                Button {
                    label: "Cancel",
                    // The Delete Space sheet over the editor takes Escape; the editor under it
                    // must not answer it too.
                    answers: if shell.read().removing_space.is_some() {
                        Answers::Nothing
                    } else {
                        Answers::Escape
                    },
                    onclick: on_primary(move || cancel(editing, spaces)),
                }
                Button {
                    label: "Save",
                    answers: if shell.read().removing_space.is_some() {
                        Answers::Nothing
                    } else {
                        Answers::Return
                    },
                    title: SAVE_TITLE.to_owned(),
                    onclick: on_primary(move || save(editing, spaces)),
                }
            }
        }
        remove::RemoveSheet { shell, spaces, editing, pages, today }
    }
}

pub(in crate::ui) use remove::close as close_remove;

#[cfg(test)]
pub(in crate::ui) mod tests;

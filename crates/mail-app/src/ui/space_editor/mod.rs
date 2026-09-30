//! The Space editor: a quire `Sheet` hung from the window's top, opened from the Space's name,
//! "+" or the gear.
//!
//! Every change goes into the window's Spaces at once, so the frame's `Ds` root repaints with
//! it (cross-fading, as a switch does), Save writes `spaces.json`, and Esc puts the Space back
//! exactly as the sheet found it. The Space's own look is quire's `SpaceEditor`: its name, the
//! colour field and its stops, theme, the card's accent, the presets and the measured contrast.
//! What quire's editor does not draw stays mailo's, in form rows under it: provider marks,
//! notifications, the accounts, contacts, rules and keys, and Cancel and Save.
//!
//! The drag preview, decided (quire's migration brief §5.1, which left it open): a drag in the
//! colour field repaints the frame through `Ds`'s own cross-fade, each step like any other
//! change. mailo keeps no `Fade` of its own (the hand-rolled `Fade::None` went with
//! `paint_script`) and holds no `Surface` override during the drag, so `Ds` has one fade
//! behaviour, and the frame a drag shows is the frame Save keeps. Nothing here depends on the
//! renderer, so the decision stands on Blitz as it does on the webview.

mod notify;
mod parts;

use self::notify::Notifications;
use self::parts::{AccentChoice, Marks as MarksChoice};
use super::frame::keep;
use super::press::on_primary;
use crate::space::Spaces;
use crate::space::edit::Draft;
use crate::view::Shell;
use dioxus::prelude::*;
use ds::components::controls::button_model::Answers;
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::components::overlays::sheet_attach::Attach;
use ds::prelude::*;
use ds_shell::prelude::SpaceEditor as LookEditor;
use ds_shell::space_editor::{DotIndex, rows::MeasuredIn};

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

/// What the Save button says it does.
const SAVE_TITLE: &str = "Save this Space and close";

/// The sheet. Renders nothing while `editing` is `None`.
#[component]
pub(super) fn SpaceEditor(
    spaces: Signal<Spaces>,
    editing: Signal<Option<Draft>>,
    shell: Signal<Shell>,
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
            onclose: move |()| cancel(editing, spaces),
            // The look scrolls; the foot under it stays put.
            div { class: "ed-scroll",
                LookEditor {
                    look: space.look.clone(),
                    scheme,
                    active_dot: DotIndex(u8::try_from(draft.active).unwrap_or(0)),
                    name: Some(space.name.clone()),
                    onchange: move |look: SpaceLook| change(editing, spaces, |draft| draft.space.look = look),
                    on_active_dot: move |dot: DotIndex| change(editing, spaces, |draft| draft.active = usize::from(dot.0)),
                    on_rename: move |name: String| change(editing, spaces, |draft| draft.space.name = name),
                    measured: MeasuredIn::EachScheme,
                }
                FieldGroup { title: "Mail",
                    AccentChoice {}
                    MarksChoice { shell }
                    Notifications {}
                    FieldRow {
                        label: "Accounts",
                        help: "A new account joins this Space when the Space shows only some accounts.",
                        Button {
                            label: "Add Account\u{2026}",
                            onclick: on_primary(move || super::add_account::open(shell)),
                        }
                    }
                    FieldRow {
                        label: "Contacts",
                        help: "Who the composer suggests: import, export, rename, forget.",
                        Button {
                            label: "Contacts\u{2026}",
                            onclick: on_primary(move || super::contacts::open(shell)),
                        }
                    }
                    FieldRow {
                        label: "Rules",
                        help: "What new mail sorts into, the vacation reply, and the server's copy.",
                        Button {
                            label: "Rules\u{2026}",
                            onclick: on_primary(move || super::rules::open(shell)),
                        }
                    }
                    FieldRow {
                        label: "Keys and certificates",
                        help: "OpenPGP keys and S/MIME certificates, yours and your correspondents': make, import, export, trust, delete.",
                        Button {
                            label: "Keys and Certificates\u{2026}",
                            onclick: on_primary(move || super::pgp::keys::open(shell)),
                        }
                    }
                }
            }
            // The sheet's own foot, outside the scroller, so Save stays in reach wherever the
            // look is scrolled. Cancel answers Escape, Save Return.
            div { class: "ed-foot",
                Button {
                    label: "Cancel",
                    answers: Answers::Escape,
                    onclick: on_primary(move || cancel(editing, spaces)),
                }
                Button {
                    label: "Save",
                    answers: Answers::Return,
                    title: SAVE_TITLE.to_owned(),
                    onclick: on_primary(move || save(editing, spaces)),
                }
            }
        }
    }
}

#[cfg(test)]
mod notify_tests;
#[cfg(test)]
pub(in crate::ui) mod tests;

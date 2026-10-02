//! The keyboard shortcuts sheet: which key does what, and the user's own keys.
//!
//! Opened from the settings' Keyboard section and from Ctrl T "Keyboard shortcuts…". Change on
//! an action waits for the next key pressed, and gives it to the action or says, by name, which
//! action holds it already. Each action can be put back as it ships, and so can all of them.
//! Every change is kept at once in `keyboard.json`, like notifications and provider marks: what
//! a key means is the window's, not a Space's. What a key may be given is [`crate::ui::keymap`]'s;
//! this only draws the map and hands it the presses.
//!
//! While the sheet is open the keyboard is its own (`App`'s key handler sends every press here),
//! so a key pressed to be bound does not also archive the open conversation.

use super::press::{SheetClose, on_primary};
use crate::ui::appearance::WindowDirs;
use crate::ui::keymap::{self, DEFAULTS, Keymap, Refused};
use crate::ui::view::{KeyboardSheet as Showing, Shell, Shortcut};
use dioxus::prelude::*;
use ds::components::controls::key_equivalent::{KeyEquivalent, KeyStyle};
use ds::prelude::{Button, Icon, Shortcut as Caps, ShortcutKey as Key};
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;

/// What the sheet and its menu entry are called.
pub(in crate::ui) const TITLE: &str = "Keyboard shortcuts";

/// Open the sheet, waiting for nothing.
pub(in crate::ui) fn open(mut shell: Signal<Shell>) {
    shell.write().keyboard = Some(Showing::default());
}

/// Close the sheet and give the keyboard back to the window.
pub(in crate::ui) fn close(mut shell: Signal<Shell>) {
    shell.write().keyboard = None;
    crate::ui::host::Host::focus_app();
}

/// Change the open sheet, if it is still open.
fn edit(mut shell: Signal<Shell>, change: impl FnOnce(&mut Showing)) {
    if let Some(sheet) = shell.write().keyboard.as_mut() {
        change(sheet);
    }
}

/// Wait for `action`'s new key.
///
/// The window's own element takes focus from the pressed button, so the key comes to `App`'s
/// handler whatever the button does with a key of its own.
fn listen(shell: Signal<Shell>, action: Shortcut) {
    edit(shell, |sheet| {
        sheet.listening = Some(action);
        sheet.said = None;
    });
    crate::ui::host::Host::focus_app();
}

/// A key pressed while the sheet is open, named as `App` names a shortcut's key (Shift already
/// folded in). `chord` is whether Ctrl, Alt or Super was held.
///
/// Waiting for a key: Esc stops waiting, a modifier alone is passed over, and anything else is
/// offered to the action. Not waiting: Esc closes the sheet, and nothing else is anything.
pub(in crate::ui) fn pressed(shell: Signal<Shell>, key: &str, chord: bool) {
    let listening = shell
        .peek()
        .keyboard
        .as_ref()
        .and_then(|sheet| sheet.listening);
    let Some(action) = listening else {
        if key == "Escape" {
            close(shell);
        }
        return;
    };
    if key == "Escape" {
        edit(shell, |sheet| sheet.listening = None);
        return;
    }
    if keymap::is_not_a_key(key) {
        return;
    }
    let bound = if chord {
        Err(Refused::Chord)
    } else {
        keymap::bind(&shell.peek().keymap, action, key)
    };
    settle(shell, bound);
}

/// Keep `changed` in the window and on disk, or say why there is no change. Either way the sheet
/// stops waiting.
fn settle(mut shell: Signal<Shell>, changed: Result<Keymap, Refused>) {
    let said = match changed {
        Ok(map) => {
            let kept = match try_consume_context::<WindowDirs>() {
                Some(dirs) => keymap::save(&dirs.config, &map),
                // A window with no home directory keeps the keys for the session.
                None => Ok(()),
            };
            shell.write().keymap = map;
            kept.err()
                .map(|why| format!("Changed for now, but not kept: {why}"))
        }
        Err(refused) => Some(refused.to_string()),
    };
    edit(shell, |sheet| {
        sheet.listening = None;
        sheet.said = said;
    });
}

/// The caps quire draws for a key as the DOM names it: a capital letter is Shift and the letter.
fn caps(key: &str) -> Vec<Key> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_uppercase() => {
            vec![Key::Shift, Key::Char(c.to_lowercase().next().unwrap_or(c))]
        }
        (Some(c), None) => vec![Key::Char(c)],
        _ => match key {
            "ArrowDown" => vec![Key::Down],
            "ArrowUp" => vec![Key::Up],
            "ArrowLeft" => vec![Key::Left],
            "ArrowRight" => vec![Key::Right],
            "Delete" => vec![Key::Delete],
            "Backspace" => vec![Key::Backspace],
            "Home" => vec![Key::Home],
            "End" => vec![Key::End],
            "PageUp" => vec![Key::PageUp],
            "PageDown" => vec![Key::PageDown],
            "Insert" => vec![Key::Insert],
            _ => Vec::new(),
        },
    }
}

/// One key, as caps where quire has them and in words where it does not (F5).
#[component]
fn KeyCaps(named: String) -> Element {
    let drawn = caps(&named);
    rsx! {
        span { class: "kb-key", title: keymap::spoken(&named),
            if drawn.is_empty() {
                "{named}"
            } else {
                KeyEquivalent {
                    shortcut: Caps(drawn),
                    style: KeyStyle::Cap,
                    size: ControlSize::Small,
                }
            }
        }
    }
}

/// The sheet. Mounted while `shell.keyboard` is `Some`.
#[component]
pub(in crate::ui) fn KeyboardSheet(shell: Signal<Shell>) -> Element {
    let Some(sheet) = shell.read().keyboard.clone() else {
        return rsx! {};
    };
    let map = shell.read().keymap.clone();
    let any_changed = DEFAULTS.iter().any(|(action, _)| map.is_changed(*action));
    rsx! {
        div {
            class: "rules-wrap",
            onclick: move |_| close(shell),
            div {
                class: "rules",
                role: "dialog",
                aria_label: TITLE,
                onclick: move |event| event.stop_propagation(),
                div { class: "rules-head",
                    h3 { "{TITLE}" }
                    SheetClose { label: "Done", on_close: move |()| close(shell) }
                }
                div { class: "rules-main",
                    div { class: "rules-part",
                        p { class: "rules-faint",
                            "Press Change, then the key you want. Letters work while you read, never while you type."
                        }
                        ul { class: "rules-list kb-list",
                            li { class: "rules-row kb-row",
                                span { class: "rules-text", b { "{keymap::name(Shortcut::Back)}" } }
                                span { class: "kb-keys", KeyCaps { named: "Escape" } }
                                span { class: "rules-row-acts kb-acts",
                                    span { class: "rules-faint", "Always" }
                                }
                            }
                            for (action, _) in DEFAULTS.iter().copied() {
                                KeyRow {
                                    key: "{action:?}",
                                    shell,
                                    action,
                                    keys: map.keys(action),
                                    changed: map.is_changed(action),
                                    listening: sheet.listening == Some(action),
                                }
                            }
                        }
                    }
                }
                div { class: "rules-part",
                    div { class: "rules-acts",
                        if let Some(why) = sheet.said.clone() {
                            p { class: "capnote files-bad", role: "alert", "{why}" }
                        }
                        if any_changed {
                            Button {
                                size: ControlSize::Small,
                                label: "Reset all".to_owned(),
                                icon: Icon::Refresh,
                                common: Common {
                                    aria_label: Some("Reset every shortcut".to_owned()),
                                    ..Common::default()
                                },
                                onclick: on_primary(move || settle(shell, Ok(Keymap::default()))),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One action: its name, its keys, and Change and Reset.
#[component]
fn KeyRow(
    shell: Signal<Shell>,
    action: Shortcut,
    keys: Vec<String>,
    changed: bool,
    listening: bool,
) -> Element {
    let name = keymap::name(action);
    rsx! {
        li { class: "rules-row kb-row", "data-action": "{action:?}",
            span { class: "rules-text", b { "{name}" } }
            span { class: "kb-keys",
                if listening {
                    span { class: "kb-wait", "Press a key…" }
                } else {
                    for key in keys {
                        KeyCaps { key: "{key}", named: key.clone() }
                    }
                }
            }
            span { class: "rules-row-acts kb-acts",
                Button {
                    size: ControlSize::Small,
                    label: if listening { "Waiting".to_owned() } else { "Change".to_owned() },
                    common: Common {
                        aria_label: Some(format!("Change the key for {name}")),
                        ..Common::default()
                    },
                    onclick: on_primary(move || listen(shell, action)),
                }
                if changed {
                    Button {
                        size: ControlSize::Small,
                        label: "Reset".to_owned(),
                        common: Common {
                            aria_label: Some(format!("Reset {name}")),
                            ..Common::default()
                        },
                        onclick: on_primary(move || {
                            let back = keymap::reset(&shell.peek().keymap, action);
                            settle(shell, back);
                        }),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

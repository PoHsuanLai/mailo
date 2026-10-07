//! The Keyboard page of Settings: which key does what, and the user's own keys.
//!
//! Opened from Settings' sidebar and from ⌘K "Keyboard shortcuts…". Change on an action waits
//! for the next key pressed, and gives it to the action or says, under that action, which action
//! holds it already. Each action can be put back as it ships, and so can all of them. Every
//! change is kept at once in `keyboard.json`, like notifications and provider marks: what a key
//! means is the window's, not a Space's. What a key may be given is [`crate::ui::keymap`]'s; this
//! only draws the map and hands it the presses.
//!
//! While an action waits, the keyboard is the page's own (the Settings window's key handler sends
//! every press here), so a key pressed to be bound does nothing else.

use super::press::{available, on_primary};
use crate::ui::appearance::WindowDirs;
use crate::ui::keymap::{self, DEFAULTS, Keymap, Refused};
use crate::ui::view::{KeySaid, KeyboardPage as Showing, SettingsPage, Shell, Shortcut};
use dioxus::prelude::*;
use ds::components::content::label::LabelRole;
use ds::components::controls::key_equivalent::{KeyEquivalent, KeyStyle};
use ds::components::fields::field_row::{FieldGroup, FieldRow};
use ds::prelude::{Button, Label, Shortcut as Caps, ShortcutKey as Key, TextLine};
use ds::root::common::Common;
use ds::style::tokens::control_size::ControlSize;

/// Whether the window's keys all go to the page: it is shown, and an action waits for its key.
pub(in crate::ui) fn capturing(shell: &Shell) -> bool {
    shell.settings == Some(SettingsPage::Keyboard) && shell.keyboard.listening.is_some()
}

/// Change the page's state.
fn edit(mut shell: Signal<Shell>, change: impl FnOnce(&mut Showing)) {
    change(&mut shell.write().keyboard);
}

/// Wait for `action`'s new key.
///
/// The window's own element takes focus from the pressed button, so the key comes to the
/// window's handler whatever the button does with a key of its own.
fn listen(shell: Signal<Shell>, action: Shortcut) {
    edit(shell, |page| {
        page.listening = Some(action);
        page.said = None;
    });
    crate::ui::host::Host::focus_app();
}

/// A key pressed while an action waits, named as the window names a shortcut's key (Shift
/// already folded in). `chord` is whether Ctrl, Alt or Super was held.
///
/// Esc stops waiting, a modifier alone is passed over, and anything else is offered to the
/// action. Waiting for nothing, a key is nothing to the page.
pub(in crate::ui) fn pressed(shell: Signal<Shell>, key: &str, chord: bool) {
    let Some(action) = shell.peek().keyboard.listening else {
        return;
    };
    if key == "Escape" {
        edit(shell, |page| page.listening = None);
        return;
    }
    if keymap::is_not_a_key(key) {
        return;
    }
    // Bound on the keymap as it is on disk, not this window's copy, so a binding made in another
    // window meanwhile is kept.
    let current = try_consume_context::<WindowDirs>()
        .map(|dirs| keymap::load(&dirs.config))
        .unwrap_or_else(|| shell.peek().keymap.clone());
    let bound = if chord {
        Err(Refused::Chord)
    } else {
        keymap::bind(&current, action, key)
    };
    settle(shell, Some(action), bound);
}

/// Keep `changed` in the window and on disk, or say why there is no change, under `about`'s row
/// (Reset All's when `None`). Either way the page stops waiting.
fn settle(mut shell: Signal<Shell>, about: Option<Shortcut>, changed: Result<Keymap, Refused>) {
    let said = match changed {
        Ok(map) => {
            let kept = match try_consume_context::<WindowDirs>() {
                Some(dirs) => keymap::save(&dirs.config, &map),
                // A window with no home directory keeps the keys for the session.
                None => Ok(()),
            };
            if kept.is_ok() {
                crate::ui::revisions::told_configuration();
            }
            shell.write().keymap = map;
            kept.err()
                .map(|why| format!("Changed for now, but not kept: {why}"))
        }
        Err(refused) => Some(refused.to_string()),
    };
    edit(shell, |page| {
        page.listening = None;
        page.said = said.map(|text| KeySaid { about, text });
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
            "Escape" => vec![Key::Escape],
            "Enter" => vec![Key::Enter],
            "Tab" => vec![Key::Tab],
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

/// What the row of `about` says under its name: why the last change to it did not happen.
fn said_for(page: &Showing, about: Option<Shortcut>) -> Option<TextLine> {
    page.said
        .as_ref()
        .filter(|said| said.about == about)
        .map(|said| TextLine::from(said.text.clone()))
}

/// The page: Back, which is always Esc, each action with its keys, and Reset All.
#[component]
pub(in crate::ui) fn KeyboardPage(shell: Signal<Shell>) -> Element {
    let page = shell.read().keyboard.clone();
    let map = shell.read().keymap.clone();
    let any_changed = DEFAULTS.iter().any(|(action, _)| map.is_changed(*action));
    rsx! {
        FieldGroup { title: "Shortcuts",
            FieldRow {
                label: keymap::name(Shortcut::Back),
                help: Some(TextLine::from("Always")),
                KeyCaps { named: "Escape" }
            }
            for (action, _) in DEFAULTS.iter().copied() {
                KeyRow {
                    key: "{action:?}",
                    shell,
                    action,
                    keys: map.keys(action),
                    changed: map.is_changed(action),
                    listening: page.listening == Some(action),
                    said: said_for(&page, Some(action)),
                }
            }
        }
        FieldGroup {
            FieldRow {
                label: "Restore the shipped keys",
                help: Some(said_for(&page, None).unwrap_or_else(|| {
                    TextLine::from("Letters work while you read, never while you type.")
                })),
                Button {
                    label: "Reset All",
                    availability: available(any_changed),
                    common: Common {
                        aria_label: Some("Reset every shortcut".to_owned()),
                        ..Common::default()
                    },
                    onclick: on_primary(move || settle(shell, None, Ok(Keymap::default()))),
                }
            }
        }
    }
}

/// One action: its name, its keys, Change, and Reset when it is not as it ships.
#[component]
fn KeyRow(
    shell: Signal<Shell>,
    action: Shortcut,
    keys: Vec<String>,
    changed: bool,
    listening: bool,
    said: Option<TextLine>,
) -> Element {
    let name = keymap::name(action);
    rsx! {
        FieldRow {
            label: name,
            help: said,
            common: crate::ui::sidebar::tagged("action", format!("{action:?}")),
            if listening {
                Label { text: "Press a key…", role: LabelRole::Secondary }
            } else {
                span { class: "kb-keys",
                    for key in keys {
                        KeyCaps { key: "{key}", named: key.clone() }
                    }
                }
            }
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
                        settle(shell, Some(action), back);
                    }),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;

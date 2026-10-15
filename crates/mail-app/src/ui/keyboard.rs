//! The Keyboard page of Settings: which key does what, and the user's own keys.
//!
//! Opened from Settings' sidebar and from the search bar's "Keyboard Shortcuts Settings". Each
//! action's keys sit in quire's shortcut recorder, in its single-key mode: a click makes it
//! listen, the next key is offered to the action, and the row says, under the action, which
//! action holds it already. Each action can be put back as it ships, and so can all of them
//! (Restore Defaults, drawn only while something differs). Every
//! change is kept at once in `keyboard.json`, like notifications and provider marks: what a key
//! means is the window's, not a Space's. What a key may be given is [`crate::ui::keymap`]'s; this
//! only draws the map and hands it the presses.
//!
//! While a recorder listens it keeps every key from the window's actions, so a key pressed to be
//! bound does nothing else.

use super::press::{available, on_primary};
use crate::ui::actions::{chord_of_key, key_of_chord};
use crate::ui::appearance::WindowDirs;
use crate::ui::copy;
use crate::ui::keymap::{self, DEFAULTS, Keymap, Refused};
use crate::ui::view::{KeySaid, KeyboardPage as Showing, Shell, Shortcut};
use dioxus::prelude::*;
use ds::components::content::label::LabelRole;
use ds::components::controls::key_equivalent::{KeyEquivalent, KeyStyle};
use ds::components::fields::field_row::FieldRow;
use ds::prelude::{
    Button, Form, FormSection, Label, PageHeader, Shortcut as Caps, ShortcutField,
    ShortcutKey as Key, ShortcutKinds, ShortcutRecorded, TextLine, Tooltip, use_keys,
};
use ds::root::common::Common;

/// Change the page's state.
fn edit(mut shell: Signal<Shell>, change: impl FnOnce(&mut Showing)) {
    change(&mut shell.write().keyboard);
}

/// What `recorded` asks of `action`: a key is offered to it, Backspace puts back the key it
/// ships with (an action always has a key), and a cancel changes nothing.
fn recorded(shell: Signal<Shell>, action: Shortcut, recorded: ShortcutRecorded) {
    match recorded {
        ShortcutRecorded::Chord(chord) => {
            // Bound on the keymap as it is on disk, not this window's copy, so a binding made in
            // another window meanwhile is kept.
            let current = try_consume_context::<WindowDirs>()
                .map(|dirs| keymap::load(&dirs.config))
                .unwrap_or_else(|| shell.peek().keymap.clone());
            let bound = match key_of_chord(&chord) {
                Some(key) => keymap::bind(&current, action, &key),
                None => Err(Refused::Chord),
            };
            settle(shell, Some(action), bound);
        }
        ShortcutRecorded::Cleared => {
            let back = keymap::reset(&shell.peek().keymap, action);
            settle(shell, Some(action), back);
        }
        _ => edit(shell, |page| page.said = None),
    }
}

/// Keep `changed` in the window and on disk, or say why there is no change, under `about`'s row
/// (Reset All's when `None`).
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
        page.said = said.map(|text| KeySaid { about, text })
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
    let spoken = keymap::spoken(&named);
    rsx! {
        Tooltip { text: spoken.clone(),
            span { class: "kb-key", aria_label: spoken,
                if drawn.is_empty() {
                    "{named}"
                } else {
                    KeyEquivalent {
                        shortcut: Caps(drawn),
                        style: KeyStyle::Cap,
                    }
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

/// The page: Back, which is always Esc, each action with its keys, and Restore Defaults while
/// any differs from the shipped keys.
#[component]
pub(in crate::ui) fn KeyboardPage(shell: Signal<Shell>) -> Element {
    let page = shell.read().keyboard.clone();
    let map = shell.read().keymap.clone();
    let any_changed = DEFAULTS.iter().any(|(action, _)| map.is_changed(*action));
    // What the keymap could not use of the keys kept here (a key the system keeps for itself),
    // in its own words.
    let problems = use_keys().problems();
    rsx! {
        PageHeader { title: copy::titles::KEYBOARD }
        Form {
            FormSection {
                footer: Some(
                    "Letters work while you read, never while you type."
                        .to_owned(),
                ),
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
                        said: said_for(&page, Some(action)),
                    }
                }
            }
            if !problems.is_empty() {
                FormSection {
                    title: Some("Not used".to_owned()),
                    for problem in problems {
                        FieldRow {
                            key: "{problem}",
                            label: "Shortcut",
                            Label { text: problem.clone(), role: LabelRole::Secondary }
                        }
                    }
                }
            }
            // Shown only while a key differs from the shipped ones, or to say why the last
            // restore did not happen.
            if any_changed || said_for(&page, None).is_some() {
                FormSection {
                    FieldRow {
                        label: "",
                        help: said_for(&page, None),
                        Button {
                            label: copy::controls::RESTORE_DEFAULTS,
                            availability: available(any_changed),
                            onclick: on_primary(move || settle(shell, None, Ok(Keymap::default()))),
                        }
                    }
                }
            }
        }
    }
}

/// One action: its name, its keys in a recorder, and Reset when it is not as it ships.
#[component]
fn KeyRow(
    shell: Signal<Shell>,
    action: Shortcut,
    keys: Vec<String>,
    changed: bool,
    said: Option<TextLine>,
) -> Element {
    let name = keymap::name(action);
    let mut chords = keys.iter().filter_map(|key| chord_of_key(key));
    let value = chords.next();
    let more: Vec<_> = chords.collect();
    rsx! {
        FieldRow {
            label: name,
            help: said,
            common: crate::ui::sidebar::tagged("action", format!("{action:?}")),
            ShortcutField {
                label: format!("Change the key for {name}"),
                value,
                more,
                accepts: ShortcutKinds::SingleKeys,
                onrecord: move |got: ShortcutRecorded| recorded(shell, action, got),
            }
            if changed {
                Button {
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

//! Which key does what: the window's shortcuts as a table, and the user's changes to it.
//!
//! [`DEFAULTS`] is the keyboard as it ships. A [`Keymap`] is that table with the keys the user
//! chose in the settings' Keyboard section laid over it, one key per changed action, kept in
//! `keyboard.json` in the config directory beside `notify.json`. Every change goes through
//! [`bind`] or [`reset`], which refuse a key another action holds and name that action, so the
//! map can never give one key two meanings. A file is read through the same two functions, and
//! one that cannot be read, or that would give a key two meanings, is the defaults.
//!
//! Esc is not in the table and cannot be given away: it closes what is open even while typing,
//! the one key that has to work from inside a field. The composer's own keys are the editor's
//! (`editor/keys.rs`), not these.
//!
//! This table is what a person can change, and the defaults it holds are what mailo declares to
//! chordkit as its actions (`ui::actions`), which resolves every press. A key the person gave an
//! action is laid over that resolution by `actions::heard`: quire's `Keys` has no place for a
//! person's changes to an app's actions (its keymap comes from the system's source), so the
//! file is read here and its keys win in `actions`.

use crate::ui::view::Shortcut;
use std::path::Path;

/// Where the user's keys are kept, in the config directory.
pub const FILE_NAME: &str = "keyboard.json";

/// Every action a key can be given, in the order the settings list them, with the keys it has
/// until the user changes them. Keys are named as the DOM names them; a capital letter is the
/// letter with Shift.
pub const DEFAULTS: &[(Shortcut, &[&str])] = &[
    (Shortcut::Next, &["j", "ArrowDown"]),
    (Shortcut::Previous, &["k", "ArrowUp"]),
    (Shortcut::ExtendNext, &["J"]),
    (Shortcut::ExtendPrevious, &["K"]),
    (Shortcut::Archive, &["e"]),
    (Shortcut::Trash, &["#", "Delete"]),
    (Shortcut::Spam, &["!"]),
    (Shortcut::ToggleStar, &["s"]),
    (Shortcut::ToggleRead, &["u"]),
    (Shortcut::TogglePin, &["p"]),
    (Shortcut::ToggleMute, &["m"]),
    (Shortcut::Reply, &["r"]),
    (Shortcut::ReplyAll, &["a"]),
    (Shortcut::Forward, &["f"]),
    (Shortcut::Compose, &["c"]),
];

/// Keys the window keeps for itself, which no action can be given: Esc closes, Enter and Space
/// press what has focus or open a hovered conversation, and Tab moves focus.
const KEPT: &[&str] = &["Escape", "Enter", "Tab", " "];

/// Presses that are not a key of their own: a modifier alone, or a key the keyboard could not
/// name. Waiting for a key ignores them rather than refusing them.
const NOT_A_KEY: &[&str] = &[
    "Shift",
    "Control",
    "Alt",
    "AltGraph",
    "Meta",
    "Super",
    "Hyper",
    "OS",
    "CapsLock",
    "NumLock",
    "Fn",
    "Dead",
    "Unidentified",
    "Process",
];

/// Whether a press is only a modifier or an unnamed key, which a wait for a key passes over.
pub fn is_not_a_key(key: &str) -> bool {
    key.is_empty() || NOT_A_KEY.contains(&key)
}

/// The shortcuts in force: [`DEFAULTS`] with the user's keys over it.
///
/// Built only by [`bind`], [`reset`] and [`load`], so no two actions ever share a key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    /// The actions the user gave a key of their own, with that key, in the order they were set.
    changed: Vec<(Shortcut, String)>,
}

impl Keymap {
    /// The keys `action` answers to now.
    pub fn keys(&self, action: Shortcut) -> Vec<String> {
        match self.changed.iter().find(|(one, _)| *one == action) {
            Some((_, key)) => vec![key.clone()],
            None => defaults(action)
                .iter()
                .map(|key| (*key).to_owned())
                .collect(),
        }
    }

    /// Whether the user gave `action` a key of its own.
    pub fn is_changed(&self, action: Shortcut) -> bool {
        self.changed.iter().any(|(one, _)| *one == action)
    }

    /// The action holding `key`, if one does.
    pub fn holder(&self, key: &str) -> Option<Shortcut> {
        DEFAULTS
            .iter()
            .map(|(action, _)| *action)
            .find(|action| self.keys(*action).iter().any(|held| held == key))
    }

    /// An action other than `besides` holding `key`, if one does. Asked of a map being built,
    /// where `besides` may hold the key too.
    fn held_elsewhere(&self, key: &str, besides: Shortcut) -> Option<Shortcut> {
        DEFAULTS
            .iter()
            .map(|(action, _)| *action)
            .filter(|action| *action != besides)
            .find(|action| self.keys(*action).iter().any(|held| held == key))
    }

    /// The action the user gave `key` of their own, if one. The defaults are chordkit's to
    /// resolve; only a key the person chose is this map's to answer.
    pub fn changed_holder(&self, key: &str) -> Option<Shortcut> {
        self.changed
            .iter()
            .find(|(_, held)| held == key)
            .map(|(action, _)| *action)
    }

    /// The shortcut a key press means, or `None` for a key that is not one.
    ///
    /// `typing` is the whole of the safety here. A letter is a shortcut when the user is reading
    /// and a letter when they are writing, and a client that gets that wrong archives a
    /// conversation because someone typed "e" into a reply. Only `Escape` survives it — closing
    /// what you are typing in is the one thing you must be able to do from inside it.
    pub fn action(&self, key: &str, typing: bool) -> Option<Shortcut> {
        if key == "Escape" {
            return Some(Shortcut::Back);
        }
        if typing {
            return None;
        }
        self.holder(key)
    }
}

/// The keys `action` ships with. Empty for [`Shortcut::Back`], which is Esc's and not the table's.
pub fn defaults(action: Shortcut) -> &'static [&'static str] {
    DEFAULTS
        .iter()
        .find(|(one, _)| *one == action)
        .map_or(&[], |(_, keys)| keys)
}

/// Why a key was not given to an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Another action already has it.
    Taken { key: String, by: Shortcut },
    /// The window keeps it for itself.
    Kept(String),
    /// Held with Ctrl, Alt or Super: those are the window's own chords.
    Chord,
    /// Not an action that takes a key from the table.
    Fixed(Shortcut),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Taken { key, by } => {
                write!(f, "{} is already {}’s key.", spoken(key), name(*by))
            }
            Refused::Kept(key) => write!(f, "{} is the window’s own key.", spoken(key)),
            Refused::Chord => {
                f.write_str("Keys held with Ctrl, Alt or Super are the window’s own.")
            }
            Refused::Fixed(action) => write!(f, "{} is always Esc.", name(*action)),
        }
    }
}

/// `map` with `action` on `key` alone, or why not.
///
/// The action's other keys go: the new key is the one it answers to. Giving an action back the
/// one key it ships with is the same as resetting it.
pub fn bind(map: &Keymap, action: Shortcut, key: &str) -> Result<Keymap, Refused> {
    if defaults(action).is_empty() {
        return Err(Refused::Fixed(action));
    }
    if is_not_a_key(key) || KEPT.contains(&key) {
        return Err(Refused::Kept(key.to_owned()));
    }
    if let Some(by) = map.held_elsewhere(key, action) {
        return Err(Refused::Taken {
            key: key.to_owned(),
            by,
        });
    }
    let mut changed: Vec<(Shortcut, String)> = map
        .changed
        .iter()
        .filter(|(one, _)| *one != action)
        .cloned()
        .collect();
    if defaults(action) != [key] {
        changed.push((action, key.to_owned()));
    }
    Ok(Keymap { changed })
}

/// `map` with `action` back on the keys it ships with, or why not: another action the user
/// moved may be on one of them now.
pub fn reset(map: &Keymap, action: Shortcut) -> Result<Keymap, Refused> {
    let changed: Vec<(Shortcut, String)> = map
        .changed
        .iter()
        .filter(|(one, _)| *one != action)
        .cloned()
        .collect();
    let back = Keymap { changed };
    for key in defaults(action) {
        if let Some(by) = back.held_elsewhere(key, action) {
            return Err(Refused::Taken {
                key: (*key).to_owned(),
                by,
            });
        }
    }
    Ok(back)
}

/// What the settings call an action, and what a refusal names.
pub fn name(action: Shortcut) -> &'static str {
    match action {
        Shortcut::Next => "Next conversation",
        Shortcut::Previous => "Previous conversation",
        Shortcut::ExtendNext => "Select the next one too",
        Shortcut::ExtendPrevious => "Select the previous one too",
        Shortcut::Back => "Close",
        Shortcut::Archive => "Archive",
        Shortcut::Trash => "Move to Trash",
        Shortcut::Spam => "Mark as spam",
        Shortcut::ToggleStar => "Star or unstar",
        Shortcut::ToggleRead => "Mark read or unread",
        Shortcut::TogglePin => "Pin or unpin",
        Shortcut::ToggleMute => "Mute or unmute",
        Shortcut::Reply => "Reply",
        Shortcut::ReplyAll => "Reply all",
        Shortcut::Forward => "Forward",
        Shortcut::Compose => "Compose",
    }
}

/// A key as a person reads it: `Shift J` for "J", `J` for "j", `Down` for "ArrowDown".
pub fn spoken(key: &str) -> String {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(' '), None) => "Space".to_owned(),
        (Some(c), None) if c.is_uppercase() => format!("Shift {c}"),
        (Some(c), None) => c.to_uppercase().collect(),
        _ => key.strip_prefix("Arrow").unwrap_or(key).to_owned(),
    }
}

/// One changed action, as `keyboard.json` holds it.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Binding {
    action: Shortcut,
    key: String,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    #[serde(default)]
    bindings: Vec<Binding>,
}

/// The keys kept in `dir`, or the defaults when there are none or the file is damaged.
///
/// Damaged includes a file that parses but names an action twice, gives Esc's action a key,
/// gives an action a key the window keeps, or leaves one key with two meanings. Half a
/// hand-edited keymap is harder to reason about than none, so any of them throws the whole file
/// out.
pub fn load(dir: &Path) -> Keymap {
    let Ok(bytes) = std::fs::read(dir.join(FILE_NAME)) else {
        return Keymap::default();
    };
    serde_json::from_slice::<Stored>(&bytes)
        .ok()
        .and_then(|stored| parsed(stored.bindings))
        .unwrap_or_default()
}

/// The keymap `bindings` describe, if it is one [`bind`] could have built.
fn parsed(bindings: Vec<Binding>) -> Option<Keymap> {
    let mut changed: Vec<(Shortcut, String)> = Vec::new();
    for Binding { action, key } in bindings {
        let usable =
            !defaults(action).is_empty() && !is_not_a_key(&key) && !KEPT.contains(&key.as_str());
        if !usable || changed.iter().any(|(one, _)| *one == action) {
            return None;
        }
        changed.push((action, key));
    }
    let map = Keymap { changed };
    let mut held: Vec<String> = DEFAULTS
        .iter()
        .flat_map(|(action, _)| map.keys(*action))
        .collect();
    let all = held.len();
    held.sort();
    held.dedup();
    (held.len() == all).then_some(map)
}

/// Remember `map` in `dir`.
pub fn save(dir: &Path, map: &Keymap) -> Result<(), String> {
    let stored = Stored {
        bindings: map
            .changed
            .iter()
            .map(|(action, key)| Binding {
                action: *action,
                key: key.clone(),
            })
            .collect(),
    };
    mail_core::config::write_json(dir, FILE_NAME, &stored)
}

#[cfg(test)]
mod tests;

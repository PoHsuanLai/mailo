//! What a key press means in a mailo window, and the shortcuts mailo shows for it.
//!
//! mailo never reads which modifier is held. quire's [`Keys`] turns a press into chordkit's
//! `Action` in a `Context` (the window's keymap follows the system's own shortcut settings),
//! and this file is the three things mailo adds to that:
//!
//! - the actions mailo declares, with portable default chords written with `Primary` (⌘ on a Mac
//!   and on our desktop, Ctrl elsewhere): the single-key mail actions of [`DEFAULTS`]
//!   (`mail.next` is `j` and Down) and the chords of [`Own`] (`mail.search` is `Primary+K`).
//!   Standard actions are never declared: ⌘N is `StandardAction::New`, ⌘F `Find`, ⌘P `Print`,
//!   ⌘Z `Undo`, ⌘, `Settings`, ⌃⌘S `ToggleSidebar`, ⌘A `SelectAll`, ⌘B, ⌘I, ⌘U the editor's.
//!   ⌘1 to ⌘9 are quire's: the Spaces kit reads `SwitchChord::Primary` itself.
//! - [`heard`], which names what a press asks of mailo, with the keys the person chose in
//!   `keyboard.json` laid over the defaults (see [`crate::ui::keymap`] for why that file stays
//!   mailo's).
//! - the tips: [`tip`], [`tip_own`] and `Shortcut::standard`, which a control draws after its
//!   name through chordkit's display, so a rebinding shows in the tip.
//!
//! A bare key is only an action outside a text field: chordkit's `Context::TextEntry` keeps
//! typing keys for the field, which is what the old `typing` test was for.

use crate::ui::keymap::{DEFAULTS, Keymap};
use crate::ui::view::Shortcut;
use chordkit::{
    Action, AppAction, AppId, Chord, Context, DefaultChord, Key as ChordKey, Modifier,
    Modifiers as Mods, NamedKey, Platform, StandardAction,
};
use dioxus::prelude::{
    Key, KeyboardEvent, Modifiers, ModifiersInteraction, try_consume_context, use_hook,
};
use ds::base::command::{chord_of, resolve};
use ds::prelude::{Keys, Shortcut as Tip, ShortcutKey};

/// mailo's name to chordkit: the first part of every action id.
const APP: &str = "mail";

/// The id of each single-key action. Esc ([`Shortcut::Back`]) has none: it is not in the table
/// and cannot be given away.
const IDS: &[(Shortcut, &str)] = &[
    (Shortcut::Next, "mail.next"),
    (Shortcut::Previous, "mail.previous"),
    (Shortcut::ExtendNext, "mail.extend-next"),
    (Shortcut::ExtendPrevious, "mail.extend-previous"),
    (Shortcut::Archive, "mail.archive"),
    (Shortcut::Trash, "mail.trash"),
    (Shortcut::Spam, "mail.spam"),
    (Shortcut::ToggleStar, "mail.toggle-star"),
    (Shortcut::ToggleRead, "mail.toggle-read"),
    (Shortcut::TogglePin, "mail.toggle-pin"),
    (Shortcut::ToggleMute, "mail.toggle-mute"),
    (Shortcut::Reply, "mail.reply"),
    (Shortcut::ReplyAll, "mail.reply-all"),
    (Shortcut::Forward, "mail.forward"),
    (Shortcut::Compose, "mail.compose"),
];

/// Chords an action has besides its keys in [`DEFAULTS`]: Shift with an arrow extends the
/// selection as Shift with its letter does.
const ALSO: &[(Shortcut, &str)] = &[
    (Shortcut::ExtendNext, "Shift+Down"),
    (Shortcut::ExtendPrevious, "Shift+Up"),
];

/// The actions that are a chord and not a bare key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Own {
    /// ⌘K: the search bar. In the composer's body it is Link, as it has always been there.
    Search,
    /// ⌘⌫: trash the open conversation, while reading.
    TrashOpen,
    /// Strikethrough in the composer.
    Strikethrough,
    /// Inline code in the composer.
    Code,
    /// Send the message.
    Send,
    /// Focus mode in the composer.
    Focus,
    /// Shift+Enter: the open conversation in a window of its own.
    OpenInWindow,
}

/// Each [`Own`] action with its id and its default chords. `Primary` is ⌘ or Ctrl.
const OWN: &[(Own, &str, &[&str])] = &[
    (Own::Search, "mail.search", &["Primary+K"]),
    (
        Own::TrashOpen,
        "mail.trash-open",
        &["Primary+Backspace", "Primary+Delete"],
    ),
    (
        Own::Strikethrough,
        "mail.strikethrough",
        &["Primary+Shift+X"],
    ),
    (Own::Code, "mail.code", &["Primary+Shift+E"]),
    (Own::Send, "mail.send", &["Primary+Enter"]),
    (Own::Focus, "mail.focus", &["Primary+Shift+F"]),
    (Own::OpenInWindow, "mail.open-in-window", &["Shift+Enter"]),
];

/// What a key press asks of a mailo window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Heard {
    /// One of the mail actions: a key of [`DEFAULTS`], the person's own key for it, or Esc.
    Mail(Shortcut),
    /// A chord mailo declares.
    Own(Own),
    /// A standard action: the window answers the ones it offers and leaves the rest.
    Standard(StandardAction),
}

/// The id of a single-key action.
pub(in crate::ui) fn id_of(shortcut: Shortcut) -> Option<&'static str> {
    IDS.iter()
        .find(|(one, _)| *one == shortcut)
        .map(|(_, id)| *id)
}

/// The action a mailo id names, when it is a single-key one.
fn shortcut_of(action: &AppAction) -> Option<Shortcut> {
    IDS.iter()
        .find(|(_, id)| *id == action.id())
        .map(|(shortcut, _)| *shortcut)
}

/// The [`Own`] action a mailo id names.
fn own_of(action: &AppAction) -> Option<Own> {
    OWN.iter()
        .find(|(_, id, _)| *id == action.id())
        .map(|(own, _, _)| *own)
}

/// The chordkit action of a single-key action.
fn app_action(shortcut: Shortcut) -> Option<AppAction> {
    id_of(shortcut).and_then(|id| AppAction::new(id).ok())
}

/// The chordkit action of an [`Own`] action.
fn own_action(own: Own) -> Option<AppAction> {
    OWN.iter()
        .find(|(one, _, _)| *one == own)
        .and_then(|(_, id, _)| AppAction::new(id).ok())
}

/// `action`'s rows, one for each chord of `chords`.
fn rows(action: &str, chords: &[&str]) -> Vec<(AppAction, DefaultChord)> {
    let Ok(action) = AppAction::new(action) else {
        return Vec::new();
    };
    chords
        .iter()
        .filter_map(|chord| chord.parse::<DefaultChord>().ok())
        .map(|chord| (action.clone(), chord))
        .collect()
}

/// A key as the keymap writes it (`m`, `J` for Shift+J, ` `, `ArrowDown`, `F5`) as the chordkit
/// key and whether Shift is part of it. `None` for a key with no chordkit name.
fn key_of(key: &str) -> Option<(ChordKey, bool)> {
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(' '), None) => Some((ChordKey::Named(NamedKey::Space), false)),
        (Some(c), None) => Some((ChordKey::character(c), c.is_uppercase())),
        _ => {
            let named = match key {
                "Escape" => NamedKey::Escape,
                "Enter" => NamedKey::Enter,
                "Tab" => NamedKey::Tab,
                "Backspace" => NamedKey::Backspace,
                "Delete" => NamedKey::Delete,
                "Insert" => NamedKey::Insert,
                "Home" => NamedKey::Home,
                "End" => NamedKey::End,
                "PageUp" => NamedKey::PageUp,
                "PageDown" => NamedKey::PageDown,
                "ArrowUp" => NamedKey::Up,
                "ArrowDown" => NamedKey::Down,
                "ArrowLeft" => NamedKey::Left,
                "ArrowRight" => NamedKey::Right,
                _ => {
                    let number = key.strip_prefix('F')?.parse().ok()?;
                    return Some((ChordKey::function(number)?, false));
                }
            };
            Some((ChordKey::Named(named), false))
        }
    }
}

/// The chord a key of the keymap is.
fn chord_of_key(key: &str) -> Option<Chord> {
    let (base, shifted) = key_of(key)?;
    let held = if shifted {
        Mods::NONE.with(Modifier::Shift)
    } else {
        Mods::NONE
    };
    Some(Chord::new(held, base))
}

/// The chords a key of the keymap is declared as. A symbol (`#`, `!`) is typed with Shift on
/// some layouts and without it on others, and the key has always meant the character, so both
/// presses are the action's.
fn chords_of_key(key: &str) -> Vec<DefaultChord> {
    let Some(chord) = chord_of_key(key) else {
        return Vec::new();
    };
    let mut declared = vec![DefaultChord::from(chord)];
    let symbol = matches!(chord.key(), ChordKey::Char(c) if !c.is_alphanumeric());
    if symbol && chord.modifiers().is_empty() {
        let with_shift = Chord::new(Mods::NONE.with(Modifier::Shift), chord.key());
        declared.push(DefaultChord::from(with_shift));
    }
    declared
}

/// The single-key actions, as declared: every key of [`DEFAULTS`] and [`ALSO`].
fn singles() -> Vec<(AppAction, DefaultChord)> {
    let mut rows_of = Vec::new();
    for (shortcut, keys) in DEFAULTS {
        let Some(action) = app_action(*shortcut) else {
            continue;
        };
        for key in *keys {
            for chord in chords_of_key(key) {
                rows_of.push((action.clone(), chord));
            }
        }
    }
    for (shortcut, chord) in ALSO {
        if let (Some(action), Ok(chord)) = (app_action(*shortcut), chord.parse::<DefaultChord>()) {
            rows_of.push((action, chord));
        }
    }
    rows_of
}

/// Everything mailo declares, in groups registered one at a time. A registration is all or
/// nothing and is refused whole when one chord is taken (the system's own settings can give
/// ⌘K to something), so the single keys are one group and each chord of [`Own`] is its own:
/// a clash costs that action alone.
fn groups() -> Vec<Vec<(AppAction, DefaultChord)>> {
    std::iter::once(singles())
        .chain(OWN.iter().map(|(_, id, chords)| rows(id, chords)))
        .filter(|group| !group.is_empty())
        .collect()
}

/// Every action mailo declares, for a keymap that is not a window's (the tests').
#[cfg(test)]
pub(in crate::ui) fn declared() -> Vec<Vec<(AppAction, DefaultChord)>> {
    groups()
}

/// mailo's name as chordkit takes it.
pub(in crate::ui) fn app_id() -> Option<AppId> {
    AppId::new(APP).ok()
}

/// The window's keymap, made here because mailo's root sits above its `Ds`: the keymap is the
/// root context's `KeySource` (the system's own shortcut settings; the platform's conventions
/// without any), and quire's `Ds` adopts the one already provided. Call it first in the root of
/// every window that reads keys, before anything (`use_spaces`) that would take a default one.
pub(in crate::ui) fn use_window_keys() -> Keys {
    ds::keys::use_keys_provider()
}

/// [`use_window_keys`] with mailo's actions registered in it, once, at the first render. Call it
/// in the root of every window that answers keys.
pub(in crate::ui) fn use_registered() -> Keys {
    let keys = use_window_keys();
    use_hook(move || register(&keys));
    keys
}

/// Declare mailo's actions on `keys`, and say on the terminal which were refused and why.
fn register(keys: &Keys) {
    let Some(app) = app_id() else {
        return;
    };
    for group in groups() {
        if let Err(conflict) = keys.register_actions(&app, &group) {
            eprintln!("keys: {conflict}");
        }
    }
}

/// What a key press asks of the window, if anything: `typing` is whether a text field has the
/// keyboard, and `user` the keys the person chose.
pub(in crate::ui) fn heard(
    keys: Keys,
    user: &Keymap,
    event: &KeyboardEvent,
    typing: bool,
) -> Option<Heard> {
    heard_key(keys, user, &event.key(), event.modifiers(), typing)
}

/// [`heard`] for a key and modifiers from any source (the composer's surface gives its own).
pub(in crate::ui) fn heard_key(
    keys: Keys,
    user: &Keymap,
    key: &Key,
    modifiers: Modifiers,
    typing: bool,
) -> Option<Heard> {
    keys.with_keymap(|chords| heard_in(chords, user, key, modifiers, typing))
}

/// Whether the press holds nothing but Shift: a key of the keymap's table is such a press, and
/// a chord (Ctrl, Alt or the command key held) never is.
fn is_plain(platform: Platform, key: &Key, modifiers: Modifiers) -> bool {
    chord_of(platform, key, modifiers)
        .is_some_and(|chord| chord.modifiers().without(Modifier::Shift).is_empty())
}

/// Whether the press is a chord of the platform (something besides Shift held), for the
/// Keyboard page, which refuses one as a key.
pub(in crate::ui) fn is_chord(keys: Keys, key: &Key, modifiers: Modifiers) -> bool {
    !is_plain(keys.platform(), key, modifiers)
}

/// What `key` with `modifiers` asks of a window whose keymap is `chords`.
///
/// Esc is always [`Shortcut::Back`], even while typing: closing what you are typing in is the one
/// thing a field cannot own. A key the person gave an action wins over chordkit's defaults, and
/// takes that action's own defaults away (the action answers to one key then).
pub(in crate::ui) fn heard_in(
    chords: &chordkit::Keymap,
    user: &Keymap,
    key: &Key,
    modifiers: Modifiers,
    typing: bool,
) -> Option<Heard> {
    if *key == Key::Escape {
        return Some(Heard::Mail(Shortcut::Back));
    }
    if !typing && is_plain(chords.platform(), key, modifiers) {
        let name = key.to_string();
        let name = if modifiers.shift() {
            crate::ui::view::shifted(&name).to_owned()
        } else {
            name
        };
        if let Some(held) = user.changed_holder(&name) {
            return Some(Heard::Mail(held));
        }
    }
    let context = if typing {
        Context::TextEntry
    } else {
        Context::Normal
    };
    match resolve(chords, key, modifiers, context)? {
        Action::Standard(StandardAction::Cancel) => Some(Heard::Mail(Shortcut::Back)),
        Action::Standard(action) => Some(Heard::Standard(action)),
        Action::App(action) => match shortcut_of(&action) {
            Some(shortcut) => (!user.is_changed(shortcut)).then_some(Heard::Mail(shortcut)),
            None => own_of(&action).map(Heard::Own),
        },
        _ => None,
    }
}

/// The chord `shortcut` is pressed with now: the person's key if they chose one, else the first
/// of `live`, chordkit's chords for the action.
fn chord_for(live: &[Chord], user: &Keymap, shortcut: Shortcut) -> Option<Chord> {
    if user.is_changed(shortcut) {
        return user
            .keys(shortcut)
            .first()
            .and_then(|key| chord_of_key(key));
    }
    live.first().copied()
}

/// A chord as a tip draws it. Drawn by quire through chordkit's display, in the platform's own
/// order and words.
fn tip_of(chord: Chord) -> Option<Tip> {
    Tip::from_default_chord(DefaultChord::from(chord))
}

/// The tip of a single-key action: the chord the window resolves it from now, so a key the
/// person chose is the one shown. `None` for Esc's action, a window with no keymap, and an
/// action with no chord.
pub(in crate::ui) fn tip(user: &Keymap, shortcut: Shortcut) -> Option<Tip> {
    let keys = try_consume_context::<Keys>()?;
    let live = app_action(shortcut)
        .map(|action| keys.chords_of(&Action::App(action)))
        .unwrap_or_default();
    chord_for(&live, user, shortcut).and_then(tip_of)
}

/// The tip of one of mailo's chords, as the window's keymap has it now.
pub(in crate::ui) fn tip_own(own: Own) -> Option<Tip> {
    let keys = try_consume_context::<Keys>()?;
    let action = own_action(own)?;
    let live = keys.chords_of(&Action::App(action));
    live.first().copied().and_then(tip_of)
}

/// The tip of a standard action the window offers. quire draws the chord the keymap really
/// binds it to.
pub(in crate::ui) fn tip_standard(action: StandardAction) -> Tip {
    Tip::standard(action)
}

/// Esc, which closes: a key of the window and not an action of the keymap.
pub(in crate::ui) fn escape() -> Tip {
    Tip(vec![ShortcutKey::Escape])
}

#[cfg(test)]
mod tests;

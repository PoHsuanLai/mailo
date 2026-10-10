use super::{
    Heard, IDS, OWN, Own, app_action, app_id, declared, escape, groups, heard_in, hint_own,
    hint_standard, overrides_of, tip_of,
};
use crate::ui::keymap::{Keymap as User, bind, load, save};
use crate::ui::view::Shortcut;
use chordkit::{Action, Context, Desktop, Keymap as Chords, Overrides, Platform, StandardAction};
use dioxus::prelude::{Key, Modifiers};
use ds::base::command::shortcut_text;

fn ours() -> Platform {
    Platform::Linux {
        desktop: Desktop::Ours,
    }
}

fn every_platform() -> [Platform; 6] {
    [
        Platform::MacOs,
        Platform::Windows,
        ours(),
        Platform::Linux {
            desktop: Desktop::Kde,
        },
        Platform::Linux {
            desktop: Desktop::Gnome,
        },
        Platform::Linux {
            desktop: Desktop::Other,
        },
    ]
}

/// `platform`'s conventions with the keys the person chose laid over them and every action mailo
/// declares registered, as a window has them: quire applies the overrides to what the source
/// gives, then makes the registration.
fn registered_with(platform: Platform, user: &User) -> Chords {
    let (mut map, problems) = Chords::conventional(platform).with_overrides(&overrides_of(user));
    assert!(problems.is_empty(), "{platform:?}: {problems:?}");
    let app = app_id().expect("mail is an app name");
    map.register_with(&declared(&app))
        .unwrap_or_else(|why| panic!("{platform:?}: {why}"));
    map
}

/// [`registered_with`] for a person who changed nothing.
fn registered(platform: Platform) -> Chords {
    registered_with(platform, &User::default())
}

fn ids() -> Vec<&'static str> {
    IDS.iter()
        .map(|(_, id)| *id)
        .chain(OWN.iter().map(|(_, id, _)| *id))
        .collect()
}

#[test]
fn every_action_has_a_default_chord_and_no_two_collide() {
    // Every id the tables name is declared, with at least one chord, and named once.
    let mut named = ids();
    let all = named.len();
    named.sort_unstable();
    named.dedup();
    assert_eq!(named.len(), all, "an id is named twice");
    let rows: Vec<_> = groups().into_iter().flatten().collect();
    for id in &named {
        assert!(rows.iter().any(|(action, _)| action.id() == *id), "{id}");
    }
    // On every platform the keymap takes them all (it refuses a chord a standard action or
    // another action holds), and each default chord is the one its action is bound to.
    for platform in every_platform() {
        let map = registered(platform);
        for (action, chord) in &rows {
            let live = chord
                .resolve(platform, Context::Normal)
                .unwrap_or_else(|| panic!("{} has no chord on {platform:?}", action.id()));
            let holder = map
                .bindings(Context::Normal)
                .iter()
                .find(|(_, held)| *held == live)
                .map(|(holder, _)| holder.clone());
            assert_eq!(
                holder,
                Some(Action::App(action.clone())),
                "{platform:?}: {}",
                live.display(platform)
            );
        }
    }
}

fn character(text: &str) -> Key {
    Key::Character(text.to_owned())
}

#[test]
fn a_press_is_the_action_the_platform_s_chord_names() {
    const NONE: Modifiers = Modifiers::empty();
    const SHIFT: Modifiers = Modifiers::SHIFT;
    const SUPER: Modifiers = Modifiers::SUPER;
    const CTRL: Modifiers = Modifiers::CONTROL;
    use Heard::{Mail, Own as Chord, Standard};
    let windows = Platform::Windows;
    // (what, platform, key, held, typing, the action it is)
    type Case = (&'static str, Platform, Key, Modifiers, bool, Option<Heard>);
    let cases: Vec<Case> = vec![
        (
            "e",
            ours(),
            character("e"),
            NONE,
            false,
            Some(Mail(Shortcut::Archive)),
        ),
        ("e in a field", ours(), character("e"), NONE, true, None),
        (
            "j",
            ours(),
            character("j"),
            NONE,
            false,
            Some(Mail(Shortcut::Next)),
        ),
        (
            "Down",
            ours(),
            Key::ArrowDown,
            NONE,
            false,
            Some(Mail(Shortcut::Next)),
        ),
        ("Down in a field", ours(), Key::ArrowDown, NONE, true, None),
        (
            "Up",
            ours(),
            Key::ArrowUp,
            NONE,
            false,
            Some(Mail(Shortcut::Previous)),
        ),
        (
            "J",
            ours(),
            character("J"),
            SHIFT,
            false,
            Some(Mail(Shortcut::ExtendNext)),
        ),
        (
            "j beside a Shift",
            ours(),
            character("j"),
            SHIFT,
            false,
            Some(Mail(Shortcut::ExtendNext)),
        ),
        (
            "Shift Down",
            ours(),
            Key::ArrowDown,
            SHIFT,
            false,
            Some(Mail(Shortcut::ExtendNext)),
        ),
        (
            "Shift Up",
            ours(),
            Key::ArrowUp,
            SHIFT,
            false,
            Some(Mail(Shortcut::ExtendPrevious)),
        ),
        (
            "# with Shift",
            ours(),
            character("#"),
            SHIFT,
            false,
            Some(Mail(Shortcut::Trash)),
        ),
        (
            "# without it",
            ours(),
            character("#"),
            NONE,
            false,
            Some(Mail(Shortcut::Trash)),
        ),
        (
            "Delete",
            ours(),
            Key::Delete,
            NONE,
            false,
            Some(Mail(Shortcut::Trash)),
        ),
        (
            "!",
            ours(),
            character("!"),
            SHIFT,
            false,
            Some(Mail(Shortcut::Spam)),
        ),
        (
            "c",
            ours(),
            character("c"),
            NONE,
            false,
            Some(Mail(Shortcut::Compose)),
        ),
        (
            "Esc",
            ours(),
            Key::Escape,
            NONE,
            false,
            Some(Mail(Shortcut::Back)),
        ),
        (
            "Esc in a field",
            ours(),
            Key::Escape,
            NONE,
            true,
            Some(Mail(Shortcut::Back)),
        ),
        (
            "a letter nobody has",
            ours(),
            character("z"),
            NONE,
            false,
            None,
        ),
        (
            "Ctrl E is not Archive",
            ours(),
            character("e"),
            CTRL,
            false,
            None,
        ),
        (
            "Alt E is not Archive",
            ours(),
            character("e"),
            Modifiers::ALT,
            false,
            None,
        ),
        (
            "⌘N is New",
            ours(),
            character("n"),
            SUPER,
            true,
            Some(Standard(StandardAction::New)),
        ),
        (
            "⌘K",
            ours(),
            character("k"),
            SUPER,
            false,
            Some(Chord(Own::Search)),
        ),
        (
            "⌘K in a field",
            ours(),
            character("k"),
            SUPER,
            true,
            Some(Chord(Own::Search)),
        ),
        (
            "⌘⌫",
            ours(),
            Key::Backspace,
            SUPER,
            false,
            Some(Chord(Own::TrashOpen)),
        ),
        (
            "⌘Return",
            ours(),
            Key::Enter,
            SUPER,
            true,
            Some(Chord(Own::Send)),
        ),
        (
            "⇧⌘F",
            ours(),
            character("F"),
            SUPER | SHIFT,
            true,
            Some(Chord(Own::Focus)),
        ),
        (
            "⇧⌘S is strikethrough, not Save As",
            ours(),
            character("S"),
            SUPER | SHIFT,
            true,
            Some(Chord(Own::Strikethrough)),
        ),
        (
            "⌘E is code, not Use Selection for Find",
            ours(),
            character("e"),
            SUPER,
            true,
            Some(Chord(Own::Code)),
        ),
        (
            "Ctrl Shift S on Windows",
            windows,
            character("S"),
            CTRL | SHIFT,
            true,
            Some(Chord(Own::Strikethrough)),
        ),
        (
            "Ctrl E on Windows",
            windows,
            character("e"),
            CTRL,
            true,
            Some(Chord(Own::Code)),
        ),
        (
            "Shift Return",
            ours(),
            Key::Enter,
            SHIFT,
            false,
            Some(Chord(Own::OpenInWindow)),
        ),
        (
            "Shift Return in a field",
            ours(),
            Key::Enter,
            SHIFT,
            true,
            None,
        ),
        (
            "⌘Z",
            ours(),
            character("z"),
            SUPER,
            false,
            Some(Standard(StandardAction::Undo)),
        ),
        (
            "Ctrl Z is not undo here",
            ours(),
            character("z"),
            CTRL,
            false,
            None,
        ),
        (
            "⇧⌘Z",
            ours(),
            character("Z"),
            SUPER | SHIFT,
            false,
            Some(Standard(StandardAction::Redo)),
        ),
        (
            "⌘P",
            ours(),
            character("p"),
            SUPER,
            false,
            Some(Standard(StandardAction::Print)),
        ),
        (
            "⌘F",
            ours(),
            character("f"),
            SUPER,
            false,
            Some(Standard(StandardAction::Find)),
        ),
        (
            "⌘,",
            ours(),
            character(","),
            SUPER,
            false,
            Some(Standard(StandardAction::Settings)),
        ),
        (
            "⌃⌘S",
            ours(),
            character("s"),
            CTRL | SUPER,
            false,
            Some(Standard(StandardAction::ToggleSidebar)),
        ),
        (
            "⌘A",
            ours(),
            character("a"),
            SUPER,
            false,
            Some(Standard(StandardAction::SelectAll)),
        ),
        (
            "⌘B",
            ours(),
            character("b"),
            SUPER,
            true,
            Some(Standard(StandardAction::Bold)),
        ),
        (
            "⌘I",
            ours(),
            character("i"),
            SUPER,
            true,
            Some(Standard(StandardAction::Italic)),
        ),
        (
            "⌘U",
            ours(),
            character("u"),
            SUPER,
            true,
            Some(Standard(StandardAction::Underline)),
        ),
        (
            "Ctrl K on Windows",
            windows,
            character("k"),
            CTRL,
            false,
            Some(Chord(Own::Search)),
        ),
        (
            "Ctrl P on Windows",
            windows,
            character("p"),
            CTRL,
            false,
            Some(Standard(StandardAction::Print)),
        ),
        (
            "Ctrl Z on Windows",
            windows,
            character("z"),
            CTRL,
            false,
            Some(Standard(StandardAction::Undo)),
        ),
        (
            "Ctrl Y on Windows",
            windows,
            character("y"),
            CTRL,
            false,
            Some(Standard(StandardAction::Redo)),
        ),
        (
            "Super K on Windows",
            windows,
            character("k"),
            SUPER,
            false,
            None,
        ),
        (
            "e on Windows",
            windows,
            character("e"),
            NONE,
            false,
            Some(Mail(Shortcut::Archive)),
        ),
    ];
    for (what, platform, key, held, typing, want) in cases {
        let map = registered(platform);
        assert_eq!(
            heard_in(&map, &key, held, typing),
            want,
            "{what} ({platform:?})"
        );
    }
}

#[test]
fn the_table_is_the_keyboard_as_it_was_for_every_key() {
    // The legacy table, keyed by the DOM's name for a key, against the keymap's answer for the
    // press that key is: a shifted letter and a symbol with its Shift, the rest bare.
    let map = registered(ours());
    let user = User::default();
    for key in ('!'..='~').map(String::from) {
        let uppercase = key.chars().all(|c| c.is_uppercase());
        for typing in [false, true] {
            let held = if uppercase {
                Modifiers::SHIFT
            } else {
                Modifiers::empty()
            };
            let heard = heard_in(&map, &character(&key), held, typing);
            let table = user.action(&key, typing);
            // A symbol is also pressed with Shift (`#` is Shift+3 on a US keyboard).
            let shifted = heard_in(&map, &character(&key), Modifiers::SHIFT, typing);
            let asked = table.map(Heard::Mail);
            assert_eq!(heard, asked, "{key:?}, typing {typing}");
            if !key.chars().all(char::is_alphanumeric) {
                assert_eq!(shifted, asked, "{key:?} with Shift, typing {typing}");
            }
        }
    }
}

#[test]
fn keyboard_json_keys_take_effect_over_the_defaults() {
    let dir = tempfile::tempdir().unwrap();
    // Archive moves to x, and Star takes the key it left.
    let user = bind(&User::default(), Shortcut::Archive, "x").unwrap();
    let user = bind(&user, Shortcut::ToggleStar, "e").unwrap();
    save(dir.path(), &user).unwrap();
    let user = load(dir.path());
    // The file's keys are the keymap's own chords: the same press is the same action on every
    // platform, and nothing outside the keymap decides it.
    for platform in every_platform() {
        let map = registered_with(platform, &user);
        let said = |key: &str, typing| heard_in(&map, &character(key), Modifiers::empty(), typing);
        assert_eq!(
            said("x", false),
            Some(Heard::Mail(Shortcut::Archive)),
            "{platform:?}"
        );
        assert_eq!(said("x", true), None, "{platform:?}: not while typing");
        assert_eq!(
            said("e", false),
            Some(Heard::Mail(Shortcut::ToggleStar)),
            "{platform:?}: the key Archive left is Star's"
        );
        assert_eq!(
            said("s", false),
            None,
            "{platform:?}: Star's old key does nothing"
        );
        assert_eq!(
            said("j", false),
            Some(Heard::Mail(Shortcut::Next)),
            "{platform:?}"
        );
    }
    // And a key with the command key held is the platform's chord (Cut), not the person's key.
    let map = registered_with(ours(), &user);
    let held = heard_in(&map, &character("x"), Modifiers::SUPER, false);
    assert_eq!(held, Some(Heard::Standard(StandardAction::Cut)));
}

/// What the tip of `shortcut` draws on `map`'s platform.
fn tip_text(map: &Chords, shortcut: Shortcut) -> Option<String> {
    let chord = app_action(shortcut)
        .map(|action| map.chords_of(&Action::App(action)))
        .and_then(|live| live.first().copied())?;
    let tip = tip_of(chord)?;
    Some(shortcut_text(map, &tip))
}

#[test]
fn a_tip_shows_the_chord_the_resolver_would_use() {
    let map = registered(ours());
    assert_eq!(tip_text(&map, Shortcut::ToggleMute).as_deref(), Some("M"));
    assert_eq!(tip_text(&map, Shortcut::Trash).as_deref(), Some("#"));
    assert_eq!(
        tip_text(&map, Shortcut::ExtendNext).as_deref(),
        Some("\u{21e7}J")
    );
    assert_eq!(tip_text(&map, Shortcut::Back), None);

    // A key the person chose in keyboard.json is the one shown, as it is the one resolved.
    let moved = bind(&User::default(), Shortcut::ToggleMute, "x").unwrap();
    assert_eq!(
        tip_text(&registered_with(ours(), &moved), Shortcut::ToggleMute).as_deref(),
        Some("X")
    );
    let moved = bind(&User::default(), Shortcut::ExtendNext, "ArrowRight").unwrap();
    assert_eq!(
        tip_text(&registered_with(ours(), &moved), Shortcut::ExtendNext).as_deref(),
        Some("\u{2192}")
    );

    // So is a chord the keymap itself was rebound to, in the platform's own words.
    for (platform, shown) in [
        (ours(), "\u{21e7}\u{2318}M"),
        (Platform::Windows, "Ctrl+Shift+M"),
    ] {
        let (rebound, problems) = registered(platform)
            .with_overrides(&Overrides::parse("mail.toggle-mute = Primary+Shift+M").0);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            tip_text(&rebound, Shortcut::ToggleMute).as_deref(),
            Some(shown),
            "{platform:?}"
        );
        // The same keymap hears the new chord, and the old key does nothing.
        let held = if platform == Platform::Windows {
            Modifiers::CONTROL | Modifiers::SHIFT
        } else {
            Modifiers::SUPER | Modifiers::SHIFT
        };
        assert_eq!(
            heard_in(&rebound, &character("M"), held, false),
            Some(Heard::Mail(Shortcut::ToggleMute)),
            "{platform:?}"
        );
        assert_eq!(
            heard_in(&rebound, &character("m"), Modifiers::empty(), false),
            None,
            "{platform:?}"
        );
    }
}

#[test]
fn a_chord_of_mailo_s_is_drawn_in_the_platform_s_words() {
    for (platform, shown) in [
        (ours(), "\u{2318}K"),
        (Platform::MacOs, "\u{2318}K"),
        (Platform::Windows, "Ctrl+K"),
    ] {
        let map = registered(platform);
        let action =
            Action::App(chordkit::AppAction::new("mail.search").expect("a well-formed action id"));
        let tip = map.chords_of(&action).first().copied().and_then(tip_of);
        assert_eq!(
            tip.map(|tip| shortcut_text(&map, &tip)).as_deref(),
            Some(shown),
            "{platform:?}"
        );
    }
    assert_eq!(escape().glyphs(), "Esc");
}

#[test]
fn the_hints_beside_commands_follow_the_platform() {
    let cases = [
        (Platform::MacOs, "\u{2318}N"),
        (Platform::Windows, "Ctrl+N"),
    ];
    for (platform, new) in cases {
        let map = registered(platform);
        assert_eq!(
            hint_standard(&map, StandardAction::New).as_deref(),
            Some(new),
            "{platform:?}"
        );
        assert!(
            hint_own(&map, Own::OpenInWindow).is_some(),
            "{platform:?} has a key for a window of its own"
        );
    }
}

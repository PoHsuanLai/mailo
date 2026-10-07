//! A settings schema drawn as rows, the way the desktop's Settings app (detent) draws it: one
//! titled group per section, one row per key, and the control a key's kind gets (quire design/22
//! section 9.1). Nothing here knows a key by name, so it can move to quire's proposed shared
//! `SettingsPage` whole; the values are the caller's (`value_of`, `with_value`), and a change is
//! handed back as the new value.

use dioxus::prelude::*;
use ds::components::controls::segmented::Tracking;
use ds::components::fields::field_row::FieldRow;
use ds::components::menus::item::item::MenuItem;
use ds::components::menus::pop_up_button::PopUpButton;
use ds::prelude::*;
use ds::root::common::Common;
use ds_settings::schema::{KeyKind, KeySpec, Schema};

/// Words that are the off side of a pair (detent's `TogglePair` rule, design/22 section 9.1).
const OFF_WORDS: [&str; 5] = ["off", "hide", "never", "none", "nothing"];

/// Pairs that are a switch though neither word is off: (on, off).
const ON_OFF_PAIRS: [(&str, &str); 2] = [("natural", "traditional"), ("reduced", "standard")];

/// A two-word key that is a switch: which word is on and which off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pair {
    pub on: String,
    pub off: String,
}

impl Pair {
    /// The switch `words` make, or `None` when they are a choice between two things.
    pub(super) fn of(words: &[String; 2]) -> Option<Pair> {
        let [first, second] = words;
        let pair = |on: &String, off: &String| Pair {
            on: on.clone(),
            off: off.clone(),
        };
        let off = |word: &String| OFF_WORDS.contains(&word.as_str());
        match (off(first), off(second)) {
            (false, true) => return Some(pair(first, second)),
            (true, false) => return Some(pair(second, first)),
            (true, true) => return None,
            (false, false) => {}
        }
        ON_OFF_PAIRS
            .iter()
            .find_map(|(on, off)| match (first.as_str(), second.as_str()) {
                (a, b) if (a, b) == (*on, *off) => Some(pair(first, second)),
                (a, b) if (a, b) == (*off, *on) => Some(pair(second, first)),
                _ => None,
            })
    }
}

/// The control a key gets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Control {
    Switch(Pair),
    /// Two short choices, side by side.
    Segments(Vec<String>),
    /// Three or more choices, or a menu: a pop-up button showing the chosen one, as the Mac
    /// draws any choice whose segments would squeeze the row's label.
    PopUp(Vec<String>),
    /// A kind this sheet does not draw yet: the value, read-only.
    Shown,
}

/// What draws `kind`.
pub(super) fn control_of(kind: &KeyKind) -> Control {
    match kind {
        KeyKind::Toggle { variants } => Pair::of(variants)
            .map(Control::Switch)
            .unwrap_or_else(|| choice(variants.to_vec())),
        KeyKind::Segmented { variants } => choice(variants.clone()),
        KeyKind::Menu { variants } => Control::PopUp(variants.clone()),
        KeyKind::Fixed { .. }
        | KeyKind::Bounded { .. }
        | KeyKind::Text
        | KeyKind::Colour
        | KeyKind::Shortcut
        | KeyKind::List(_)
        | KeyKind::Rows { .. }
        | KeyKind::Live { .. } => Control::Shown,
    }
}

/// The control for a choice between `words`: segments for two, a pop-up for more.
fn choice(words: Vec<String>) -> Control {
    match words.len() {
        0..=2 => Control::Segments(words),
        _ => Control::PopUp(words),
    }
}

/// A stored word as a person reads it: the schema's label when it has one, else the word with
/// its first letter capitalised and underscores as spaces.
pub(super) fn word_label(key: &KeySpec, word: &str) -> String {
    if let Some(label) = key.labels.of(word) {
        return label.to_owned();
    }
    let spaced = word.replace('_', " ");
    let mut chars = spaced.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The value at `path` (`domain.key`) in `values`, else the key's default.
pub(super) fn value_of(values: &toml::Value, key: &KeySpec) -> toml::Value {
    key.path
        .0
        .split('.')
        .try_fold(values, |at, part| at.get(part))
        .cloned()
        .unwrap_or_else(|| key.default.clone())
}

/// `values` with `path` set to `value`, the tables on the way made where missing.
pub(in crate::ui) fn with_value(
    mut values: toml::Value,
    path: &str,
    value: toml::Value,
) -> toml::Value {
    let mut at = &mut values;
    let parts: Vec<&str> = path.split('.').collect();
    for (index, part) in parts.iter().enumerate() {
        let Some(table) = at.as_table_mut() else {
            return values;
        };
        if index + 1 == parts.len() {
            table.insert((*part).to_owned(), value);
            return values;
        }
        at = table
            .entry((*part).to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    }
    values
}

/// The sections of `schema`, in the order their first key comes, each with its keys.
pub(super) fn sections(schema: &Schema) -> Vec<(String, Vec<KeySpec>)> {
    let mut out: Vec<(String, Vec<KeySpec>)> = Vec::new();
    for key in &schema.key {
        let title = key.section.0.clone();
        match out.iter_mut().find(|(name, _)| *name == title) {
            Some((_, keys)) => keys.push(key.clone()),
            None => out.push((title, vec![key.clone()])),
        }
    }
    out
}

/// One section: its title over a row per key, and the note that explains the whole group, if the
/// caller has one, under it.
#[component]
pub(super) fn SchemaSection(
    title: String,
    keys: Vec<KeySpec>,
    values: toml::Value,
    onedit: EventHandler<(String, toml::Value)>,
    /// What the caller adds under the rows: a row that belongs to the section.
    #[props(default)]
    children: Element,
    /// A note about the group as a whole, drawn under it in the help type.
    #[props(default)]
    footer: Option<String>,
) -> Element {
    rsx! {
        FormSection { title: (!title.is_empty()).then(|| title.clone()), footer,
            for key in keys {
                KeyRow { key: "{key.path.0}", value: value_of(&values, &key), spec: key.clone(), onedit }
            }
            {children}
        }
    }
}

/// A key's row: its label and help, and its control.
#[component]
fn KeyRow(
    spec: KeySpec,
    value: toml::Value,
    onedit: EventHandler<(String, toml::Value)>,
) -> Element {
    let label = spec.label.0.clone();
    let help = (!spec.help.0.is_empty()).then(|| TextLine::from(spec.help.0.clone()));
    let path = spec.path.0.clone();
    let current = value.as_str().unwrap_or_default().to_owned();
    let control = match control_of(&spec.kind) {
        Control::Switch(pair) => {
            let on = current == pair.on;
            rsx! {
                Toggle {
                    label: label.clone(),
                    value: if on { Check::On } else { Check::Off },
                    onchange: move |next| {
                        let word = match next {
                            Check::On => pair.on.clone(),
                            Check::Off | Check::Mixed => pair.off.clone(),
                        };
                        onedit.call((path.clone(), toml::Value::String(word)));
                    },
                }
            }
        }
        Control::Segments(words) => {
            let choices = Choice::pairs(
                words
                    .iter()
                    .map(|word| (word.clone(), word_label(&spec, word))),
            );
            rsx! {
                SegmentedControl::<String> {
                    label: label.clone(),
                    choices,
                    tracking: Tracking::SelectOne(current.clone()),
                    onchange: move |word: String| onedit.call((path.clone(), toml::Value::String(word))),
                }
            }
        }
        Control::PopUp(words) => {
            let items = words
                .iter()
                .map(|word| MenuItem::new(word.clone(), word_label(&spec, word)))
                .collect::<Vec<_>>();
            rsx! {
                PopUpButton::<String> {
                    items,
                    value: Some(current.clone()),
                    common: Common { aria_label: Some(label.clone()), ..Common::default() },
                    onpick: move |word: String| onedit.call((path.clone(), toml::Value::String(word))),
                }
            }
        }
        Control::Shown => rsx! { Label { text: value.to_string() } },
    };
    rsx! {
        FieldRow { label: TextLine::from(label.clone()), help, {control} }
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;

//! Spaces: a tint, and whose mail they show.
//!
//! Stored in `spaces.json` under the config directory, beside appearance and
//! not in the mail database. A cosmetic choice must not be a write against
//! someone's mail.

use crate::ui::appearance::Legacy;
use ds::prelude::{SpaceLook, Theme, Word};
use ds::style::space::look::{CardAccent, Grain};
use ds::style::space::palette::{Dot, NEUTRAL_DOT};
use ds::style::space::presets::default_look;
use ds::style::tokens::person::PersonSwatch;
use mail_core::config::write_json;
use mail_domain::AccountId;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub mod edit;
mod migrate;
mod presets;
mod recall;

pub use presets::{PRESET_NAMES, PRESETS};
pub use recall::Recall;

const FILE_NAME: &str = "spaces.json";

/// Which accounts a Space shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Scope {
    /// Every account, in one list.
    #[default]
    All,
    /// These accounts and no others.
    Accounts(Vec<AccountId>),
}

/// A shortcut that stays in the sidebar.
///
/// A person, or a saved search. Neither expires; Today is the list that does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Pinned {
    /// A person, pinned by the address they write from.
    Person { name: String, email: String },
    /// A saved search, pinned by the query that runs it.
    Search { name: String, query: String },
}

/// One context: a name, how its frame looks, and whose mail it shows.
///
/// The look is quire's [`SpaceLook`]: one to three dots (empty input is the neutral dot; a
/// fourth is dropped), the palette this Space resolves to independently of the window, and
/// whether the card follows its hue. It is written flat into `spaces.json`, beside the fields
/// that are mailo's; it is read through [`SpaceRaw`], leniently, field by field. A file written
/// before quire's one Look may still hold a `motion` per Space: nobody reads it, and the next
/// write drops it. The `grain` key is read again; a file without one takes the preset's own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "SpaceRaw")]
pub struct Space {
    /// What the switcher calls it. "Space 1" until someone renames it.
    pub name: String,
    /// The frame's dots, theme and card accent: what the window's `Ds` root paints.
    #[serde(flatten)]
    pub look: SpaceLook,
    /// Whose mail this Space shows.
    pub scope: Scope,
    /// People and saved searches that stay put.
    pub pins: Vec<Pinned>,
    /// Each account's avatar colour, stored so three blue providers do not look alike.
    ///
    /// Missing entries take quire's person swatches (`ds::PersonSwatch`) in order. The colour is
    /// the account's, not the provider's.
    pub colors: BTreeMap<AccountId, String>,
}

/// The swatch an account at `index` takes when a Space has not chosen one, as stored.
fn swatch(index: usize) -> String {
    PersonSwatch::nth(index).hex().css()
}

/// The colour `id` wears in `space`, or the swatch at `index` when none was stored.
pub fn avatar_color(space: &Space, id: AccountId, index: usize) -> String {
    space
        .colors
        .get(&id)
        .cloned()
        .unwrap_or_else(|| swatch(index))
}

/// Fill any account that has no colour yet. Returns whether the Space changed.
pub fn ensure_colors(space: &mut Space, accounts: &[AccountId]) -> bool {
    let mut changed = false;
    for (index, id) in accounts.iter().enumerate() {
        if space.colors.contains_key(id) {
            continue;
        }
        space.colors.insert(*id, swatch(index));
        changed = true;
    }
    changed
}

impl Default for Space {
    fn default() -> Self {
        Self {
            name: String::new(),
            look: SpaceLook {
                dots: vec![NEUTRAL_DOT],
                grain: Grain::default(),
                theme: Theme::default(),
                card_accent: CardAccent::default(),
            },
            scope: Scope::default(),
            pins: Vec::new(),
            colors: BTreeMap::new(),
        }
    }
}

impl Spaces {
    /// The Space on screen, or a blank one when the file held none.
    pub fn current_space(&self) -> Space {
        self.spaces.get(self.current).cloned().unwrap_or_default()
    }
}

/// Every Space, and which one is on screen.
///
/// A missing file is an empty list, not a first run: [`first_run`] builds the
/// list once the accounts are known.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(from = "SpacesRaw")]
pub struct Spaces {
    pub spaces: Vec<Space>,
    /// Index into [`Self::spaces`]. Past the end, it becomes the last Space.
    pub current: usize,
    /// Where each Space was left, by index: its place, its open thread, its account tile.
    ///
    /// Beside `current` rather than inside a [`Space`] because it is where you are, not how
    /// the Space looks: the editor's Esc restores a Space exactly and must not also move you.
    /// A Space with no entry opens on the Inbox.
    pub recall: BTreeMap<usize, Recall>,
}

#[derive(Deserialize)]
struct SpaceRaw {
    #[serde(default)]
    name: String,
    #[serde(default)]
    dots: Vec<Dot>,
    /// Absent, or not a number: the preset's own grain for this Space's place in the list.
    #[serde(default, deserialize_with = "de_grain")]
    grain: Option<u8>,
    #[serde(default, deserialize_with = "de_theme")]
    theme: Theme,
    #[serde(default, deserialize_with = "de_card_accent")]
    card_accent: CardAccent,
    #[serde(default, deserialize_with = "de_scope")]
    scope: Scope,
    #[serde(default)]
    pins: Vec<Pinned>,
    #[serde(default)]
    colors: BTreeMap<AccountId, String>,
}

#[derive(Deserialize)]
struct SpacesRaw {
    #[serde(default)]
    spaces: Vec<SpaceRaw>,
    #[serde(default, deserialize_with = "de_index")]
    current: usize,
    #[serde(default, deserialize_with = "recall::de_recall")]
    recall: BTreeMap<usize, Recall>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
enum ScopeRaw {
    All,
    Accounts(Vec<AccountId>),
    #[serde(other)]
    Unknown,
}

impl From<SpaceRaw> for Space {
    fn from(raw: SpaceRaw) -> Self {
        Self::from_raw(raw, 0)
    }
}

impl Space {
    /// `raw` as the Space at `index`: a missing grain is that index's preset default.
    fn from_raw(raw: SpaceRaw, index: usize) -> Self {
        let mut dots: Vec<Dot> = raw.dots.into_iter().map(clamp_dot).collect();
        if dots.is_empty() {
            dots.push(NEUTRAL_DOT);
        } else if dots.len() > 3 {
            dots.truncate(3);
        }
        Self {
            name: raw.name,
            look: SpaceLook {
                dots,
                grain: raw.grain.map_or_else(|| Grain(preset_grain(index)), Grain),
                theme: raw.theme,
                card_accent: raw.card_accent,
            },
            scope: raw.scope,
            pins: raw.pins,
            colors: raw.colors,
        }
    }
}

impl From<SpacesRaw> for Spaces {
    fn from(raw: SpacesRaw) -> Self {
        let spaces: Vec<Space> = raw
            .spaces
            .into_iter()
            .enumerate()
            .map(|(index, space)| Space::from_raw(space, index))
            .collect();
        let current = match spaces.len() {
            0 => 0,
            count => raw.current.min(count - 1),
        };
        let recall = raw
            .recall
            .into_iter()
            .filter(|(index, _)| *index < spaces.len())
            .collect();
        Self {
            spaces,
            current,
            recall,
        }
    }
}

fn clamp_dot(dot: Dot) -> Dot {
    let hue = if dot.hue.is_finite() {
        dot.hue.rem_euclid(360.0)
    } else {
        NEUTRAL_DOT.hue
    };
    let chroma = if dot.chroma.is_finite() {
        dot.chroma.clamp(0.0, 1.0)
    } else {
        NEUTRAL_DOT.chroma
    };
    Dot { hue, chroma }
}

fn de_index<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let value = i64::deserialize(deserializer)?;
    Ok(usize::try_from(value).unwrap_or(0))
}

fn de_grain<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .map(|grain| u8::try_from(grain).unwrap_or(u8::MAX).min(100)))
}

/// The grain the preset at `index` paints with.
fn preset_grain(index: usize) -> u8 {
    default_look(index, Grain::default(), CardAccent::default())
        .grain
        .0
}

fn de_theme<'de, D>(deserializer: D) -> Result<Theme, D::Error>
where
    D: Deserializer<'de>,
{
    let word = String::deserialize(deserializer)?;
    Ok(Theme::parse(&word).unwrap_or_default())
}

fn de_card_accent<'de, D>(deserializer: D) -> Result<CardAccent, D::Error>
where
    D: Deserializer<'de>,
{
    let word = String::deserialize(deserializer)?;
    // `hint` is what mailo wrote before the look was quire's; `space_hue` is quire's word
    // for the same choice, and what is written now. `postmark`, the accent mailo once had, and
    // every other word are the chosen accent: Blue unless the person picked another.
    Ok(match word.as_str() {
        "hint" | "space_hue" => CardAccent::SpaceHue,
        _ => CardAccent::Chosen,
    })
}

fn de_scope<'de, D>(deserializer: D) -> Result<Scope, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match ScopeRaw::deserialize(deserializer)? {
        ScopeRaw::All | ScopeRaw::Unknown => Scope::All,
        ScopeRaw::Accounts(ids) => Scope::Accounts(ids),
    })
}

/// The stored Spaces, or an empty list when there is no file or it cannot be read.
///
/// Dots are kept to the first three, and `current` to a real
/// index. A missing dot list is the neutral dot. An unknown word for one field
/// is that field's default, not a failure of the whole file.
///
/// The theme moved from `appearance.json` into each Space. A Space written before that has
/// none, and takes the window-wide value stored beside it in the same directory, so the first
/// read after the upgrade looks the way the window did.
pub fn load(dir: &Path) -> Spaces {
    let Ok(bytes) = std::fs::read(dir.join(FILE_NAME)) else {
        return Spaces::default();
    };
    migrate::read_with(&bytes, &crate::ui::appearance::legacy(dir))
}

/// Give every Space `look`'s theme. For Spaces made before any were stored.
pub fn inherit(spaces: &mut Spaces, look: &Legacy) {
    for space in &mut spaces.spaces {
        space.look.theme = look.theme;
    }
}

/// A Space for "+": named after its position, tinted from the next preset in turn.
///
/// It covers every account, and keeps the current Space's theme so making one does not also
/// change how the window looks.
pub fn new_space(spaces: &Spaces) -> Space {
    let count = spaces.spaces.len();
    let current = spaces.current_space();
    let mut made = Space {
        name: format!("Space {}", count + 1),
        scope: Scope::All,
        ..Space::default()
    };
    made.look.dots = PRESETS[count % PRESETS.len()].to_vec();
    made.look.grain = Grain(preset_grain(count));
    made.look.theme = current.look.theme;
    made
}

/// Write `spaces` to `dir/spaces.json`, creating `dir` if needed.
pub fn save(dir: &Path, spaces: &Spaces) -> Result<(), String> {
    write_json(dir, FILE_NAME, spaces)
}

/// The first-run look with the dots of preset `index`, in turn.
fn preset_look(index: usize) -> SpaceLook {
    default_look(index, Grain::default(), CardAccent::default())
}

/// One Space per account, named "Space 1" onward, tinted from [`PRESETS`] in order.
///
/// No accounts still yields one Space, covering everything, so the window has
/// somewhere to put the mail. Nothing is pinned.
pub fn first_run(accounts: &[AccountId]) -> Spaces {
    if accounts.is_empty() {
        return Spaces {
            spaces: vec![Space {
                name: "Space 1".to_owned(),
                look: preset_look(0),
                scope: Scope::All,
                ..Space::default()
            }],
            current: 0,
            recall: BTreeMap::new(),
        };
    }
    let spaces = accounts
        .iter()
        .enumerate()
        .map(|(index, id)| Space {
            name: format!("Space {}", index + 1),
            look: preset_look(index),
            scope: Scope::Accounts(vec![*id]),
            ..Space::default()
        })
        .collect();
    Spaces {
        spaces,
        current: 0,
        recall: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests;

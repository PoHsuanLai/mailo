//! Spaces: a tint, a grain, and whose mail they show.
//!
//! Stored in `spaces.json` under the config directory, beside appearance and
//! not in the mail database. A cosmetic choice must not be a write against
//! someone's mail.

use crate::appearance::{read_json, write_json};
use crate::palette::{Dot, NEUTRAL_DOT};
use crate::view::Theme;
use mail_domain::AccountId;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const FILE_NAME: &str = "spaces.json";
const DEFAULT_GRAIN: u8 = 35;

/// Whether the card's accent follows the Space or stays Postmark.
///
/// `Hint` borrows the Space's hue at a fraction of a free accent's chroma.
/// `Postmark` keeps the ink-blue the rest of the window uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum CardAccent {
    /// The card borrows the Space's hue.
    #[default]
    Hint,
    /// The card keeps Postmark, whatever hue the Space is.
    Postmark,
}

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

/// One context: a name, up to three dots, and whose mail it shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "SpaceRaw")]
pub struct Space {
    /// What the switcher calls it. "Space 1" until someone renames it.
    pub name: String,
    /// One to three dots. Empty input is the neutral dot; a fourth is dropped.
    pub dots: Vec<Dot>,
    /// How loud the grain is, from 0 to 100. The first-run value is 35.
    pub grain: u8,
    /// Which palette this Space resolves to, independently of the window.
    pub theme: Theme,
    /// Whether the card follows this Space's hue.
    pub card_accent: CardAccent,
    /// Whose mail this Space shows.
    pub scope: Scope,
    /// People and saved searches that stay put.
    pub pins: Vec<Pinned>,
    /// Each account's avatar colour, stored so three blue providers do not look alike.
    ///
    /// Missing entries take [`AVATAR`] in order. The colour is the account's, not the provider's.
    pub colors: BTreeMap<AccountId, String>,
}

/// Avatar colours, taken in order when a Space has not chosen one for an account.
pub const AVATAR: &[&str] = &[
    "#5B4FC4", "#2F7F6E", "#B0662E", "#3C8A5B", "#7A4A9E", "#C0782E", "#2E7F8C", "#6D7A3A",
];

/// The colour `id` wears in `space`, or the preset at `index` when none was stored.
pub fn avatar_color(space: &Space, id: AccountId, index: usize) -> String {
    space
        .colors
        .get(&id)
        .cloned()
        .unwrap_or_else(|| AVATAR[index % AVATAR.len()].to_owned())
}

/// Fill any account that has no colour yet. Returns whether the Space changed.
pub fn ensure_colors(space: &mut Space, accounts: &[AccountId]) -> bool {
    let mut changed = false;
    for (index, id) in accounts.iter().enumerate() {
        if space.colors.contains_key(id) {
            continue;
        }
        space
            .colors
            .insert(*id, AVATAR[index % AVATAR.len()].to_owned());
        changed = true;
    }
    changed
}

impl Default for Space {
    fn default() -> Self {
        Self {
            name: String::new(),
            dots: vec![NEUTRAL_DOT],
            grain: DEFAULT_GRAIN,
            theme: Theme::default(),
            card_accent: CardAccent::default(),
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
}

#[derive(Deserialize)]
struct SpaceRaw {
    #[serde(default)]
    name: String,
    #[serde(default)]
    dots: Vec<Dot>,
    #[serde(default = "default_grain", deserialize_with = "de_grain")]
    grain: u8,
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
    spaces: Vec<Space>,
    #[serde(default, deserialize_with = "de_index")]
    current: usize,
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
enum ScopeRaw {
    All,
    Accounts(Vec<AccountId>),
    #[serde(other)]
    Unknown,
}

fn default_grain() -> u8 {
    DEFAULT_GRAIN
}

impl From<SpaceRaw> for Space {
    fn from(raw: SpaceRaw) -> Self {
        let mut dots: Vec<Dot> = raw.dots.into_iter().map(clamp_dot).collect();
        if dots.is_empty() {
            dots.push(NEUTRAL_DOT);
        } else if dots.len() > 3 {
            dots.truncate(3);
        }
        Self {
            name: raw.name,
            dots,
            grain: raw.grain.min(100),
            theme: raw.theme,
            card_accent: raw.card_accent,
            scope: raw.scope,
            pins: raw.pins,
            colors: raw.colors,
        }
    }
}

impl From<SpacesRaw> for Spaces {
    fn from(raw: SpacesRaw) -> Self {
        let spaces = raw.spaces;
        let current = match spaces.len() {
            0 => 0,
            count => raw.current.min(count - 1),
        };
        Self { spaces, current }
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

fn de_grain<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = i64::deserialize(deserializer)?;
    Ok(match u8::try_from(value) {
        Ok(grain) if grain <= 100 => grain,
        _ if value <= 0 => 0,
        _ => 100,
    })
}

fn de_index<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    let value = i64::deserialize(deserializer)?;
    Ok(usize::try_from(value).unwrap_or(0))
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
    Ok(match word.as_str() {
        "hint" => CardAccent::Hint,
        "postmark" => CardAccent::Postmark,
        _ => CardAccent::default(),
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
/// Dots are kept to the first three, grain to 0..=100, and `current` to a real
/// index. A missing dot list is the neutral dot. An unknown word for one field
/// is that field's default, not a failure of the whole file.
pub fn load(dir: &Path) -> Spaces {
    read_json(dir, FILE_NAME)
}

/// Write `spaces` to `dir/spaces.json`, creating `dir` if needed.
pub fn save(dir: &Path, spaces: &Spaces) -> Result<(), String> {
    write_json(dir, FILE_NAME, spaces)
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
                dots: PRESETS[0].to_vec(),
                scope: Scope::All,
                ..Space::default()
            }],
            current: 0,
        };
    }
    let spaces = accounts
        .iter()
        .enumerate()
        .map(|(index, id)| Space {
            name: format!("Space {}", index + 1),
            dots: PRESETS[index % PRESETS.len()].to_vec(),
            scope: Scope::Accounts(vec![*id]),
            ..Space::default()
        })
        .collect();
    Spaces { spaces, current: 0 }
}

/// The eight gradients from the mockup, then the six Part A accent hues as single dots.
///
/// The accent dots sit at chroma 0.7, except graphite, whose own colour is almost
/// neutral and is stored at 0.08. Each hue is the OKLCH hue of that Part A swatch.
pub const PRESETS: &[&[Dot]] = &[
    &[
        Dot {
            hue: 268.0,
            chroma: 0.72,
        },
        Dot {
            hue: 318.0,
            chroma: 0.55,
        },
    ],
    &[
        Dot {
            hue: 152.0,
            chroma: 0.62,
        },
        Dot {
            hue: 62.0,
            chroma: 0.55,
        },
        Dot {
            hue: 28.0,
            chroma: 0.5,
        },
    ],
    &[Dot {
        hue: 220.0,
        chroma: 0.7,
    }],
    &[
        Dot {
            hue: 20.0,
            chroma: 0.66,
        },
        Dot {
            hue: 55.0,
            chroma: 0.6,
        },
    ],
    &[
        Dot {
            hue: 190.0,
            chroma: 0.6,
        },
        Dot {
            hue: 240.0,
            chroma: 0.55,
        },
    ],
    &[
        Dot {
            hue: 340.0,
            chroma: 0.6,
        },
        Dot {
            hue: 290.0,
            chroma: 0.5,
        },
    ],
    &[Dot {
        hue: 95.0,
        chroma: 0.5,
    }],
    &[Dot {
        hue: 250.0,
        chroma: 0.06,
    }],
    &[Dot {
        hue: 257.437_8,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 137.85431,
        chroma: 0.08,
    }],
    &[Dot {
        hue: 164.06635,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 276.64212,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 22.80671,
        chroma: 0.7,
    }],
    &[Dot {
        hue: 32.172_4,
        chroma: 0.7,
    }],
];

#[cfg(test)]
mod tests;

//! Spaces: a tint, and whose mail they show.
//!
//! The Spaces are quire's kit (`ds::components::app::spaces`): the list, its ids, switching,
//! the look, the menu and Today are the same in every app. What a Space holds that is mail's is
//! its [`Mail`] payload: whose mail it shows, what stays pinned, and each account's colour. Where
//! a Space was left is mailo's [`Recall`].
//!
//! Stored in `spaces.json` under the config directory, beside appearance and not in the mail
//! database: a cosmetic choice must not be a write against someone's mail. quire reads the file
//! leniently, field by field; [`Mail`] reads its own fields the same way.

use crate::ui::appearance::{Legacy, WindowDirs};
use ds::components::app::spaces;
use ds::prelude::SpaceLook;
use ds::style::tokens::person::PersonSwatch;
use ds_settings::{Fixup, SpacesStorage};
use mail_domain::Filter;
use porter_core::AccountId;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod forget;
mod member;
mod recall;

pub use ds::components::app::spaces::SpaceId;
pub use forget::{Forgot, forget_account, forget_unknown};
pub use member::{Member, with_member};
pub use recall::Recall;

/// One context: quire's Space over mail's payload.
pub type Space = spaces::Space<Mail>;

/// Every Space, which one is on screen, and where each was left.
pub type Spaces = spaces::Spaces<Mail, Recall>;

/// The window's Spaces as quire's controller drives them: switching, the menu, and writing.
pub type Handle = spaces::SpacesHandle<Mail, Recall>;

/// Which accounts a Space shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Scope {
    /// Every account, in one list.
    #[default]
    All,
    /// These accounts and no others. None at all is a Space that shows nothing, which is what
    /// a Space whose accounts were all removed is until one is added to it.
    Accounts(Vec<AccountId>),
}

impl Scope {
    /// Whether `account`'s mail is in it.
    pub fn shows(&self, account: AccountId) -> bool {
        match self {
            Scope::All => true,
            Scope::Accounts(ids) => ids.contains(&account),
        }
    }

    /// Only `pressed`, when an account tile is pressed; else this.
    pub fn narrowed(&self, pressed: Option<AccountId>) -> Scope {
        match pressed {
            Some(id) => Scope::Accounts(vec![id]),
            None => self.clone(),
        }
    }

    /// As a store filter. `None` is every account, and no accounts is [`Filter::Nothing`].
    pub fn filter(&self) -> Option<Filter> {
        match self {
            Scope::All => None,
            Scope::Accounts(ids) => Some(match ids.as_slice() {
                [] => Filter::Nothing,
                [one] => Filter::Account(one.clone()),
                many => Filter::Or(many.iter().cloned().map(Filter::Account).collect()),
            }),
        }
    }
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

/// What a Space holds that is mail's. Written flat beside the Space's own fields in
/// `spaces.json`, as mailo always wrote them; a field that is missing or unreadable is its
/// default, never a failure of the whole file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Mail {
    /// Whose mail this Space shows.
    #[serde(deserialize_with = "de_scope")]
    pub scope: Scope,
    /// People and saved searches that stay put.
    pub pins: Vec<Pinned>,
    /// Each account's avatar colour, stored so three blue providers do not look alike.
    ///
    /// Missing entries take quire's person swatches (`ds::PersonSwatch`) in order. The colour is
    /// the account's, not the provider's.
    pub colors: BTreeMap<AccountId, String>,
}

impl Mail {
    /// A Space's mail over `scope`, nothing pinned and no colour chosen.
    pub fn over(scope: Scope) -> Mail {
        Mail {
            scope,
            ..Mail::default()
        }
    }

    /// Put `account` in this Space when it shows some accounts and not that one yet. Returns
    /// whether it changed: a Space over every account already shows it.
    pub(in crate::ui) fn widen(&mut self, account: AccountId) -> bool {
        match &mut self.scope {
            Scope::Accounts(ids) if !ids.contains(&account) => {
                ids.push(account);
                true
            }
            _ => false,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
enum ScopeRaw {
    All,
    Accounts(Vec<AccountId>),
    #[serde(other)]
    Unknown,
}

fn de_scope<'de, D>(deserializer: D) -> Result<Scope, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match serde_json::from_value::<ScopeRaw>(value) {
        Ok(ScopeRaw::Accounts(ids)) => Scope::Accounts(ids),
        Ok(ScopeRaw::All | ScopeRaw::Unknown) | Err(_) => Scope::All,
    })
}

/// The swatch an account at `index` takes when a Space has not chosen one, as stored.
fn swatch(index: usize) -> String {
    PersonSwatch::nth(index).hex().css()
}

/// The colour `id` wears in `space`, or the swatch at `index` when none was stored.
pub fn avatar_color(space: &Space, id: AccountId, index: usize) -> String {
    space
        .payload
        .colors
        .get(&id)
        .cloned()
        .unwrap_or_else(|| swatch(index))
}

/// Fill any account that has no colour yet. Returns whether the Space changed.
pub fn ensure_colors(mail: &mut Mail, accounts: &[AccountId]) -> bool {
    let mut changed = false;
    for (index, id) in accounts.iter().enumerate() {
        if mail.colors.contains_key(id) {
            continue;
        }
        mail.colors.insert(id.clone(), swatch(index));
        changed = true;
    }
    changed
}

/// One Space per account, named "Space 1" onward and tinted from quire's presets in order; no
/// accounts is one Space over every account, so the window has somewhere to put the mail.
/// Nothing is pinned.
pub fn first_run(accounts: &[AccountId]) -> Spaces {
    Spaces::first_run(
        accounts
            .iter()
            .map(|id| Mail::over(Scope::Accounts(vec![id.clone()]))),
        || Mail::over(Scope::All),
    )
}

/// Spaces made from `spaces`, each a name, a look and its mail, in order, with the one at
/// `current` on screen. For a window handed its Spaces (a test's) rather than reading them.
pub fn built(spaces: Vec<(String, SpaceLook, Mail)>, current: usize) -> Spaces {
    let (names, mails): (Vec<(String, SpaceLook)>, Vec<Mail>) = spaces
        .into_iter()
        .map(|(name, look, mail)| ((name, look), mail))
        .unzip();
    let mut made = Spaces::first_run(mails, Mail::default);
    let ids: Vec<SpaceId> = made.list().iter().map(|space| space.id).collect();
    for (id, (name, look)) in ids.iter().zip(names) {
        made.edit(*id, |space| {
            space.name = name;
            space.look = look;
        });
    }
    if let Some(id) = ids.get(current) {
        made.select(*id);
    }
    made
}

/// Where the window's Spaces and Today are kept: its directories, or nowhere (a window launched
/// with none keeps them for the session).
pub fn storage(dirs: Option<&WindowDirs>) -> SpacesStorage {
    SpacesStorage::at(
        dirs.map(|dirs| dirs.config.clone()),
        dirs.map(|dirs| dirs.state.clone()),
    )
}

/// The window's first Spaces: the stored ones, else a first run over `accounts`; the current
/// Space's accounts given a colour each. Written when that was a first run or a colour was
/// filled.
///
/// The theme moved from `appearance.json` into each Space. A Space written before that has
/// none, and takes the window-wide one `legacy` read, so the first read after the upgrade looks
/// the way the window did; a first run starts from it too.
pub fn boot(dirs: Option<&WindowDirs>, accounts: &[AccountId], legacy: &Legacy) -> Spaces {
    let theme = serde_json::to_value(legacy.theme).unwrap_or(serde_json::Value::Null);
    let raw_fix = |value: &mut serde_json::Value| inherit_theme(value, &theme);
    let made = || {
        let mut spaces = first_run(accounts);
        let ids: Vec<SpaceId> = spaces.list().iter().map(|space| space.id).collect();
        for id in ids {
            spaces.edit(id, |space| space.look.theme = legacy.theme);
        }
        spaces
    };
    let colour = |spaces: &mut Spaces| {
        let current = spaces.current().id;
        let mut changed = false;
        spaces.edit(current, |space| {
            changed = ensure_colors(&mut space.payload, accounts);
        });
        if changed { Fixup::Changed } else { Fixup::Kept }
    };
    storage(dirs).boot_spaces(raw_fix, made, colour).0
}

/// Give each stored Space that has no `theme` of its own `theme`.
///
/// Only a missing key inherits. A Space that stored a word, even one this build does not know,
/// made a choice of its own, and that word falls to the field's default as it always has.
fn inherit_theme(value: &mut serde_json::Value, theme: &serde_json::Value) {
    if let Some(list) = value
        .get_mut("spaces")
        .and_then(serde_json::Value::as_array_mut)
    {
        for space in list.iter_mut().filter_map(serde_json::Value::as_object_mut) {
            space.entry("theme").or_insert_with(|| theme.clone());
        }
    }
}

/// The Spaces as stored in `dirs`, for a window that only wears them (Settings, a message
/// window): no first run is written, and with no file the one default Space stands in.
pub fn load(dirs: &WindowDirs) -> Spaces {
    storage(Some(dirs))
        .load_spaces(|_| {})
        .unwrap_or_else(|| first_run(&[]))
}

/// Write `spaces` where the window keeps them. A write that cannot happen leaves them for the
/// session.
pub fn save(dirs: Option<&WindowDirs>, spaces: &Spaces) -> Result<(), String> {
    storage(dirs)
        .save_spaces(spaces)
        .map_err(|why| why.to_string())
}

#[cfg(test)]
mod tests;

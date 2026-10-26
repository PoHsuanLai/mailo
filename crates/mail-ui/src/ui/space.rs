//! Spaces: a tint, and whose mail they show.
//!
//! The Spaces are quire's kit (`ds::components::app::spaces`): the list, its ids, switching,
//! the look, the menu and Today are the same in every app. What a Space holds that is mail's is
//! its [`Mail`] payload: whose mail it shows and what stays pinned. Where
//! a Space was left is mailo's [`Recall`].
//!
//! The accounts are the store's, a level above the Spaces, and apart from them: a Space names the
//! ones it shows, and only those, so an account added later shows where it is put and nowhere
//! else. A Space may show no account, and an account may be in no Space; it still syncs, and is
//! in Settings › Accounts.
//!
//! Stored in `spaces.json` under the config directory, beside appearance and not in the mail
//! database: a cosmetic choice must not be a write against someone's mail. quire reads the file
//! leniently, field by field; [`Mail`] reads its own fields the same way.

use crate::ui::appearance::{Legacy, WindowDirs};
use ds::components::app::spaces;
use ds::prelude::SpaceLook;
use ds_settings::{Fixup, SpacesStorage};
use mail_client::profile::ProfileId;
use porter_core::AccountId;
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

mod forget;
mod member;
mod recall;
mod upgrade;

pub use ds::components::app::spaces::SpaceId;
pub use forget::{Forgot, forget_account, forget_unknown};
pub use mail_client::scope::Scope;
pub use member::{Member, with_member};
pub use recall::Recall;

/// One context: quire's Space over mail's payload.
pub type Space = spaces::Space<Mail>;

/// Every Space, which one is on screen, and where each was left.
pub type Spaces = spaces::Spaces<Mail, Recall>;

/// The window's Spaces as quire's controller drives them: switching, the menu, and writing.
pub type Handle = spaces::SpacesHandle<Mail, Recall>;

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
/// default, never a failure of the whole file, and a field no longer read (each account's
/// colour, from when accounts had tiles) is passed over. A file from before a Space named its
/// accounts (a `scope`, which could be every account) is brought up to date as it is read
/// ([`upgrade`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Mail {
    /// The Profile whose store this Space's accounts are in. A Space written before Profiles
    /// existed has none, and belongs to the default Profile.
    #[serde(deserialize_with = "de_profile")]
    pub profile: ProfileId,
    /// The accounts whose mail this Space shows, in the order they were put in it. None is a
    /// Space that shows nothing until one is put in it.
    #[serde(deserialize_with = "de_accounts")]
    pub accounts: Vec<AccountId>,
    /// People and saved searches that stay put.
    pub pins: Vec<Pinned>,
}

impl Mail {
    /// A Space's mail over `accounts`, nothing pinned.
    pub fn of(accounts: Vec<AccountId>) -> Mail {
        Mail {
            accounts,
            ..Mail::default()
        }
    }

    /// A new Space's mail in `profile`: no account until some are put in it, nothing pinned.
    pub fn new_in(profile: &ProfileId) -> Mail {
        Mail {
            profile: profile.clone(),
            ..Mail::default()
        }
    }

    /// The accounts this Space shows, as a view asks over them.
    pub fn scope(&self) -> Scope {
        Scope::Accounts(self.accounts.clone())
    }

    /// Whether this Space shows `account`'s mail.
    pub fn shows(&self, account: &AccountId) -> bool {
        self.accounts.contains(account)
    }

    /// Put `account`, one of `profile`'s, in this Space when it belongs to `profile` and does
    /// not show it yet. Returns whether it changed: a Space of another Profile does not take
    /// this store's accounts.
    pub(in crate::ui) fn widen(&mut self, profile: &ProfileId, account: AccountId) -> bool {
        if self.profile != *profile || self.shows(&account) {
            return false;
        }
        self.accounts.push(account);
        true
    }
}

/// A stored Profile id, or the default Profile's when it is not one: the file stays readable.
fn de_profile<'de, D>(deserializer: D) -> Result<ProfileId, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// The stored accounts, each read on its own: one that is not an account id is passed over, and
/// a list that is not a list is none.
fn de_accounts<'de, D>(deserializer: D) -> Result<Vec<AccountId>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// One Space per account, named "Space 1" onward and tinted from quire's presets in order; no
/// accounts is one Space of none, so the window has somewhere to put the first one added.
/// Nothing is pinned.
pub fn first_run(accounts: &[AccountId]) -> Spaces {
    Spaces::first_run(
        accounts.iter().map(|id| Mail::of(vec![id.clone()])),
        Mail::default,
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

/// The window's first Spaces: the stored ones, else a first run over `accounts`, the default
/// Profile's. Written when that was a first run, or when reading them changed them.
///
/// The theme moved from `appearance.json` into each Space. A Space written before that has
/// none, and takes the window-wide one `legacy` read, so the first read after the upgrade looks
/// the way the window did; a first run starts from it too. A Space written before Spaces named
/// their accounts names them now ([`upgrade`]).
pub fn boot(dirs: Option<&WindowDirs>, accounts: &[AccountId], legacy: &Legacy) -> Spaces {
    let theme = serde_json::to_value(legacy.theme).unwrap_or(serde_json::Value::Null);
    let upgraded = std::cell::Cell::new(Fixup::Kept);
    let raw_fix = |value: &mut serde_json::Value| {
        inherit_theme(value, &theme);
        upgraded.set(upgrade::name_accounts(value, accounts));
    };
    let made = || {
        let mut spaces = first_run(accounts);
        let ids: Vec<SpaceId> = spaces.list().iter().map(|space| space.id).collect();
        for id in ids {
            spaces.edit(id, |space| space.look.theme = legacy.theme);
        }
        spaces
    };
    let on_load = |_: &mut Spaces| upgraded.get();
    storage(dirs).boot_spaces(raw_fix, made, on_load).0
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
/// window): nothing is written, and with no file, or one that cannot be used, the one default
/// Space stands in. The main window boots first and writes a file it brought up to date, so a
/// Space here that still says every account is one no window has booted over, and shows none.
pub fn load(dirs: &WindowDirs) -> Spaces {
    stored(dirs).unwrap_or_else(|| first_run(&[]))
}

/// The Spaces stored in `dirs`, if there are any that can be used. Nothing is written.
pub fn stored(dirs: &WindowDirs) -> Option<Spaces> {
    match read(dirs) {
        ds_settings::SpacesLoad::Stored(spaces) => Some(spaces),
        _ => None,
    }
}

/// The Spaces file in `dirs` as it reads, without writing.
fn read(dirs: &WindowDirs) -> ds_settings::SpacesLoad<Mail, Recall> {
    let raw_fix = |value: &mut serde_json::Value| {
        upgrade::name_accounts(value, &[]);
    };
    storage(Some(dirs)).load_spaces(raw_fix)
}

/// The file the Spaces are kept in, in the config directory.
pub const FILE_NAME: &str = "spaces.json";

/// Why `spaces.json` in `dirs` cannot be used: `None` when it is missing or reads.
pub fn damage(dirs: &WindowDirs) -> Option<String> {
    match read(dirs) {
        ds_settings::SpacesLoad::Damaged(damaged) => Some(damaged.reason),
        _ => None,
    }
}

/// Write `spaces` where the window keeps them. A write that cannot happen leaves them for the
/// session.
pub fn save(dirs: Option<&WindowDirs>, spaces: &Spaces) -> Result<(), String> {
    let saved = storage(dirs)
        .save_spaces(spaces)
        .map_err(|why| why.to_string())?;
    kept(&saved);
    Ok(())
}

/// Says where a file that could not be read was moved before a new one took its place (the
/// moved file's name says which it was).
pub(super) fn kept(saved: &ds_settings::Saved) {
    if let Some(at) = &saved.kept_at {
        log::warn!(
            "mailo: a file that could not be read was kept at {} and a new one written",
            at.display()
        );
    }
}

#[cfg(test)]
mod tests;

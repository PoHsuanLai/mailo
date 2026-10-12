//! Profiles: whose mail a store holds, so that one person can keep two lives apart.
//!
//! A Profile is a store of its own: its own mail database and blobs, its accounts' settings, its
//! own signing keys in the keyring, and its own agent. Spaces sit over Profiles the way Arc's do,
//! each Space showing the mail of one; two Profiles share only the program and its app-wide
//! settings.
//!
//! The default Profile is the install that was here before Profiles were: its database stays in
//! the data directory itself and its settings in the config directory, and its keyring entries and
//! agent keep the names they always had, so nothing moves and an older mailo still finds it. Every
//! other Profile lives in a folder of its own, `profiles/<id>/` under the data directory.
//!
//! Which Profiles there are is kept in `profiles.json` in the data directory ([`Registry`]). No
//! such file is the ordinary case of one Profile, the default, and reading the registry writes
//! nothing: the file appears when a second Profile is added. Removing a Profile is two steps, so
//! that a removal stopped half way (its folder half deleted) is found and finished on the next
//! run: [`Registry::begin_remove`] records it, the caller deletes what the Profile kept, and
//! [`Registry::finish_remove`] forgets it.
//!
//! The directories are the caller's to find and hand in ([`Roots`]); this module reads no
//! environment.

use crate::config::write_json;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

const FILE_NAME: &str = "profiles.json";

/// The newest `profiles.json` this build reads. A newer one is refused rather than read and
/// written back without what it added.
const VERSION: u32 = 1;

/// The default Profile's id: the install that was here before Profiles were.
const DEFAULT_ID: &str = "default";

/// What names a Profile on disk, in the keyring and to the command line: `default`, or eight
/// lowercase hex digits.
///
/// It becomes a directory name and part of a keyring service, so it is checked once, where it
/// enters (parsing, and reading `profiles.json`), and nothing holding one checks it again.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProfileId(String);

impl ProfileId {
    /// A new id, from random bytes. It may be one already in use: [`Registry::add`] draws again
    /// until it is not.
    fn fresh() -> Self {
        let mut hex = uuid::Uuid::new_v4().simple().to_string();
        hex.truncate(8);
        Self(hex)
    }

    /// Whether this is the default Profile's.
    pub fn is_default(&self) -> bool {
        self.0 == DEFAULT_ID
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ProfileId {
    /// The default Profile's id.
    fn default() -> Self {
        Self(DEFAULT_ID.to_owned())
    }
}

fn is_valid(text: &str) -> bool {
    text == DEFAULT_ID
        || (text.len() == 8 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
}

/// Text that is not a Profile id.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a profile id: an id is \"default\" or eight lowercase hex digits")]
pub struct NotAProfileId(pub String);

impl FromStr for ProfileId {
    type Err = NotAProfileId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if is_valid(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(NotAProfileId(text.to_owned()))
        }
    }
}

impl TryFrom<String> for ProfileId {
    type Error = NotAProfileId;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if is_valid(&text) {
            Ok(Self(text))
        } else {
            Err(NotAProfileId(text))
        }
    }
}

impl From<ProfileId> for String {
    fn from(id: ProfileId) -> Self {
        id.0
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a Profile keeps what is its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Home {
    /// Where the install before Profiles kept it: the data directory itself holds the database
    /// and blobs, and the settings are the config directory's files.
    Legacy,
    /// `profiles/<id>/` under the data directory, settings in its `settings/`.
    Folder,
}

/// The two directories every Profile's paths are under, found by the caller: mailo's data
/// directory and its config directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roots {
    pub data: PathBuf,
    pub config: PathBuf,
}

/// One Profile: its id, and the name it was given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    id: ProfileId,
    /// `None` for the default Profile until someone names it: the front end words it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl Profile {
    pub fn id(&self) -> &ProfileId {
        &self.id
    }

    /// The name it was given; `None` for a default Profile nobody has named.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Where it keeps what is its own. The default Profile is the install in place, every other
    /// one has a folder.
    pub fn home(&self) -> Home {
        if self.id.is_default() {
            Home::Legacy
        } else {
            Home::Folder
        }
    }

    /// The directory its database and blobs are in.
    pub fn data_dir(&self, roots: &Roots) -> PathBuf {
        match self.home() {
            Home::Legacy => roots.data.clone(),
            Home::Folder => roots.data.join("profiles").join(self.id.as_str()),
        }
    }

    /// Its mail database.
    pub fn db(&self, roots: &Roots) -> PathBuf {
        self.data_dir(roots).join("mail.db")
    }

    /// Its blob directory: message bodies and attachments.
    pub fn blobs(&self, roots: &Roots) -> PathBuf {
        self.data_dir(roots).join("blobs")
    }

    /// The directory its own settings are in: the ones about its accounts (which are kept
    /// offline, server search, which senders' images and logos are trusted). The settings about
    /// the program (`settings.toml`, appearance, keyboard, `spaces.json`) are app-wide and stay
    /// in the config directory whichever Profile is open.
    pub fn settings_dir(&self, roots: &Roots) -> PathBuf {
        match self.home() {
            Home::Legacy => roots.config.clone(),
            Home::Folder => self.data_dir(roots).join("settings"),
        }
    }

    /// The keyring service its signing keys are kept under: `mailo` for the default Profile, as
    /// before, and `mailo/<id>` for another, so one key in two Profiles is two entries.
    pub fn signing_service(&self) -> String {
        match self.home() {
            Home::Legacy => "mailo".to_owned(),
            Home::Folder => format!("mailo/{}", self.id),
        }
    }

    /// The name of its agent, the one process that holds its store: `mailo` for the default
    /// Profile, as before, and `mailo-<id>` for another.
    pub fn agent_name(&self) -> String {
        match self.home() {
            Home::Legacy => "mailo".to_owned(),
            Home::Folder => format!("mailo-{}", self.id),
        }
    }
}

/// `profiles.json` as it is written. Every field has a default, so a later field reads against
/// an older file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct File {
    version: u32,
    profiles: Vec<Profile>,
    /// Profiles whose removal began and has not finished.
    removing: Vec<ProfileId>,
}

impl Default for File {
    fn default() -> Self {
        Self {
            version: VERSION,
            profiles: Vec::new(),
            removing: Vec::new(),
        }
    }
}

impl File {
    /// The default Profile first, whether or not the file names it, and each id once.
    fn normalized(mut self) -> Self {
        let mut seen = std::collections::BTreeSet::new();
        self.profiles.retain(|profile| {
            let first = seen.insert(profile.id.clone());
            if !first {
                let id = &profile.id;
                log::warn!("{FILE_NAME}: profile {id} is listed twice; keeping the first");
            }
            first
        });
        match self.profiles.iter().position(|p| p.id.is_default()) {
            Some(0) => {}
            Some(at) => {
                let default = self.profiles.remove(at);
                self.profiles.insert(0, default);
            }
            None => self.profiles.insert(
                0,
                Profile {
                    id: ProfileId::default(),
                    name: None,
                },
            ),
        }
        let mut seen = std::collections::BTreeSet::new();
        self.removing.retain(|id| seen.insert(id.clone()));
        self
    }
}

/// The Profiles there are, read from and written to `profiles.json` in a data directory.
#[derive(Debug, Clone)]
pub struct Registry {
    root: PathBuf,
    file: File,
}

impl Registry {
    /// The Profiles kept in `data_root`: one, the default, when there is no `profiles.json`.
    /// Writes nothing.
    ///
    /// A file that cannot be read, is not a registry, or was written by a newer mailo is an
    /// error, not one default Profile: that would hide every other Profile, and the next save
    /// would forget them.
    pub fn load(data_root: &Path) -> Result<Self, CoreError> {
        let path = data_root.join(FILE_NAME);
        let file = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<File>(&bytes)
                .map_err(|e| CoreError::context(path.display().to_string(), e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => File::default(),
            Err(e) => return Err(CoreError::at(&path)(e)),
        };
        if file.version > VERSION {
            return Err(CoreError::ProfilesTooNew {
                path,
                version: file.version,
            });
        }
        Ok(Self {
            root: data_root.to_path_buf(),
            file: file.normalized(),
        })
    }

    /// Write the registry to `profiles.json`, through a temporary file and a rename.
    pub fn save(&self) -> Result<(), CoreError> {
        write_json(&self.root, FILE_NAME, &self.file)
    }

    /// Every Profile, the default first.
    pub fn profiles(&self) -> &[Profile] {
        &self.file.profiles
    }

    pub fn get(&self, id: &ProfileId) -> Option<&Profile> {
        self.file.profiles.iter().find(|p| &p.id == id)
    }

    /// The Profile `text` names: its id in any case, else its name in any case.
    pub fn resolve(&self, text: &str) -> Result<&Profile, CoreError> {
        let by_id = text
            .to_ascii_lowercase()
            .parse::<ProfileId>()
            .ok() // text that is no id may still be a name, tried next
            .and_then(|id| self.get(&id));
        let wanted = text.trim().to_lowercase();
        by_id
            .or_else(|| {
                self.file.profiles.iter().find(|p| {
                    p.name
                        .as_deref()
                        .is_some_and(|n| n.to_lowercase() == wanted)
                })
            })
            .ok_or_else(|| CoreError::NoProfile(text.to_owned()))
    }

    /// A new Profile called `name`, with a folder home and an id no Profile has. Not saved until
    /// [`save`](Self::save).
    pub fn add(&mut self, name: &str) -> Result<Profile, CoreError> {
        let name = self.usable_name(name, None)?;
        let id = loop {
            let id = ProfileId::fresh();
            if self.get(&id).is_none() && !self.file.removing.contains(&id) {
                break id;
            }
        };
        let profile = Profile {
            id,
            name: Some(name),
        };
        self.file.profiles.push(profile.clone());
        Ok(profile)
    }

    /// Call the Profile `id` `name`. Not saved until [`save`](Self::save).
    pub fn rename(&mut self, id: &ProfileId, name: &str) -> Result<(), CoreError> {
        let name = self.usable_name(name, Some(id))?;
        let profile = self
            .file
            .profiles
            .iter_mut()
            .find(|p| &p.id == id)
            .ok_or_else(|| CoreError::NoProfile(id.to_string()))?;
        profile.name = Some(name);
        Ok(())
    }

    /// `name` trimmed, if it can name a Profile other than `except`: not blank, not something
    /// [`resolve`](Self::resolve) would read as an id, and no other Profile's name.
    fn usable_name(&self, name: &str, except: Option<&ProfileId>) -> Result<String, CoreError> {
        let name = name.trim();
        if name.is_empty() || name.to_ascii_lowercase().parse::<ProfileId>().is_ok() {
            return Err(CoreError::ProfileNameUnusable(name.to_owned()));
        }
        let lower = name.to_lowercase();
        let taken = self.file.profiles.iter().any(|p| {
            Some(&p.id) != except && p.name.as_deref().is_some_and(|n| n.to_lowercase() == lower)
        });
        if taken {
            return Err(CoreError::ProfileNameTaken(name.to_owned()));
        }
        Ok(name.to_owned())
    }

    /// Record that the Profile `id` is being removed, and save. It stays listed until
    /// [`finish_remove`](Self::finish_remove); deleting what it kept is the caller's, between
    /// the two.
    pub fn begin_remove(&mut self, id: &ProfileId) -> Result<(), CoreError> {
        if id.is_default() {
            return Err(CoreError::DefaultProfileStays);
        }
        if self.get(id).is_none() {
            return Err(CoreError::NoProfile(id.to_string()));
        }
        if !self.file.removing.contains(id) {
            self.file.removing.push(id.clone());
        }
        self.save()
    }

    /// Forget the Profile `id`, whose removal is done, and save.
    pub fn finish_remove(&mut self, id: &ProfileId) -> Result<(), CoreError> {
        if id.is_default() {
            return Err(CoreError::DefaultProfileStays);
        }
        self.file.profiles.retain(|p| &p.id != id);
        self.file.removing.retain(|r| r != id);
        self.save()
    }

    /// The Profiles whose removal began and did not finish, to be finished.
    pub fn pending_removals(&self) -> &[ProfileId] {
        &self.file.removing
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;

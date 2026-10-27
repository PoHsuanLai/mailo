//! How each account is synced: whether all its mail is kept on this computer, large attachments
//! included, how often a timed pass checks it, how far back its mail is downloaded, and, for
//! POP3, when the server's copy of a message is removed.
//!
//! A sync fetches every message, but a large IMAP message only as its text: its attachments stay
//! on the server until someone opens one. An account kept offline in full has
//! them fetched too, a few each pass, after the bodies and largest last. Off unless asked for.
//!
//! The other three are the account page's in Settings, beside Download All Mail, and are kept the
//! same way: [`Every`] is what the window's scheduler and `mailo watch` time passes by
//! ([`crate::sync::due`]), [`Window`] how old a message may be and still have its body fetched by
//! a pass, and [`Remove`] when a POP3 pass sends `DELE` for what it holds here. Each account that
//! has never been set is the default, which is how every account synced before these existed.
//!
//! Preferences, so they live beside the others in the config directory (`offline.json`), not in
//! the mail database and not in the account's plan, which is a frozen domain type. Keyed by
//! account id, as `spaces.json` keys whose mail a Space shows; an id the database no longer has
//! is simply never asked about.

use crate::config::{json_damage, read_json, write_json};
use crate::error::CoreError;
use chrono::{DateTime, TimeDelta, Utc};
use mail_store::{Offline, Store};
use porter_core::AccountId;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// How much of an account a sync keeps here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Keep {
    /// Every message, with large attachments left on the server until opened.
    #[default]
    Bodies,
    /// Every message and every attachment.
    Everything,
}

/// The file the setting is kept in, in the config directory.
pub const FILE_NAME: &str = "offline.json";

/// How often a timed pass checks an account for new mail: the account page's Check for New Mail.
///
/// [`Every::Automatic`] is what every account did before this was a choice: a server that can
/// push is waited on, and anything else is polled at the interval its capabilities ask for
/// ([`crate::sync::due::every`]). A fixed interval is polled at that, and is never pushed to, as
/// Mail does: a person who asks for every fifteen minutes is not asking for a connection held
/// open. [`Every::Manually`] is neither: no timed pass and no connection, only a person's Sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Every {
    #[default]
    Automatic,
    /// Every this many minutes.
    Minutes(u16),
    Manually,
}

impl Every {
    /// What the account page offers, in its order.
    pub const CHOICES: [Every; 7] = [
        Every::Automatic,
        Every::Minutes(1),
        Every::Minutes(5),
        Every::Minutes(15),
        Every::Minutes(30),
        Every::Minutes(60),
        Every::Manually,
    ];
}

/// How far back a pass downloads an account's mail: the account page's Download Mail From.
///
/// A message older than the window is still listed, from the headers every pass fetches, but its
/// body is not fetched until it is opened, or shown on the screen ([`mail_runtime::wanted`]).
/// Headers are what a POP3 server and a list need to say how old a message is at all, so they are
/// fetched whatever its age; it is the body, and its attachments, that make an account large.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Window {
    #[default]
    All,
    /// The last this many days.
    Days(u16),
}

impl Window {
    /// What the account page offers, in its order.
    pub const CHOICES: [Window; 4] = [
        Window::All,
        Window::Days(30),
        Window::Days(90),
        Window::Days(365),
    ];

    /// The oldest a message may be dated at `now` and still be downloaded; `None` for all of it.
    pub fn since(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            Window::All => None,
            Window::Days(days) => Some(now - TimeDelta::days(i64::from(days))),
        }
    }
}

/// When a POP3 pass removes the server's copy of a message this computer holds in full: the
/// account page's Remove from Server.
///
/// [`Remove::Never`] by default, whatever the plan's `LeaveOnServer` says: nothing sent `DELE`
/// before this, so an account added with delete-after-fetch has always left its mail on the
/// server, and an upgrade must not start deleting it unasked. A message's age is its date, the
/// only time the store keeps of it. Only a message whose body is held here is ever removed, so
/// a first sync that stops halfway never leaves a message nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "v", rename_all = "snake_case")]
pub enum Remove {
    #[default]
    Never,
    /// Once the message is this many days old.
    AfterDays(u16),
    /// Once it is in the Trash here, or deleted forever.
    WhenDeleted,
}

impl Remove {
    /// What the account page offers, in its order.
    pub const CHOICES: [Remove; 5] = [
        Remove::Never,
        Remove::AfterDays(1),
        Remove::AfterDays(7),
        Remove::AfterDays(30),
        Remove::WhenDeleted,
    ];
}

/// The stored setting: the accounts kept offline in full, and each account's other choices where
/// it has made one. Every other account is [`Keep::Bodies`] and the defaults. An account set back
/// to a default is left out, so the file holds only what differs.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Kept {
    #[serde(default)]
    everything: BTreeSet<AccountId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    every: BTreeMap<AccountId, Every>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    window: BTreeMap<AccountId, Window>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    remove: BTreeMap<AccountId, Remove>,
}

/// `map` with `account` set to `value`, or left out when that is the default.
fn chosen<T: Default + PartialEq>(map: &mut BTreeMap<AccountId, T>, account: AccountId, value: T) {
    if value == T::default() {
        map.remove(&account);
    } else {
        map.insert(account, value);
    }
}

impl Kept {
    /// What `account` keeps.
    pub fn of(&self, account: AccountId) -> Keep {
        if self.everything.contains(&account) {
            Keep::Everything
        } else {
            Keep::Bodies
        }
    }

    /// The same, with `account` set to `keep`.
    pub fn with(mut self, account: AccountId, keep: Keep) -> Self {
        match keep {
            Keep::Everything => self.everything.insert(account),
            Keep::Bodies => self.everything.remove(&account),
        };
        self
    }

    /// How often `account` is checked.
    pub fn every_of(&self, account: &AccountId) -> Every {
        // Never chosen is the default: what every account did before this was a choice.
        self.every.get(account).copied().unwrap_or_default()
    }

    /// How far back `account` is downloaded.
    pub fn window_of(&self, account: &AccountId) -> Window {
        // Never chosen is the default: what every account did before this was a choice.
        self.window.get(account).copied().unwrap_or_default()
    }

    /// When `account`'s server copies are removed.
    pub fn remove_of(&self, account: &AccountId) -> Remove {
        // Never chosen is the default: what every account did before this was a choice.
        self.remove.get(account).copied().unwrap_or_default()
    }

    /// The same, with `account` checked `every`.
    pub fn with_every(mut self, account: AccountId, every: Every) -> Self {
        chosen(&mut self.every, account, every);
        self
    }

    /// The same, with `account` downloaded as far back as `window`.
    pub fn with_window(mut self, account: AccountId, window: Window) -> Self {
        chosen(&mut self.window, account, window);
        self
    }

    /// The same, with `account`'s server copies removed by `remove`.
    pub fn with_remove(mut self, account: AccountId, remove: Remove) -> Self {
        chosen(&mut self.remove, account, remove);
        self
    }

    /// The same, with nothing kept of `account`: what its removal leaves.
    pub fn without(self, account: AccountId) -> Self {
        self.with(account.clone(), Keep::Bodies)
            .with_every(account.clone(), Every::default())
            .with_window(account.clone(), Window::default())
            .with_remove(account, Remove::default())
    }
}

/// The stored setting, or every account off when there is none or it cannot be read. The window
/// and the command line ask about a file that cannot be read ([`damage`]) before anything runs
/// over it.
pub fn load(dir: &Path) -> Kept {
    read_json(dir, FILE_NAME)
}

/// Why `offline.json` in `dir` cannot be read: `None` when it is missing or reads.
pub fn damage(dir: &Path) -> Option<String> {
    json_damage::<Kept>(dir, FILE_NAME)
}

/// The setting in the user's config directory, for a sync the binary or the window starts.
/// Every account off when there is no config directory.
pub fn load_default() -> Kept {
    crate::config::config_dir()
        .map(|dir| load(&dir))
        .unwrap_or_default()
}

/// Set what `account` keeps, leaving every other account as it was. A file that cannot be read is
/// left as it is, and nothing is set: writing would put the defaults in place of what it held.
pub fn save(dir: &Path, account: AccountId, keep: Keep) -> Result<(), CoreError> {
    change(dir, |kept| kept.with(account, keep))
}

/// Write the setting in `dir` as `edit` leaves it, every other account as it was; [`save`]'s
/// rule for a file that cannot be read holds for every choice.
pub fn change(dir: &Path, edit: impl FnOnce(Kept) -> Kept) -> Result<(), CoreError> {
    if let Some(why) = damage(dir) {
        return Err(CoreError::Damaged {
            path: dir.join(FILE_NAME),
            why,
        });
    }
    write_json(dir, FILE_NAME, &edit(load(dir)))
}

/// "1,204 of 1,310 messages offline", and what still waits on the server when anything does.
pub fn said(offline: &Offline) -> String {
    let mut out = format!(
        "{} of {} messages offline",
        grouped(offline.held),
        grouped(offline.messages)
    );
    if offline.parts_remote > 0 {
        out.push_str(&format!(
            "; {} {} ({}) on the server",
            grouped(offline.parts_remote),
            if offline.parts_remote == 1 {
                "attachment"
            } else {
                "attachments"
            },
            crate::attach::human_size(offline.remote_bytes)
        ));
    }
    out
}

/// `1310` as `1,310`.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Where one account stands: what it keeps, and how much of it is here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    pub address: String,
    pub keep: Keep,
    pub counted: Offline,
}

/// Set what one account keeps (when `set` is given), then say where each stands: the account
/// whose address is `address` (any case), or every one when there is none.
///
/// `dir` is `None` when there is no home directory to keep the setting in; then only a request
/// to change it fails.
pub fn standing(
    dir: Option<&Path>,
    store: &dyn Store,
    accounts: &[(AccountId, String)],
    address: Option<&str>,
    set: Option<Keep>,
) -> Result<Vec<Standing>, CoreError> {
    let chosen: Vec<&(AccountId, String)> = match address {
        None => accounts.iter().collect(),
        Some(address) => {
            let found: Vec<_> = accounts
                .iter()
                .filter(|(_, a)| a.eq_ignore_ascii_case(address))
                .collect();
            if found.is_empty() {
                return Err(CoreError::UnknownAccount(address.to_owned()));
            }
            found
        }
    };
    if let Some(keep) = set {
        let Some(dir) = dir else {
            return Err(CoreError::NoConfigDir);
        };
        for (id, _) in &chosen {
            save(dir, id.clone(), keep)?;
        }
    }
    let kept = dir.map(load).unwrap_or_default();
    let mut out = Vec::new();
    for (id, address) in chosen {
        out.push(Standing {
            address: address.clone(),
            keep: kept.of(id.clone()),
            counted: store.offline(id.clone())?,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_domain::id::account_id_from_uuid;

    fn acct_ada() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a1"))
    }
    fn acct_bea() -> AccountId {
        account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-0000000000a2"))
    }

    #[test]
    fn off_until_turned_on_and_one_account_at_a_time() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load(dir.path()).of(acct_ada()),
            Keep::Bodies,
            "off by default"
        );

        save(dir.path(), acct_ada(), Keep::Everything).unwrap();
        let kept = load(dir.path());
        assert_eq!(kept.of(acct_ada()), Keep::Everything);
        assert_eq!(
            kept.of(acct_bea()),
            Keep::Bodies,
            "the other account is untouched"
        );

        save(dir.path(), acct_bea(), Keep::Everything).unwrap();
        save(dir.path(), acct_ada(), Keep::Bodies).unwrap();
        let kept = load(dir.path());
        assert_eq!(
            (kept.of(acct_ada()), kept.of(acct_bea())),
            (Keep::Bodies, Keep::Everything)
        );
    }

    /// Each account's choices are its own, a default is left out of the file, and a file written
    /// before they existed reads as every account at the defaults.
    #[test]
    fn each_choice_is_one_accounts_and_a_default_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        change(dir.path(), |kept| {
            kept.with_every(acct_ada(), Every::Minutes(15))
                .with_window(acct_ada(), Window::Days(90))
                .with_remove(acct_bea(), Remove::AfterDays(7))
        })
        .unwrap();
        let kept = load(dir.path());
        assert_eq!(
            (
                kept.every_of(&acct_ada()),
                kept.window_of(&acct_ada()),
                kept.remove_of(&acct_ada())
            ),
            (Every::Minutes(15), Window::Days(90), Remove::Never)
        );
        assert_eq!(
            (
                kept.every_of(&acct_bea()),
                kept.window_of(&acct_bea()),
                kept.remove_of(&acct_bea())
            ),
            (Every::Automatic, Window::All, Remove::AfterDays(7))
        );

        change(dir.path(), |kept| {
            kept.without(acct_ada()).without(acct_bea())
        })
        .unwrap();
        let written = std::fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();
        assert_eq!(
            written.trim(),
            r#"{"everything":[]}"#,
            "the defaults are not written"
        );

        std::fs::write(
            dir.path().join(FILE_NAME),
            r#"{"everything":["00000000-0000-4000-8000-0000000000a1"]}"#,
        )
        .unwrap();
        let older = load(dir.path());
        assert_eq!(older.of(acct_ada()), Keep::Everything);
        assert_eq!(older.every_of(&acct_ada()), Every::Automatic);
    }

    #[test]
    fn a_window_reaches_back_its_days_and_all_of_it_reaches_back_for_ever() {
        let now = chrono::Utc::now();
        assert_eq!(Window::All.since(now), None);
        assert_eq!(Window::Days(30).since(now), Some(now - TimeDelta::days(30)));
    }

    #[test]
    fn a_damaged_file_is_every_account_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), b"{not json").unwrap();
        assert_eq!(load(dir.path()).of(acct_ada()), Keep::Bodies);
    }

    #[test]
    fn a_damaged_file_is_said_and_never_written_over() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(damage(dir.path()), None, "no file is not damage");
        std::fs::write(dir.path().join(FILE_NAME), b"{not json").unwrap();
        assert!(damage(dir.path()).is_some());
        assert!(matches!(
            save(dir.path(), acct_ada(), Keep::Everything),
            Err(CoreError::Damaged { .. })
        ));
        assert_eq!(
            std::fs::read(dir.path().join(FILE_NAME)).unwrap(),
            b"{not json"
        );
    }

    #[test]
    fn the_count_reads_as_a_sentence() {
        const CASES: &[(Offline, &str)] = &[
            (
                Offline {
                    messages: 1310,
                    held: 1204,
                    parts_remote: 0,
                    remote_bytes: 0,
                },
                "1,204 of 1,310 messages offline",
            ),
            (
                Offline {
                    messages: 3,
                    held: 2,
                    parts_remote: 1,
                    remote_bytes: 2 * 1024 * 1024,
                },
                "2 of 3 messages offline; 1 attachment (2.0 MB) on the server",
            ),
        ];
        for (offline, want) in CASES {
            assert_eq!(said(offline), *want, "{offline:?}");
        }
    }
}

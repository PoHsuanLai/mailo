//! Which accounts keep all their mail on this computer, large attachments included.
//!
//! A sync fetches every message, but a large IMAP message only as its text: its attachments stay
//! on the server until someone opens one (`plan.md` 9.6). An account kept offline in full has
//! them fetched too, a few each pass, after the bodies and largest last. Off unless asked for.
//!
//! A preference, so it lives beside the others in the config directory (`offline.json`), not in
//! the mail database and not in the account's plan, which is a frozen domain type. Keyed by
//! account id, as `spaces.json` keys whose mail a Space shows; an id the database no longer has
//! is simply never asked about.

use crate::config::{read_json, write_json};
use mail_store::{Offline, Store};
use porter_core::AccountId;
use std::collections::BTreeSet;
use std::fmt::Write as _;
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

const FILE_NAME: &str = "offline.json";

/// The stored setting: the accounts kept offline in full. Every other account is [`Keep::Bodies`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Kept {
    #[serde(default)]
    everything: BTreeSet<AccountId>,
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
}

/// The stored setting, or every account off when there is none or it cannot be read.
pub fn load(dir: &Path) -> Kept {
    read_json(dir, FILE_NAME)
}

/// The setting in the user's config directory, for a sync the binary or the window starts.
/// Every account off when there is no config directory.
pub fn load_default() -> Kept {
    crate::config::config_dir()
        .map(|dir| load(&dir))
        .unwrap_or_default()
}

/// Set what `account` keeps, leaving every other account as it was.
pub fn save(dir: &Path, account: AccountId, keep: Keep) -> Result<(), String> {
    write_json(dir, FILE_NAME, &load(dir).with(account, keep))
}

/// "1,204 of 1,310 messages offline", and what still waits on the server when anything does.
pub fn said(offline: &Offline) -> String {
    let mut out = format!(
        "{} of {} messages offline",
        grouped(offline.held),
        grouped(offline.messages)
    );
    if offline.parts_remote > 0 {
        let _ = write!(
            out,
            "; {} {} ({}) on the server",
            grouped(offline.parts_remote),
            if offline.parts_remote == 1 {
                "attachment"
            } else {
                "attachments"
            },
            crate::attach::human_size(offline.remote_bytes)
        );
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

/// `mailo offline [<address> [on|off]]`: set one account, or say where each stands.
///
/// `dir` is `None` when there is no home directory to keep the setting in; then only a request
/// to change it fails.
pub fn command(
    dir: Option<&Path>,
    store: &dyn Store,
    accounts: &[(AccountId, String)],
    address: Option<&str>,
    set: Option<Keep>,
) -> Result<String, String> {
    let chosen: Vec<&(AccountId, String)> = match address {
        None => accounts.iter().collect(),
        Some(address) => {
            let found: Vec<_> = accounts
                .iter()
                .filter(|(_, a)| a.eq_ignore_ascii_case(address))
                .collect();
            if found.is_empty() {
                return Err(format!("no account {address:?}"));
            }
            found
        }
    };
    if let Some(keep) = set {
        let Some(dir) = dir else {
            return Err("no config directory (neither XDG_CONFIG_HOME nor HOME is set)".to_owned());
        };
        for (id, _) in &chosen {
            save(dir, id.clone(), keep)?;
        }
    }
    let kept = dir.map(load).unwrap_or_default();
    let mut out = String::new();
    for (id, address) in chosen {
        let counted = store.offline(id.clone()).map_err(|e| e.to_string())?;
        let keeps = match kept.of(id.clone()) {
            Keep::Everything => "all mail kept offline",
            Keep::Bodies => "large attachments left on the server until opened",
        };
        let _ = writeln!(out, "{address}: {keeps}; {}", said(&counted));
    }
    if out.is_empty() {
        out.push_str("no accounts. Add one with: mailo account add <address>\n");
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

    #[test]
    fn a_damaged_file_is_every_account_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE_NAME), b"{not json").unwrap();
        assert_eq!(load(dir.path()).of(acct_ada()), Keep::Bodies);
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

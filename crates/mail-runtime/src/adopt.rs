//! One-shot adoption of what earlier builds kept in the keyring under the service `mailo`.
//!
//! Until E2 an account's secrets were entries `<account>:incoming|outgoing|oauth|carddav` in
//! mailo's own JSON ([`Stored`]), in parts where the Windows Credential Manager could not hold
//! one entry ([`chunks`]). They are porter's now, filed through [`porter_secrets::Secrets`] under
//! porter's attributes, and this moves what an earlier build left. For each entry:
//!
//! 1. read it as it was written (through [`chunks::get`], so a Windows entry in parts is read
//!    whole), decoding it with the old [`Stored`] reader;
//! 2. file it under porter's key for the same purpose;
//! 3. read that back and compare;
//! 4. only then delete the old entry, every part and the head ([`chunks::forget`]).
//!
//! It is idempotent (a second run finds no entry and does nothing), resumable (every cut between
//! two of those steps leaves a state the next run finishes, see the table in `tests`), and it
//! never loses a secret: an entry is deleted only when porter's store has been seen to hold the
//! same credential, or already held one. A failed put keeps the old entry and the account usable
//! through it ([`crate::account_secrets::PlatformSecrets`] falls back to it) until the next run.
//!
//! When porter's store already holds the key, its credential wins and the old entry is only
//! deleted: the only writer under porter's attributes is this process, after it began to use them,
//! so what is filed is at least as new as what an earlier build wrote (a refresh token that was
//! rotated since would otherwise be put back as the stale one).
//!
//! Completion is recorded per account in mail-store (migration 0028, `secrets_adopted`): an
//! account is recorded once none of its four entries is left. A recorded account is not asked
//! about again.
//!
//! The accounts daemon's `Adopt` (E6) reaches the same state from the other side: the items are
//! the same Secret Service items under porter's attributes, so when accountd later adopts an
//! account mailo has already adopted, its secrets are filed. See FINDINGS.

use crate::RuntimeError;
use crate::signing_store::chunks::{self, Slots};
use crate::signing_store::stored::Stored;
use chrono::{DateTime, Utc};
use mail_store::SqliteStore;
use porter_core::UnixSeconds;
use porter_core::{AccountId, CapabilityKind, Credential, SecretKey, SecretPurpose, SecretText};
use porter_secrets::{Secrets, SecretsError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

/// The old entries, as [`chunks`] reads and writes them. Shared and `'static` because each call
/// runs on a thread of its own ([`on_thread`]).
pub type Legacy = Arc<dyn Slots + Send + Sync>;

/// The four things an earlier build kept for an account, and the purpose porter files each under.
const ITEMS: [(&str, SecretPurpose); 4] = [
    ("incoming", SecretPurpose::IncomingPassword),
    ("outgoing", SecretPurpose::OutgoingPassword),
    ("oauth", SecretPurpose::OAuthRefresh),
    (
        "carddav",
        SecretPurpose::ServicePassword(CapabilityKind::Contacts),
    ),
];

/// The entry an earlier build kept `key` in, or `None` for a purpose it never kept.
fn legacy_name(key: &SecretKey) -> Option<String> {
    ITEMS
        .iter()
        .find(|(_, purpose)| *purpose == key.purpose)
        .map(|(word, _)| format!("{}:{word}", key.account))
}

/// Whether a run has left no account with an entry to move, shared by the run and the platform
/// store that falls back to the old entries until then. See [`PlatformSecrets`].
///
/// One per process for the platform's ([`platform_drained`]); the tests make their own.
#[derive(Debug, Clone, Default)]
pub struct Drained(Arc<AtomicBool>);

impl Drained {
    /// A run in this process found no entry left to move.
    pub fn get(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn set(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The process's: what [`run`] sets and the platform store reads.
pub(crate) fn platform_drained() -> Drained {
    static DRAINED: OnceLock<Drained> = OnceLock::new();
    DRAINED.get_or_init(Drained::default).clone()
}

/// Run `work` on a thread of its own and wait for it without blocking this one.
///
/// The old entries are read with keyring-core's blocking calls, which on Linux reach zbus's
/// blocking API, which under zbus's `tokio` feature starts a runtime of its own per call and
/// panics on a thread that already has one, as a spawned blocking task does. A plain thread has
/// none. This is the legacy half only: no account secret is read this way.
async fn on_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.await.ok()
}

pub(crate) fn platform_legacy() -> Legacy {
    Arc::new(crate::signing_store::Keyring)
}

/// What an old entry held, as porter's credential.
fn credential_of(stored: Stored) -> Result<Credential, RuntimeError> {
    match stored {
        Stored::Password(password) => Ok(Credential::Password(SecretText::new(password))),
        Stored::OAuth {
            access,
            refresh,
            expires_at,
        } => Ok(Credential::OAuth {
            access: SecretText::new(access),
            refresh: SecretText::new(refresh),
            expires_at: UnixSeconds(expires_at.timestamp()),
        }),
        Stored::OpenPgp(_) | Stored::SmimeKey(_) => Err(RuntimeError::Secrets(
            "the keyring entry holds a signing key, not an account's credential".to_owned(),
        )),
    }
}

/// The old entry `name`, whole and decoded: `None` when there is none.
fn read_legacy(slots: &dyn Slots, name: &str) -> Result<Option<Credential>, RuntimeError> {
    let Some(text) = chunks::get(slots, name)? else {
        return Ok(None);
    };
    credential_of(crate::signing_store::decode(&text)?).map(Some)
}

/// The old entry for `key`, for the platform store's fallback. Anything that stops it being read
/// is no entry.
pub(crate) async fn legacy_credential(legacy: &Legacy, key: &SecretKey) -> Option<Credential> {
    let name = legacy_name(key)?;
    let slots = legacy.clone();
    on_thread(move || read_legacy(slots.as_ref(), &name).ok().flatten())
        .await
        .flatten()
}

/// Forget the old entry for `key`, if there is one and it can be reached.
pub(crate) async fn forget_legacy(legacy: &Legacy, key: &SecretKey) {
    if let Some(name) = legacy_name(key) {
        forget(legacy, &name).await;
    }
}

/// Forget the old entries of `account`, as far as they can be reached.
pub(crate) async fn forget_legacy_account(legacy: &Legacy, account: &AccountId) {
    for (word, _) in ITEMS {
        forget(legacy, &format!("{account}:{word}")).await;
    }
}

/// Why an entry was left for the next run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Left {
    /// The old entry cannot be read: damaged, in the wrong shape, or the keyring is out of reach.
    Unreadable,
    /// porter's store could not be asked what it holds.
    StoreUnavailable,
    /// porter's store refused the put. The old entry stays and the account is used through it.
    PutFailed,
    /// What was filed did not read back the same. The old entry stays.
    ReadBackDiffers,
    /// Filed and verified, but the old entry could not be deleted: the next run deletes it.
    DeleteFailed,
}

/// What became of one entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// There was no old entry.
    Absent,
    /// Filed, read back the same, and the old entry deleted.
    Moved,
    /// porter's store already held the key (an earlier run was cut after the put): the old entry
    /// is deleted and what is filed stays.
    AlreadyFiled,
    /// Left for the next run.
    Left(Left),
}

impl Item {
    fn done(&self) -> bool {
        !matches!(self, Item::Left(_))
    }
}

/// Adopt one old entry. `secrets` is porter's store; `name` the old entry's; `key` porter's key.
pub async fn adopt_item(
    legacy: &Legacy,
    secrets: &impl Secrets,
    name: &str,
    key: &SecretKey,
) -> Item {
    let read = {
        let (slots, name) = (legacy.clone(), name.to_owned());
        on_thread(move || read_legacy(slots.as_ref(), &name)).await
    };
    let held = match read {
        Some(Ok(None)) => return Item::Absent,
        Some(Ok(Some(held))) => Ok(held),
        Some(Err(why)) => Err(why),
        None => Err(RuntimeError::Secrets("the keyring thread died".to_owned())),
    };
    match secrets.get(key).await {
        // Filed already: porter's wins. The old entry goes whether or not it can be read, which
        // is how an entry whose parts were half deleted by a cut is finished.
        Ok(_) => {
            return match forget(legacy, name).await {
                true => Item::AlreadyFiled,
                false => Item::Left(Left::DeleteFailed),
            };
        }
        Err(SecretsError::Missing | SecretsError::Unreadable) => {}
        Err(SecretsError::Locked | SecretsError::Unavailable) => {
            return Item::Left(Left::StoreUnavailable);
        }
    }
    let Ok(held) = held else {
        return Item::Left(Left::Unreadable);
    };
    if secrets.put(key, &held).await.is_err() {
        return Item::Left(Left::PutFailed);
    }
    match secrets.get(key).await {
        Ok(back) if back == held => {}
        _ => return Item::Left(Left::ReadBackDiffers),
    }
    match forget(legacy, name).await {
        true => Item::Moved,
        false => Item::Left(Left::DeleteFailed),
    }
}

async fn forget(legacy: &Legacy, name: &str) -> bool {
    let (slots, name) = (legacy.clone(), name.to_owned());
    matches!(
        on_thread(move || chunks::forget(slots.as_ref(), &name)).await,
        Some(Ok(()))
    )
}

/// What one account's four entries came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adopted {
    pub account: AccountId,
    pub items: Vec<(&'static str, Item)>,
}

impl Adopted {
    /// No entry of this account is left to move.
    pub fn finished(&self) -> bool {
        self.items.iter().all(|(_, item)| item.done())
    }

    /// Every entry's read of the old keyring failed: it is out of reach, not damaged.
    fn out_of_reach(&self) -> bool {
        self.items
            .iter()
            .all(|(_, item)| *item == Item::Left(Left::Unreadable))
    }
}

/// Adopt the four entries of `account`.
pub async fn adopt_account(
    legacy: &Legacy,
    secrets: &impl Secrets,
    account: &AccountId,
) -> Adopted {
    let mut items = Vec::new();
    for (word, purpose) in ITEMS {
        let key = SecretKey {
            account: account.clone(),
            purpose,
        };
        let item = adopt_item(legacy, secrets, &format!("{account}:{word}"), &key).await;
        items.push((word, item));
    }
    Adopted {
        account: account.clone(),
        items,
    }
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Accounts with nothing left to move, now recorded as adopted.
    pub finished: Vec<AccountId>,
    /// Accounts with something left for the next run, and why.
    pub unfinished: Vec<Adopted>,
    /// The entries filed and deleted by this run.
    pub moved: usize,
}

impl Report {
    /// This run had nothing to do: every account was already recorded.
    pub fn nothing_to_do(&self) -> bool {
        self.finished.is_empty() && self.unfinished.is_empty()
    }
}

/// Adopt every account the store does not record as adopted: the platform's old entries into the
/// platform's store (not [`PlatformSecrets`](crate::PlatformSecrets), whose fallback to the old
/// entries would make an entry look filed that is not), and record each that finished.
///
/// In a debug build run by `dev/scenarios` it is the scenario directory's files instead, and
/// never the person's keyring.
pub async fn run(
    store: &SqliteStore,
    now: DateTime<Utc>,
) -> Result<Report, mail_store::StoreError> {
    let (legacy, drained) = (platform_legacy(), platform_drained());
    #[cfg(debug_assertions)]
    if let Some(files) = crate::account_secrets::scenario_files() {
        return run_over(store, &legacy, &drained, &files, now).await;
    }
    run_over(
        store,
        &legacy,
        &drained,
        &crate::account_secrets::native(),
        now,
    )
    .await
}

/// [`run`] over given old entries: what the tests do.
pub async fn run_over(
    store: &SqliteStore,
    legacy: &Legacy,
    drained: &Drained,
    secrets: &impl Secrets,
    now: DateTime<Utc>,
) -> Result<Report, mail_store::StoreError> {
    let mut report = Report::default();
    for account in store.unadopted_accounts()? {
        let adopted = adopt_account(legacy, secrets, &account).await;
        report.moved += adopted
            .items
            .iter()
            .filter(|(_, item)| *item == Item::Moved)
            .count();
        if adopted.finished() {
            store.mark_secrets_adopted(&account, now)?;
            report.finished.push(account);
        } else {
            let out = adopted.out_of_reach();
            report.unfinished.push(adopted);
            // A keyring that answers none of the four is out of reach, for every account alike:
            // asking each of them in turn only waits on it again.
            if out {
                break;
            }
        }
    }
    if report.unfinished.is_empty() {
        drained.set();
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    //! The cut points. Every operation on the old entries, one after another, is the place a
    //! crash could land: the run is cut there (every later operation fails, as if the process
    //! had died), then run again over the same state, and the end state must be the same as an
    //! uncut run's, with no secret lost at any point between.
    //!
    //! | cut | state left | next run |
    //! | --- | --- | --- |
    //! | before the first read | all old entries, nothing filed | does everything |
    //! | after a read, before the put | old entry, nothing filed | files it |
    //! | after the put, before the read back | old entry and the filed one | porter's wins; deletes the old |
    //! | after the read back, before the delete | the same | the same |
    //! | between the parts of a chunked delete | head and some parts of the old entry | the old entry no longer reads, porter holds the key: deletes the rest |
    //! | between the last part and the head | a head naming parts that are gone | the same |
    //! | after the deletes, before the account is recorded | nothing old | finds none, records it |

    use super::*;
    use crate::signing_store::chunks::Limit;
    use chrono::TimeZone;
    use porter_secrets::MemorySecrets;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    /// Old entries in a map, as the Credential Manager keeps them: refusing a value over `limit`.
    #[derive(Default)]
    struct Mem {
        limit: usize,
        entries: Mutex<BTreeMap<String, String>>,
    }

    impl Slots for Mem {
        fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
            Ok(self.entries.lock().unwrap().get(name).cloned())
        }
        fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
            if value.encode_utf16().count() > self.limit {
                return Err(RuntimeError::Secrets(format!("{name} is too long")));
            }
            self.entries
                .lock()
                .unwrap()
                .insert(name.to_owned(), value.to_owned());
            Ok(())
        }
        fn delete(&self, name: &str) -> Result<(), RuntimeError> {
            self.entries.lock().unwrap().remove(name);
            Ok(())
        }
    }

    /// `Mem`, until `budget` operations have been made, and then a dead process.
    struct Cut {
        inner: Arc<Mem>,
        budget: AtomicUsize,
        refused: std::sync::atomic::AtomicBool,
    }

    impl Cut {
        fn tick(&self) -> Result<(), RuntimeError> {
            let left = self.budget.load(Ordering::SeqCst);
            if left == 0 {
                self.refused.store(true, Ordering::SeqCst);
                return Err(RuntimeError::Secrets("cut".to_owned()));
            }
            self.budget.store(left - 1, Ordering::SeqCst);
            Ok(())
        }
        fn was_cut(&self) -> bool {
            self.refused.load(Ordering::SeqCst)
        }
    }

    impl Slots for Cut {
        fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
            self.tick()?;
            self.inner.read(name)
        }
        fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
            self.tick()?;
            self.inner.write(name, value)
        }
        fn delete(&self, name: &str) -> Result<(), RuntimeError> {
            self.tick()?;
            self.inner.delete(name)
        }
    }

    fn account() -> AccountId {
        mail_domain::id::account_id_from_uuid(uuid::uuid!("00000000-0000-4000-8000-00000000a0a0"))
    }

    /// The window's limit, so an OAuth entry with a JWT in it is in parts.
    const WINDOWS: usize = 1280;

    /// What an earlier build wrote for the account, from the frozen fixture and one oversized
    /// Microsoft token: incoming password, the sign-in's OAuth, a CardDAV password.
    fn legacy_entries() -> (Arc<Mem>, Vec<(&'static str, Credential)>) {
        let fixture: Vec<Stored> =
            serde_json::from_str(include_str!("../tests/fixtures/keyring/account.json")).unwrap();
        let big = Stored::OAuth {
            access: format!("eyJ0eXAi{}", "a".repeat(3000)),
            refresh: "1//refresh".to_owned(),
            expires_at: DateTime::from_timestamp(1_767_672_306, 0).unwrap(),
        };
        let carddav = Stored::Password("dav-secret".to_owned());
        let mem = Arc::new(Mem {
            limit: WINDOWS,
            ..Mem::default()
        });
        let a = account();
        let mut want = Vec::new();
        for (word, stored) in [
            ("incoming", fixture[0].clone()),
            ("oauth", big),
            ("carddav", carddav),
        ] {
            let name = format!("{a}:{word}");
            let text = serde_json::to_string(&stored).unwrap();
            chunks::put(mem.as_ref(), &name, &text, Limit::Utf16Units(WINDOWS)).unwrap();
            want.push((word, credential_of(stored).unwrap()));
        }
        (mem, want)
    }

    fn key_of(word: &str) -> SecretKey {
        let purpose = ITEMS.iter().find(|(w, _)| *w == word).unwrap().1;
        SecretKey {
            account: account(),
            purpose,
        }
    }

    fn store_with_account() -> (SqliteStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::in_memory(dir.path()).unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO accounts (id, address, plan, created_at)
                 VALUES (?1, 'me@example.test', '{}', datetime('now'))",
                [account().to_string()],
            )
            .unwrap();
        (store, dir)
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 6, 0, 0, 0).unwrap()
    }

    fn legacy_of(mem: &Arc<Mem>) -> Legacy {
        mem.clone()
    }

    #[tokio::test]
    async fn a_run_moves_every_entry_deletes_every_part_and_is_idempotent() {
        let (mem, want) = legacy_entries();
        assert!(
            mem.entries.lock().unwrap().len() > 4,
            "the oauth fixture is in parts"
        );
        let (store, _dir) = store_with_account();
        let secrets = MemorySecrets::default();
        let drained = Drained::default();

        let first = run_over(&store, &legacy_of(&mem), &drained, &secrets, now())
            .await
            .unwrap();
        assert_eq!(first.finished, [account()]);
        assert_eq!(first.moved, 3);
        assert!(drained.get());
        // No orphans: not the head, not a part.
        assert!(mem.entries.lock().unwrap().is_empty(), "{:?}", mem.entries);
        for (word, credential) in &want {
            assert_eq!(
                &secrets.get(&key_of(word)).await.unwrap(),
                credential,
                "{word}"
            );
        }
        assert!(store.secrets_adopted(&account()).unwrap());

        let second = run_over(&store, &legacy_of(&mem), &drained, &secrets, now())
            .await
            .unwrap();
        assert!(second.nothing_to_do(), "{second:?}");
    }

    #[tokio::test]
    async fn a_cut_at_any_operation_is_finished_by_the_next_run_and_loses_nothing() {
        let (_, want) = legacy_entries();
        let mut cut_at = 0;
        loop {
            let (mem, _) = legacy_entries();
            let (store, _dir) = store_with_account();
            let secrets = MemorySecrets::default();
            let drained = Drained::default();
            let cut = Arc::new(Cut {
                inner: mem.clone(),
                budget: AtomicUsize::new(cut_at),
                refused: std::sync::atomic::AtomicBool::new(false),
            });
            let legacy: Legacy = cut.clone();
            let first = run_over(&store, &legacy, &drained, &secrets, now())
                .await
                .unwrap();

            // Whatever the cut left, every secret is somewhere: filed, or still in the old entry.
            for (word, credential) in &want {
                let filed = secrets.get(&key_of(word)).await.ok();
                let old = read_legacy(mem.as_ref(), &format!("{}:{word}", account()))
                    .ok()
                    .flatten();
                assert!(
                    filed.as_ref() == Some(credential) || old.as_ref() == Some(credential),
                    "cut at {cut_at}: {word} was lost"
                );
            }
            if !cut.was_cut() {
                assert!(first.unfinished.is_empty(), "{first:?}");
                break;
            }
            assert!(!drained.get(), "cut at {cut_at}: drained with work left");
            assert!(
                !store.secrets_adopted(&account()).unwrap(),
                "cut at {cut_at}"
            );

            // The next run, over the same state, with a process that lives.
            let again = run_over(&store, &legacy_of(&mem), &drained, &secrets, now())
                .await
                .unwrap();
            assert!(again.unfinished.is_empty(), "cut at {cut_at}: {again:?}");
            assert!(
                mem.entries.lock().unwrap().is_empty(),
                "cut at {cut_at}: {:?}",
                mem.entries
            );
            assert!(
                store.secrets_adopted(&account()).unwrap(),
                "cut at {cut_at}"
            );
            for (word, credential) in &want {
                assert_eq!(
                    &secrets.get(&key_of(word)).await.unwrap(),
                    credential,
                    "cut at {cut_at}: {word}"
                );
            }
            cut_at += 1;
        }
        // Every operation of a whole run was tried as the cut: reads, writes, the deletes of
        // each part, and the head last.
        assert!(cut_at > 12, "only {cut_at} cut points");
    }

    /// A store that refuses puts, or answers a get with something else once it has one.
    #[derive(Default)]
    struct Flaky {
        inner: MemorySecrets,
        refuse_put: bool,
        garble: bool,
    }

    impl Secrets for Flaky {
        async fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), SecretsError> {
            if self.refuse_put {
                return Err(SecretsError::Unavailable);
            }
            let stored = if self.garble {
                Credential::Password(SecretText::new("garbled"))
            } else {
                value.clone()
            };
            self.inner.put(key, &stored).await
        }
        async fn get(&self, key: &SecretKey) -> Result<Credential, SecretsError> {
            self.inner.get(key).await
        }
        async fn delete(&self, key: &SecretKey) -> Result<(), SecretsError> {
            self.inner.delete(key).await
        }
        async fn delete_account(&self, a: &AccountId) -> Result<(), SecretsError> {
            self.inner.delete_account(a).await
        }
    }

    #[tokio::test]
    async fn a_failed_put_keeps_every_old_entry_and_the_account_is_not_recorded() {
        let (mem, want) = legacy_entries();
        let before = mem.entries.lock().unwrap().clone();
        let (store, _dir) = store_with_account();
        let flaky = Flaky {
            refuse_put: true,
            ..Flaky::default()
        };
        let drained = Drained::default();
        let report = run_over(&store, &legacy_of(&mem), &drained, &flaky, now())
            .await
            .unwrap();
        assert_eq!(report.finished, []);
        assert_eq!(report.unfinished.len(), 1);
        assert!(
            report.unfinished[0]
                .items
                .iter()
                .any(|(w, i)| *w == "oauth" && *i == Item::Left(Left::PutFailed))
        );
        assert_eq!(
            *mem.entries.lock().unwrap(),
            before,
            "an old entry was touched"
        );
        assert!(!store.secrets_adopted(&account()).unwrap());
        assert!(!drained.get());

        // The account stays usable through the old entries until the next run: the platform
        // store reads them behind the empty one.
        let platform = crate::PlatformSecrets::over(
            MemorySecrets::default(),
            legacy_of(&mem),
            drained.clone(),
        );
        for (word, credential) in &want {
            assert_eq!(
                &platform.get(&key_of(word)).await.unwrap(),
                credential,
                "{word}"
            );
        }

        // And the next run, with a store that works, finishes it.
        let ok = run_over(
            &store,
            &legacy_of(&mem),
            &drained,
            &MemorySecrets::default(),
            now(),
        )
        .await
        .unwrap();
        assert_eq!(ok.finished, [account()]);
        assert!(drained.get());
    }

    #[tokio::test]
    async fn a_put_that_does_not_read_back_the_same_keeps_the_old_entry() {
        let (mem, _) = legacy_entries();
        let (store, _dir) = store_with_account();
        let flaky = Flaky {
            garble: true,
            ..Flaky::default()
        };
        let report = run_over(&store, &legacy_of(&mem), &Drained::default(), &flaky, now())
            .await
            .unwrap();
        assert!(report.finished.is_empty());
        assert!(
            report.unfinished[0]
                .items
                .iter()
                .any(|(_, i)| *i == Item::Left(Left::ReadBackDiffers))
        );
        assert!(
            read_legacy(mem.as_ref(), &format!("{}:carddav", account()))
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn what_is_already_filed_wins_over_a_stale_old_entry() {
        // A refresh token rotated since the run was cut: the old entry's is dead.
        let (mem, _) = legacy_entries();
        let (store, _dir) = store_with_account();
        let secrets = MemorySecrets::default();
        let newer = Credential::Password(SecretText::new("rotated"));
        secrets.put(&key_of("carddav"), &newer).await.unwrap();
        run_over(
            &store,
            &legacy_of(&mem),
            &Drained::default(),
            &secrets,
            now(),
        )
        .await
        .unwrap();
        assert_eq!(secrets.get(&key_of("carddav")).await.unwrap(), newer);
        assert!(mem.entries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_entry_that_does_not_read_is_left_alone_and_never_filed() {
        let (mem, _) = legacy_entries();
        mem.write(&format!("{}:incoming", account()), "not json")
            .unwrap();
        let (store, _dir) = store_with_account();
        let secrets = MemorySecrets::default();
        let report = run_over(
            &store,
            &legacy_of(&mem),
            &Drained::default(),
            &secrets,
            now(),
        )
        .await
        .unwrap();
        assert!(report.finished.is_empty());
        assert!(secrets.get(&key_of("incoming")).await.is_err());
        assert_eq!(
            mem.read(&format!("{}:incoming", account()))
                .unwrap()
                .as_deref(),
            Some("not json")
        );
    }

    #[tokio::test]
    async fn an_account_with_nothing_old_is_recorded_without_filing_anything() {
        let (store, _dir) = store_with_account();
        let mem = Arc::new(Mem {
            limit: WINDOWS,
            ..Mem::default()
        });
        let secrets = MemorySecrets::default();
        let drained = Drained::default();
        let report = run_over(&store, &legacy_of(&mem), &drained, &secrets, now())
            .await
            .unwrap();
        assert_eq!((report.finished.len(), report.moved), (1, 0));
        assert!(drained.get());
    }

    #[tokio::test]
    async fn forgetting_through_the_platform_store_forgets_the_old_entry_too() {
        let (mem, _) = legacy_entries();
        let platform = crate::PlatformSecrets::over(
            MemorySecrets::default(),
            legacy_of(&mem),
            Drained::default(),
        );
        platform.delete_account(&account()).await.unwrap();
        assert!(mem.entries.lock().unwrap().is_empty());
        assert!(platform.get(&key_of("incoming")).await.is_err());
    }

    /// `Mem`, holding every run at its first read until both have made it: neither has put or
    /// deleted anything when each has read the whole old entry, the worst interleaving.
    struct Gate {
        inner: Arc<Mem>,
        barrier: std::sync::Barrier,
    }

    impl Slots for Gate {
        fn read(&self, name: &str) -> Result<Option<String>, RuntimeError> {
            let read = self.inner.read(name);
            if name.ends_with(":incoming") {
                self.barrier.wait();
            }
            read
        }
        fn write(&self, name: &str, value: &str) -> Result<(), RuntimeError> {
            self.inner.write(name, value)
        }
        fn delete(&self, name: &str) -> Result<(), RuntimeError> {
            self.inner.delete(name)
        }
    }

    /// Two processes starting together (the window and `watch`) both run adoption over the same
    /// store, the same old entries and the same porter store. No secret is lost, every account
    /// is recorded, and where porter already held a value it is the one that stays.
    #[test]
    fn two_runs_at_once_over_one_store_converge_and_lose_nothing() {
        for round in 0..20 {
            let (mem, want) = legacy_entries();
            let gate = Arc::new(Gate {
                inner: mem.clone(),
                barrier: std::sync::Barrier::new(2),
            });
            let legacy: Legacy = gate.clone();
            let (store, _dir) = store_with_account();
            let secrets = MemorySecrets::default();
            let newer = Credential::Password(SecretText::new("rotated"));
            crate::block_on(secrets.put(&key_of("carddav"), &newer)).unwrap();
            let (drained_a, drained_b) = (Drained::default(), Drained::default());

            let (a, b) = std::thread::scope(|scope| {
                let a = scope.spawn(|| {
                    crate::block_on(run_over(&store, &legacy, &drained_a, &secrets, now()))
                });
                let b = scope.spawn(|| {
                    crate::block_on(run_over(&store, &legacy, &drained_b, &secrets, now()))
                });
                (a.join().unwrap().unwrap(), b.join().unwrap().unwrap())
            });

            assert!(
                a.unfinished.is_empty() && b.unfinished.is_empty(),
                "round {round}: {a:?} {b:?}"
            );
            assert!(drained_a.get() && drained_b.get(), "round {round}");
            assert!(mem.entries.lock().unwrap().is_empty(), "{:?}", mem.entries);
            assert!(store.secrets_adopted(&account()).unwrap());
            assert!(store.unadopted_accounts().unwrap().is_empty());
            for (word, credential) in want.iter().filter(|(w, _)| *w != "carddav") {
                let held = crate::block_on(secrets.get(&key_of(word))).unwrap();
                assert_eq!(&held, credential, "round {round}: {word}");
            }
            // Porter's was there first and stays, whichever run looked.
            assert_eq!(
                crate::block_on(secrets.get(&key_of("carddav"))).unwrap(),
                newer,
                "round {round}"
            );
        }
    }

    /// The old reader still reads what every earlier build wrote.
    #[test]
    fn the_frozen_account_fixture_reads_as_porter_credentials() {
        let entries: Vec<Stored> =
            serde_json::from_str(include_str!("../tests/fixtures/keyring/account.json")).unwrap();
        let read: Vec<Credential> = entries
            .into_iter()
            .map(|e| credential_of(e).unwrap())
            .collect();
        assert_eq!(read[0], Credential::Password(SecretText::new("hunter2")));
        let Credential::OAuth {
            access,
            refresh,
            expires_at,
        } = &read[1]
        else {
            panic!("{:?}", read[1]);
        };
        assert_eq!(access.expose(), "ya29.access");
        assert_eq!(refresh.expose(), "1//refresh");
        // 2026-01-06T04:05:06Z
        assert_eq!(*expires_at, UnixSeconds(1_767_672_306));
    }
}

//! Removing an account forgets its sign-in and every row that names it, and nothing else; when
//! it cannot, nothing at all.

use super::{RemoveError, remove};
use crate::account::{Credentials, add_with_password};
use crate::password::Password;
use mail_domain::id::{account_id_from_uuid, new_account_id};
use mail_runtime::{ClientRegistry, block_on};
use mail_store::SqliteStore;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::{MemorySecrets, Secrets, SecretsError};

const KEPT: &str = "kept@example.edu";
const GONE: &str = "gone@example.edu";

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

fn pop3() -> crate::account::Setup {
    crate::account::Setup::Pop3(mail_domain::presets::ManualPop3 {
        pop3_host: "pop.example.edu".to_owned(),
        pop3_port: 995,
        smtp_host: "smtp.example.edu".to_owned(),
        smtp_port: 465,
        login: None,
    })
}

/// A store with two password accounts, each with a label of its own, and the keyring holding
/// both passwords.
fn two_accounts(secrets: &MemorySecrets) -> (SqliteStore, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    for address in [KEPT, GONE] {
        let password = Password::new(format!("{address}-secret"));
        add_with_password(
            &store,
            address,
            Some(&pop3()),
            false,
            false,
            now(),
            Credentials {
                password: Some(&password),
                saved: &ClientRegistry::default(),
                secrets,
                on_url: &|url| panic!("a password account asked for a browser: {url}"),
                signed: None,
            },
        )
        .unwrap();
        store
            .connection()
            .execute(
                "INSERT INTO labels (id, account, name, origin) VALUES (?1, ?2, 'travel', 'user')",
                [
                    uuid::Uuid::new_v4().to_string(),
                    account_of(&store, address).to_string(),
                ],
            )
            .unwrap();
    }
    (store, dir)
}

fn account_of(store: &SqliteStore, address: &str) -> AccountId {
    let id: String = store
        .connection()
        .query_row(
            "SELECT id FROM accounts WHERE address = ?1",
            [address],
            |r| r.get(0),
        )
        .unwrap();
    account_id_from_uuid(id.parse().unwrap())
}

/// How many rows name `account`, in every table with an `account` column and in `accounts`.
fn rows_naming(store: &SqliteStore, account: AccountId) -> i64 {
    let db = store.connection();
    let tables: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let id = account.to_string();
    let mut count: i64 = db
        .query_row("SELECT count(*) FROM accounts WHERE id = ?1", [&id], |r| {
            r.get(0)
        })
        .unwrap();
    for table in tables {
        let has_account: bool = db
            .query_row(
                &format!(
                    "SELECT count(*) FROM pragma_table_info('{table}') WHERE name = 'account'"
                ),
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            > 0;
        if has_account {
            count += db
                .query_row(
                    &format!("SELECT count(*) FROM \"{table}\" WHERE account = ?1"),
                    [&id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap();
        }
    }
    count
}

fn password_of(secrets: &MemorySecrets, account: AccountId) -> Option<Credential> {
    block_on(Secrets::get(
        secrets,
        &SecretKey {
            account,
            purpose: SecretPurpose::IncomingPassword,
        },
    ))
    .ok()
}

#[test]
fn removing_an_account_forgets_its_sign_in_and_every_row_naming_it_and_nothing_else() {
    let secrets = MemorySecrets::default();
    let (store, _dir) = two_accounts(&secrets);
    let gone = account_of(&store, GONE);
    let kept = account_of(&store, KEPT);
    assert!(
        rows_naming(&store, gone.clone()) >= 3,
        "the fixture has nothing to remove"
    );
    let before = rows_naming(&store, kept.clone());

    let removed = block_on(remove(&store, &secrets, gone.clone())).unwrap();

    assert_eq!(removed.address, GONE);
    assert_eq!(rows_naming(&store, gone.clone()), 0);
    assert_eq!(password_of(&secrets, gone), None);
    assert_eq!(rows_naming(&store, kept.clone()), before);
    assert!(password_of(&secrets, kept).is_some());
}

#[test]
fn an_account_already_removed_is_unknown() {
    let secrets = MemorySecrets::default();
    let (store, _dir) = two_accounts(&secrets);
    let gone = account_of(&store, GONE);
    block_on(remove(&store, &secrets, gone.clone())).unwrap();
    for account in [gone, new_account_id()] {
        assert!(matches!(
            block_on(remove(&store, &secrets, account)),
            Err(RemoveError::Unknown)
        ));
    }
}

#[test]
fn the_account_that_keeps_mail_here_is_refused_and_kept() {
    let secrets = MemorySecrets::default();
    let (store, _dir) = two_accounts(&secrets);
    let local = crate::account::local(&store, now()).unwrap();
    assert!(matches!(
        block_on(remove(&store, &secrets, local.clone())),
        Err(RemoveError::Local)
    ));
    assert!(rows_naming(&store, local) > 0);
}

/// A keyring that is locked: it reads, and refuses to forget anything.
struct Locked<'a>(&'a MemorySecrets);

impl Secrets for Locked<'_> {
    async fn get(&self, key: &SecretKey) -> Result<Credential, SecretsError> {
        self.0.get(key).await
    }

    async fn put(&self, key: &SecretKey, value: &Credential) -> Result<(), SecretsError> {
        self.0.put(key, value).await
    }

    async fn delete(&self, _: &SecretKey) -> Result<(), SecretsError> {
        Err(SecretsError::Locked)
    }

    async fn delete_account(&self, _: &AccountId) -> Result<(), SecretsError> {
        Err(SecretsError::Locked)
    }
}

#[test]
fn a_keyring_that_will_not_forget_stops_the_removal_with_nothing_deleted() {
    let secrets = MemorySecrets::default();
    let (store, _dir) = two_accounts(&secrets);
    let gone = account_of(&store, GONE);
    let before = rows_naming(&store, gone.clone());

    let refused = block_on(remove(&store, &Locked(&secrets), gone.clone()));

    assert!(
        matches!(&refused, Err(RemoveError::Keyring(said)) if said.contains("locked")),
        "{refused:?}"
    );
    assert_eq!(rows_naming(&store, gone.clone()), before);
    assert!(password_of(&secrets, gone).is_some());
}

#[test]
fn removing_an_account_forgets_every_purpose_of_it_through_delete_account() {
    use porter_core::{CapabilityKind, SecretText};
    let secrets = MemorySecrets::default();
    let (store, _dir) = two_accounts(&secrets);
    let gone = account_of(&store, GONE);
    let kept = account_of(&store, KEPT);
    let purposes = [
        SecretPurpose::OutgoingPassword,
        SecretPurpose::OAuthRefresh,
        SecretPurpose::ServicePassword(CapabilityKind::Contacts),
    ];
    for account in [&gone, &kept] {
        for purpose in &purposes {
            block_on(secrets.put(
                &SecretKey {
                    account: account.clone(),
                    purpose: *purpose,
                },
                &Credential::Password(SecretText::new("x")),
            ))
            .unwrap();
        }
    }

    block_on(remove(&store, &secrets, gone.clone())).unwrap();

    for purpose in [SecretPurpose::IncomingPassword]
        .into_iter()
        .chain(purposes.iter().copied())
    {
        let key = |account: &AccountId| SecretKey {
            account: account.clone(),
            purpose,
        };
        assert!(block_on(secrets.get(&key(&gone))).is_err(), "{purpose:?}");
        assert!(block_on(secrets.get(&key(&kept))).is_ok(), "{purpose:?}");
    }
}

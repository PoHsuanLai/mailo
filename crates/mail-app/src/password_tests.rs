//! The password type refuses to be printed, and `add_with_password` puts it in the credential
//! store it is handed and nowhere else.

use super::Password;
use crate::account::{Credentials, add_with_password};
use mail_domain::{AccountId, Credential, SecretKey, SecretPurpose};
use mail_runtime::{MapSecrets, OAuthRegistry, Secrets};
use mail_store::SqliteStore;

const SECRET: &str = "hunter2-correct-horse";

#[test]
fn debug_never_prints_the_password() {
    let password = Password::new(SECRET.to_owned());
    for shown in [format!("{password:?}"), format!("{password:#?}")] {
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("redacted"), "{shown}");
    }
    // Inside something that derives Debug, too: that is where a stray `{:?}` usually is.
    let held = Some(password);
    assert!(!format!("{held:?}").contains("hunter2"));
}

#[test]
fn an_empty_password_says_so_and_nothing_else() {
    let password = Password::default();
    assert!(password.is_empty());
    assert_eq!(format!("{password:?}"), "Password(<empty>)");
}

fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

/// A POP3 server that logs in with a number, named by hand.
fn pop3() -> crate::cli::Setup {
    crate::cli::Setup::Pop3(mail_domain::presets::ManualPop3 {
        pop3_host: "pop.example.edu".to_owned(),
        pop3_port: 995,
        smtp_host: "smtp.example.edu".to_owned(),
        smtp_port: 465,
        login: Some("s1234567".to_owned()),
    })
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
    AccountId::from_uuid(id.parse().unwrap())
}

/// Every text column of every table, joined: where a password written to SQLite would be.
fn everything_in(store: &SqliteStore) -> String {
    let db = store.connection();
    let mut tables = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap();
    let names: Vec<String> = tables
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut all = String::new();
    for name in names {
        let Ok(mut rows) = db.prepare(&format!("SELECT * FROM \"{name}\"")) else {
            continue;
        };
        let columns = rows.column_count();
        let mut query = rows.query([]).unwrap();
        while let Some(row) = query.next().unwrap() {
            for at in 0..columns {
                if let Ok(Some(text)) = row.get::<_, Option<String>>(at) {
                    all.push_str(&text);
                    all.push('\n');
                }
            }
        }
    }
    all
}

#[test]
fn add_with_password_keeps_the_password_in_the_store_it_is_handed_and_nowhere_else() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let secrets = MapSecrets::default();
    let password = Password::new(SECRET.to_owned());
    let said = add_with_password(
        &store,
        "s1234567@example.edu",
        Some(&pop3()),
        false,
        false,
        now(),
        Credentials {
            password: Some(&password),
            saved: &OAuthRegistry::default(),
            secrets: &secrets,
            on_url: &|url| panic!("a password account asked for a browser: {url}"),
        },
    )
    .unwrap();
    assert!(said.contains("password stored"), "{said}");
    assert!(!said.contains(SECRET), "the password was in what add said");

    let account = account_of(&store, "s1234567@example.edu");
    for purpose in [
        SecretPurpose::IncomingPassword,
        SecretPurpose::OutgoingPassword,
    ] {
        let kept = secrets.get(&SecretKey { account, purpose }).unwrap();
        assert_eq!(kept, Credential::Password(SECRET.to_owned()), "{purpose:?}");
    }
    assert!(
        !everything_in(&store).contains(SECRET),
        "the password reached SQLite"
    );
}

#[test]
fn no_password_stores_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let secrets = MapSecrets::default();
    let empty = Password::default();
    for password in [None, Some(&empty)] {
        let said = add_with_password(
            &store,
            "s1234567@example.edu",
            Some(&pop3()),
            false,
            false,
            now(),
            Credentials {
                password,
                saved: &OAuthRegistry::default(),
                secrets: &secrets,
                on_url: &|url| panic!("a password account asked for a browser: {url}"),
            },
        )
        .unwrap();
        assert!(said.contains("no password stored"), "{said}");
    }
    let account = account_of(&store, "s1234567@example.edu");
    assert!(
        secrets
            .get(&SecretKey {
                account,
                purpose: SecretPurpose::IncomingPassword,
            })
            .is_err()
    );
}

#[test]
fn a_jmap_bearer_token_goes_where_a_password_would_and_the_plan_says_bearer() {
    // The window's path: the token handed over in `Credentials`, `HttpAuth::Bearer` named in
    // the setup, and no environment variable read.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let secrets = MapSecrets::default();
    let token = Password::new(SECRET.to_owned());
    let said = add_with_password(
        &store,
        "me@example.test",
        Some(&crate::cli::Setup::Jmap {
            session: Some("https://jmap.example.test/.well-known/jmap".to_owned()),
            login: None,
            auth: mail_domain::HttpAuth::Bearer,
        }),
        false,
        false,
        now(),
        Credentials {
            password: Some(&token),
            saved: &OAuthRegistry::default(),
            secrets: &secrets,
            on_url: &|url| panic!("a JMAP account asked for a browser: {url}"),
        },
    )
    .unwrap();
    assert!(said.contains("token stored"), "{said}");
    assert!(!said.contains(SECRET));
    let account = account_of(&store, "me@example.test");
    let kept = secrets
        .get(&SecretKey {
            account,
            purpose: SecretPurpose::IncomingPassword,
        })
        .unwrap();
    assert_eq!(kept, Credential::Password(SECRET.to_owned()));
    let plan: String = store
        .connection()
        .query_row(
            "SELECT plan FROM accounts WHERE id = ?1",
            [account.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let plan: mail_domain::AccountPlan = serde_json::from_str(&plan).unwrap();
    assert!(matches!(
        plan.incoming,
        mail_domain::Incoming::Jmap {
            auth: mail_domain::HttpAuth::Bearer,
            ..
        }
    ));
    assert!(
        !everything_in(&store).contains(SECRET),
        "the token reached SQLite"
    );
}

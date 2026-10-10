//! The password type refuses to be printed, and `add_with_password` puts it in the credential
//! store it is handed and nowhere else.

use super::Password;
use crate::Environment;
use crate::account::{Credentials, Outcome, add_with_password};
use mail_runtime::{AccountSecrets, ClientRegistry};
use mail_store::SqliteStore;
use porter_core::SecretText;
use porter_core::{AccountId, Credential, SecretKey, SecretPurpose};
use porter_secrets::MemorySecrets;

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
fn pop3() -> crate::account::Setup {
    crate::account::Setup::Pop3(mail_domain::presets::ManualPop3 {
        pop3_host: "pop.example.edu".to_owned(),
        pop3_port: 995,
        smtp_host: "smtp.example.edu".to_owned(),
        smtp_port: 465,
        login: Some("s1234567".to_owned()),
    })
}

fn account_of(store: &SqliteStore, address: &str) -> AccountId {
    store
        .account_by_address(address)
        .unwrap()
        .expect("the account")
        .id
}

/// Every text column of every table, joined: where a password written to SQLite would be.
fn everything_in(store: &SqliteStore) -> String {
    mail_store::testing::all_text(store)
}

#[test]
fn add_with_password_keeps_the_password_in_the_store_it_is_handed_and_nowhere_else() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let secrets = MemorySecrets::default();
    let password = Password::new(SECRET.to_owned());
    let said = mail_runtime::block_on(add_with_password(
        &store,
        "s1234567@example.edu",
        Some(&pop3()),
        false,
        false,
        now(),
        Credentials {
            password: Some(&password),
            saved: &ClientRegistry::default(),
            secrets: &secrets,
            environment: &Environment::default(),
            on_url: &|url| panic!("a password account asked for a browser: {url}"),
            signed: None,
        },
    ))
    .unwrap();
    assert!(
        matches!(
            &said.outcome,
            Outcome::PasswordStored { bearer: false, login, .. } if login == "s1234567"
        ),
        "{said:?}"
    );
    assert!(
        !format!("{said:?}").contains(SECRET),
        "the password was in what add said"
    );

    let account = account_of(&store, "s1234567@example.edu");
    for purpose in [
        SecretPurpose::IncomingPassword,
        SecretPurpose::OutgoingPassword,
    ] {
        let kept = mail_runtime::block_on(secrets.get(&SecretKey {
            account: account.clone(),
            purpose,
        }))
        .unwrap();
        assert_eq!(
            kept,
            Credential::Password(SecretText::new(SECRET.to_owned())),
            "{purpose:?}"
        );
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
    let secrets = MemorySecrets::default();
    let empty = Password::default();
    for password in [None, Some(&empty)] {
        let said = mail_runtime::block_on(add_with_password(
            &store,
            "s1234567@example.edu",
            Some(&pop3()),
            false,
            false,
            now(),
            Credentials {
                password,
                saved: &ClientRegistry::default(),
                secrets: &secrets,
                environment: &Environment::default(),
                on_url: &|url| panic!("a password account asked for a browser: {url}"),
                signed: None,
            },
        ))
        .unwrap();
        assert!(
            matches!(&said.outcome, Outcome::PasswordMissing { .. }),
            "{said:?}"
        );
    }
    let account = account_of(&store, "s1234567@example.edu");
    assert!(
        mail_runtime::block_on(secrets.get(&SecretKey {
            account,
            purpose: SecretPurpose::IncomingPassword,
        }))
        .is_err()
    );
}

#[test]
fn a_jmap_bearer_token_goes_where_a_password_would_and_the_plan_says_bearer() {
    // The window's path: the token handed over in `Credentials`, `HttpAuth::Bearer` named in
    // the setup, and no environment variable read.
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let secrets = MemorySecrets::default();
    let token = Password::new(SECRET.to_owned());
    let said = mail_runtime::block_on(add_with_password(
        &store,
        "me@example.test",
        Some(&crate::account::Setup::Jmap {
            session: Some("https://jmap.example.test/.well-known/jmap".to_owned()),
            login: None,
            auth: mail_domain::HttpAuth::Bearer,
        }),
        false,
        false,
        now(),
        Credentials {
            password: Some(&token),
            saved: &ClientRegistry::default(),
            secrets: &secrets,
            environment: &Environment::default(),
            on_url: &|url| panic!("a JMAP account asked for a browser: {url}"),
            signed: None,
        },
    ))
    .unwrap();
    assert!(
        matches!(&said.outcome, Outcome::PasswordStored { bearer: true, .. }),
        "{said:?}"
    );
    assert!(!format!("{said:?}").contains(SECRET));
    let account = account_of(&store, "me@example.test");
    let kept = mail_runtime::block_on(secrets.get(&SecretKey {
        account: account.clone(),
        purpose: SecretPurpose::IncomingPassword,
    }))
    .unwrap();
    assert_eq!(
        kept,
        Credential::Password(SecretText::new(SECRET.to_owned()))
    );
    let plan: mail_domain::AccountPlan = store
        .account(account.clone())
        .unwrap()
        .expect("the account")
        .plan
        .unwrap();
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

#[test]
fn a_credential_a_sign_in_already_made_is_filed_without_asking_for_a_browser_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::in_memory(dir.path()).unwrap();
    let secrets = MemorySecrets::default();
    let saved = mail_runtime::clients::registry_of(vec![mail_runtime::clients::entry(
        porter_provider::Issuer::Google,
        "client-id",
        Some("client-secret"),
    )]);
    let signed = Credential::Password(SecretText::new("refresh-token"));
    let said = mail_runtime::block_on(add_with_password(
        &store,
        "ada@gmail.com",
        None,
        false,
        false,
        now(),
        Credentials {
            password: None,
            saved: &saved,
            secrets: &secrets,
            environment: &Environment::default(),
            on_url: &|url| panic!("the sign-in was made already, and asked for a browser: {url}"),
            signed: Some(&signed),
        },
    ))
    .unwrap();
    assert!(
        matches!(&said.outcome, Outcome::SignedIn { .. }),
        "{said:?}"
    );
    let account = account_of(&store, "ada@gmail.com");
    let kept = mail_runtime::block_on(secrets.get(&SecretKey {
        account,
        purpose: SecretPurpose::OAuthRefresh,
    }))
    .unwrap();
    assert_eq!(kept, signed);
}
